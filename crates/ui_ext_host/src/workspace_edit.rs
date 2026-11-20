//! The editor side of `workspace:edit` (W2c's [`WorkspaceEdits`], plan 4.8, design.md 0.3 §10).
//!
//! `ui_ext_wasm` has checked the edit, the overlaps, the protected paths and the broker. Here, on
//! the main thread, every target is resolved against the open worktrees (with the extension's
//! opaque worktree ids) and checked: an untrusted worktree is `denied`; a collaboration guest, a
//! link out of the worktree, a `file_scan_exclusions` or `private_files` match, a config file
//! `fyx_guard` protects or a file over 16 MiB is `not-allowed`; a missing file without `create` is
//! `not-found`. The buffers are loaded, and then, in one synchronous step: the versions
//! (`conflict`), the ranges (`invalid`), and every file's edits as one transaction of its own, so
//! one undo reverts it and typing just before does not merge into it. Nothing is saved; the host
//! holds each edited buffer until it is clean again, since the project holds buffers weakly. A card
//! "<extension>: <label>, N files" offers Show (a multibuffer of the edited ranges, in which one
//! undo reverts the whole edit) and Undo.
//!
//! The extension thread waits on a channel. Dropping its future (reload, revocation) before the
//! synchronous step leaves nothing applied: the step checks that someone still waits.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
};

use collections::HashMap;
use editor::{Editor, MultiBuffer, PathKey};
use fs::Fs;
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::BoxFuture,
};
use gpui::{App, AppContext as _, AsyncApp, Entity, EntityId, Global, SharedString, WeakEntity};
use language::{Bias, Buffer, BufferSnapshot, Capability, Point};
use project::{Project, ProjectPath, ProjectTransaction, WorktreeSettings};
use settings::{Settings as _, SettingsLocation};
use ui_ext_wasm::edit::{
    EditError, EditTarget, MAX_FILE_BYTES, Position, TextEdit, WorkspaceEdit, WorkspaceEdits,
};
use workspace::{
    Workspace,
    notifications::{NotificationId, simple_message_notification::MessageNotification},
};

use crate::worktrees::{OpenWorktree, OpenWorktrees};

/// The open workspaces by entity id (an [`OpenWorktree`]'s `key.0`), to find a worktree's
/// project and the window to notify. Main thread only.
#[derive(Default)]
struct EditWorkspaces(HashMap<EntityId, Registered>);

struct Registered {
    /// `None` in tests, which have no workspace: no card then.
    workspace: Option<WeakEntity<Workspace>>,
    project: WeakEntity<Project>,
    /// The workspace's window, to open files in it (`workspace:open`); `None` in tests.
    window: Option<gpui::AnyWindowHandle>,
}

impl Global for EditWorkspaces {}

pub(crate) fn register_workspace(
    id: EntityId,
    workspace: Option<WeakEntity<Workspace>>,
    project: WeakEntity<Project>,
    cx: &mut App,
) {
    cx.default_global::<EditWorkspaces>().0.insert(
        id,
        Registered {
            workspace,
            project,
            window: None,
        },
    );
}

/// The window of a registered workspace (`workspace:open`).
pub(crate) fn set_workspace_window(id: EntityId, window: gpui::AnyWindowHandle, cx: &mut App) {
    if let Some(registered) = cx.default_global::<EditWorkspaces>().0.get_mut(&id) {
        registered.window = Some(window);
    }
}

/// A registered workspace and its window, by the workspace's entity id.
pub(crate) fn workspace_and_window(
    id: EntityId,
    cx: &App,
) -> Option<(gpui::Entity<Workspace>, gpui::AnyWindowHandle)> {
    let registered = cx.try_global::<EditWorkspaces>()?.0.get(&id)?;
    Some((
        registered.workspace.as_ref()?.upgrade()?,
        registered.window?,
    ))
}

pub(crate) fn unregister_workspace(id: EntityId, cx: &mut App) {
    if cx.has_global::<EditWorkspaces>() {
        cx.global_mut::<EditWorkspaces>().0.remove(&id);
    }
}

/// Whether an extension may not see `abs_path`'s contents (integration-5 S1): it matches
/// `file_scan_exclusions` or `private_files` (always, not only in shield mode), or `fyx_guard`
/// refuses an agent read of it, in any open project that has it. Fails closed (5.19 P7): a path
/// that no registered project resolves, or any path before a project is registered, counts as
/// private. Callers that only show worktree files check the worktree themselves.
pub(crate) fn is_private_path(abs_path: &Path, cx: &App) -> bool {
    let Some(workspaces) = cx.try_global::<EditWorkspaces>() else {
        return true;
    };
    let mut resolved = false;
    let private = workspaces.0.values().any(|registered| {
        let Some(project) = registered.project.upgrade() else {
            return false;
        };
        let project = project.read(cx);
        let Some(project_path) = project.find_project_path(abs_path, cx) else {
            return false;
        };
        resolved = true;
        let path = project_path.path.as_ref();
        let settings = WorktreeSettings::get(
            Some(SettingsLocation {
                worktree_id: project_path.worktree_id,
                path,
            }),
            cx,
        );
        let global = WorktreeSettings::get_global(cx);
        settings.is_path_excluded(path)
            || global.is_path_excluded(path)
            || settings.is_path_private(path)
            || global.is_path_private(path)
            || fyx_guard::extension_read_denial(project, &project_path, cx).is_some()
    });
    private || !resolved
}

fn registered(
    worktree: &OpenWorktree,
    cx: &App,
) -> Option<(Option<WeakEntity<Workspace>>, Entity<Project>)> {
    let registered = cx
        .try_global::<EditWorkspaces>()?
        .0
        .get(&EntityId::from(worktree.key.0))?;
    Some((registered.workspace.clone(), registered.project.upgrade()?))
}

/// The version an edit's `expected-version` names (WIT `document-info.version`): it grows with
/// every change to the buffer, undo and redo included. A documents provider must report this one.
pub fn document_version(buffer: &Buffer) -> u64 {
    buffer
        .version()
        .iter()
        .map(|timestamp| u64::from(timestamp.value))
        .sum()
}

struct Request {
    /// What the extension's worktree ids are keyed by (its id, or a dev extension's path).
    key: Arc<str>,
    extension_id: String,
    edit: WorkspaceEdit,
    reply: oneshot::Sender<Result<(), EditError>>,
}

/// The main-thread loop's inbox; edits apply one at a time, in the order they arrive.
#[derive(Clone)]
struct EditService(mpsc::UnboundedSender<Request>);

impl Global for EditService {}

/// The [`WorkspaceEdits`] of one load. `key` is what [`OpenWorktrees::for_extension`] was given.
pub(crate) fn for_extension(key: &str, cx: &mut App) -> Arc<dyn WorkspaceEdits> {
    let sender = match cx.try_global::<EditService>() {
        Some(service) => service.0.clone(),
        None => {
            let (sender, mut requests) = mpsc::unbounded::<Request>();
            cx.spawn(async move |cx| {
                while let Some(request) = requests.next().await {
                    handle(request, cx).await;
                }
            })
            .detach();
            cx.set_global(EditService(sender.clone()));
            sender
        }
    };
    Arc::new(ExtensionEdits {
        key: key.into(),
        sender,
    })
}

struct ExtensionEdits {
    key: Arc<str>,
    sender: mpsc::UnboundedSender<Request>,
}

impl WorkspaceEdits for ExtensionEdits {
    fn apply(
        &self,
        extension_id: &str,
        edit: WorkspaceEdit,
    ) -> BoxFuture<'static, Result<(), EditError>> {
        let (reply, outcome) = oneshot::channel();
        let sent = self.sender.unbounded_send(Request {
            key: self.key.clone(),
            extension_id: extension_id.to_string(),
            edit,
            reply,
        });
        async move {
            if sent.is_err() {
                return Err(EditError::Cancelled);
            }
            outcome.await.unwrap_or(Err(EditError::Cancelled))
        }
        .boxed()
    }
}

async fn handle(request: Request, cx: &mut AsyncApp) {
    let Request {
        key,
        extension_id,
        edit,
        reply,
    } = request;
    if reply.is_canceled() {
        return;
    }
    let label = edit.label.clone();
    let result = apply_edit(&key, edit, &|| reply.is_canceled(), cx).await;
    let result = result.map(|applied| {
        cx.update(|cx| notify(&extension_id, &label, applied, cx));
    });
    reply.send(result).ok();
}

/// One file of the edit, resolved.
struct Target {
    project: Entity<Project>,
    project_path: ProjectPath,
    abs_path: PathBuf,
    root: PathBuf,
    fs: Arc<dyn Fs>,
    /// The path as the extension named it, for messages.
    display: String,
    exists: bool,
    expected_version: Option<u64>,
    edits: Vec<TextEdit>,
}

/// What an edit changed, for the card.
pub(crate) struct Applied {
    pub workspace: Option<WeakEntity<Workspace>>,
    pub transaction: ProjectTransaction,
}

/// Resolves, checks and applies `edit` for the extension whose worktree ids are keyed by `key`.
/// `canceled` is asked right before anything is applied.
pub(crate) async fn apply_edit(
    key: &str,
    edit: WorkspaceEdit,
    canceled: &dyn Fn() -> bool,
    cx: &mut AsyncApp,
) -> Result<Applied, EditError> {
    let (targets, workspace) = cx.update(|cx| resolve(key, edit, cx))?;
    for target in &targets {
        stays_inside(target).await?;
    }
    if canceled() {
        return Err(EditError::Cancelled);
    }
    // `create`: an empty file first (design.md §10). If the edit fails after that, the files it
    // created are removed again, so a failed edit leaves nothing behind.
    let mut created = Vec::new();
    let result = async {
        for target in targets.iter().filter(|target| !target.exists) {
            if let Ok(Some(_)) = target.fs.metadata(&target.abs_path).await {
                // It appeared since the worktree was scanned: not ours to remove on failure.
                continue;
            }
            let options = fs::CreateOptions {
                overwrite: false,
                ignore_if_exists: true,
            };
            target
                .fs
                .create_file(&target.abs_path, options)
                .await
                .map_err(|error| {
                    EditError::NotFound(format!("{} could not be created: {error}", target.display))
                })?;
            created.push(target);
        }
        let mut buffers = Vec::with_capacity(targets.len());
        for target in &targets {
            let open = target.project.update(cx, |project, cx| {
                project.open_buffer(target.project_path.clone(), cx)
            });
            let buffer = open.await.map_err(|error| {
                EditError::NotFound(format!("{} could not be opened: {error}", target.display))
            })?;
            buffers.push(buffer);
        }
        cx.update(|cx| apply_resolved(&targets, &buffers, canceled, cx))
    }
    .await;
    match result {
        Ok(transaction) => Ok(Applied {
            workspace,
            transaction,
        }),
        Err(error) => {
            for target in created {
                let options = fs::RemoveOptions {
                    recursive: false,
                    ignore_if_not_exists: true,
                };
                target.fs.remove_file(&target.abs_path, options).await.ok();
            }
            Err(error)
        }
    }
}

/// One file of a worktree, resolved and checked as an edit target is (`review:propose`, §B.10).
pub(crate) struct ResolvedFile {
    pub project: Entity<Project>,
    pub project_path: ProjectPath,
    pub abs_path: PathBuf,
}

/// Resolves `file` for the extension whose worktree ids are keyed by `key`, with every check of
/// an edit target (trust, collaboration, links, exclusions, `private_files`, `fyx_guard`, size).
/// A missing file is `not-found` unless `create`; nothing is created here.
pub(crate) async fn resolve_file(
    key: &str,
    file: ui_ext::process::FileRef,
    create: bool,
    cx: &mut AsyncApp,
) -> Result<ResolvedFile, EditError> {
    let edit = WorkspaceEdit {
        label: String::new(),
        files: vec![ui_ext_wasm::edit::FileEdit {
            target: EditTarget::File(file),
            expected_version: None,
            create,
            edits: Vec::new(),
        }],
    };
    let (mut targets, _) = cx.update(|cx| resolve(key, edit, cx))?;
    let target = targets
        .pop()
        .ok_or_else(|| EditError::NotFound("no file".into()))?;
    stays_inside(&target).await?;
    Ok(ResolvedFile {
        project: target.project,
        project_path: target.project_path,
        abs_path: target.abs_path,
    })
}

/// What an [`EditError`] says, for a host whose errors are strings.
pub(crate) fn describe_error(error: &EditError) -> String {
    match error {
        EditError::Denied => "denied".into(),
        EditError::Cancelled => "cancelled".into(),
        EditError::UntrustedWorktree => "the worktree is not trusted (restricted mode)".into(),
        EditError::Conflict(why)
        | EditError::NotAllowed(why)
        | EditError::NotFound(why)
        | EditError::Invalid(why)
        | EditError::Other(why) => why.clone(),
    }
}

fn not_allowed(display: &str, why: &str) -> EditError {
    EditError::NotAllowed(format!("{display}: {why}"))
}

fn resolve(
    key: &str,
    edit: WorkspaceEdit,
    cx: &mut App,
) -> Result<(Vec<Target>, Option<WeakEntity<Workspace>>), EditError> {
    let open = OpenWorktrees::global(cx).all();
    let mut targets = Vec::with_capacity(edit.files.len());
    let mut workspace = None;
    for file in edit.files {
        let file_ref = match file.target {
            EditTarget::Document(_) => {
                return Err(EditError::NotFound(
                    "no such document (this host has no documents yet)".into(),
                ));
            }
            EditTarget::File(file_ref) => file_ref,
        };
        let display = file_ref.path.clone();
        // The worktrees it may be in, with the path's project path in each.
        let mut located = Vec::new();
        for worktree in open.iter().filter(|worktree| {
            file_ref
                .worktree
                .is_none_or(|id| crate::worktrees::opaque_id(key, worktree.key) == id)
        }) {
            let Some((owner, project)) = registered(worktree, cx) else {
                continue;
            };
            let abs_path = file_ref
                .path
                .split('/')
                .fold(worktree.root.clone(), |path, component| {
                    path.join(component)
                });
            let project_path = project
                .read(cx)
                .find_project_path(&abs_path, cx)
                .filter(|path| path.worktree_id.to_proto() == worktree.key.1);
            if let Some(project_path) = project_path {
                let exists = project.read(cx).entry_for_path(&project_path, cx).is_some();
                located.push((worktree, owner, project, project_path, abs_path, exists));
            }
        }
        let existing = located.iter().filter(|located| located.5).count();
        let chosen = match (existing, located.len()) {
            (1, _) => located.into_iter().find(|located| located.5),
            (0, 0) if file_ref.worktree.is_some() => {
                return Err(EditError::NotFound("no such worktree".into()));
            }
            (0, 0) => return Err(EditError::NotFound(format!("{display} is in no worktree"))),
            (0, _) if !file.create => {
                return Err(EditError::NotFound(format!("{display} does not exist")));
            }
            (0, 1) => located.into_iter().next(),
            (0, _) => {
                return Err(EditError::Invalid(format!(
                    "several worktrees could hold {display}; name one"
                )));
            }
            _ => {
                return Err(EditError::Invalid(format!(
                    "{display} is in several worktrees; name one"
                )));
            }
        };
        let Some((worktree, owner, project, project_path, abs_path, exists)) = chosen else {
            return Err(EditError::NotFound(format!("{display} does not exist")));
        };
        if !worktree.trusted {
            // Decision 26: restricted mode refuses without a prompt, with its own code (W2c A5).
            return Err(EditError::UntrustedWorktree);
        }
        check_target(&project, &project_path, &abs_path, exists, &display, cx)?;
        if workspace.is_none() {
            workspace = owner;
        }
        targets.push(Target {
            fs: project.read(cx).fs().clone(),
            project,
            project_path,
            abs_path,
            root: worktree.root.clone(),
            display,
            exists,
            expected_version: file.expected_version,
            edits: file.edits,
        });
    }
    Ok((targets, workspace))
}

/// The checks a resolved file must pass, before anything is loaded.
fn check_target(
    project: &Entity<Project>,
    project_path: &ProjectPath,
    abs_path: &Path,
    exists: bool,
    display: &str,
    cx: &App,
) -> Result<(), EditError> {
    let project = project.read(cx);
    if project.is_via_collab() {
        return Err(not_allowed(
            display,
            "the project is someone else's (collaboration)",
        ));
    }
    let worktree = project
        .worktree_for_id(project_path.worktree_id, cx)
        .ok_or_else(|| EditError::NotFound(format!("{display}: the worktree closed")))?;
    let worktree = worktree.read(cx);
    // A link out of the worktree, at the file or at any directory above it.
    let mut ancestor = Some(project_path.path.as_ref());
    while let Some(path) = ancestor {
        if worktree
            .entry_for_path(path)
            .is_some_and(|entry| entry.is_external)
        {
            return Err(not_allowed(
                display,
                "it leaves the worktree through a link",
            ));
        }
        ancestor = path.parent();
    }
    let path = project_path.path.as_ref();
    let settings = WorktreeSettings::get(
        Some(SettingsLocation {
            worktree_id: project_path.worktree_id,
            path,
        }),
        cx,
    );
    if settings.is_path_excluded(path) || WorktreeSettings::get_global(cx).is_path_excluded(path) {
        return Err(not_allowed(display, "it is excluded from the project"));
    }
    if settings.is_path_private(path) || WorktreeSettings::get_global(cx).is_path_private(path) {
        return Err(not_allowed(
            display,
            "it matches the `private_files` setting",
        ));
    }
    // `fyx_guard`'s rules for extensions, in every security mode (integration-5 S1, S2): its
    // private paths, agent configs at any depth, `read_only` patterns, links out.
    if let Some(why) = fyx_guard::extension_read_denial(project, project_path, cx)
        .or_else(|| fyx_guard::extension_write_denial(project, abs_path, cx))
    {
        return Err(not_allowed(display, &why));
    }
    match worktree.entry_for_path(path) {
        Some(entry) if entry.is_dir() => {
            Err(EditError::Invalid(format!("{display} is a directory")))
        }
        Some(entry) if entry.size > MAX_FILE_BYTES => {
            Err(not_allowed(display, "it is larger than 16 MiB"))
        }
        Some(_) => Ok(()),
        None if exists => Ok(()),
        None => {
            let directory = path.parent();
            if directory
                .and_then(|directory| worktree.entry_for_path(directory))
                .is_some_and(|entry| entry.is_dir())
            {
                Ok(())
            } else {
                Err(EditError::NotFound(format!(
                    "the directory of {display} does not exist"
                )))
            }
        }
    }
}

/// The file on disk (or the directory a new one goes into), links resolved, is inside its
/// worktree's root. A file that is on disk though the worktree has no entry for it (an unscanned
/// directory) is checked itself, links and size included, before it is loaded (integration-5 S8).
async fn stays_inside(target: &Target) -> Result<(), EditError> {
    let root = target.fs.canonicalize(&target.root).await;
    let metadata = target.fs.metadata(&target.abs_path).await.ok().flatten();
    if let Some(metadata) = &metadata {
        if metadata.is_dir {
            return Err(EditError::Invalid(format!(
                "{} is a directory",
                target.display
            )));
        }
        if metadata.len > MAX_FILE_BYTES {
            return Err(not_allowed(&target.display, "it is larger than 16 MiB"));
        }
    }
    let on_disk = if target.exists || metadata.is_some() {
        target.abs_path.as_path()
    } else {
        target.abs_path.parent().unwrap_or(&target.root)
    };
    let file = target.fs.canonicalize(on_disk).await;
    match (root, file) {
        (Ok(root), Ok(file)) if file.starts_with(&root) => Ok(()),
        (Ok(_), Ok(_)) => Err(not_allowed(
            &target.display,
            "it leaves the worktree through a link",
        )),
        (_, Err(error)) => Err(EditError::NotFound(format!(
            "{} could not be read: {error}",
            target.display
        ))),
        (Err(error), _) => Err(EditError::NotFound(format!(
            "the worktree of {} could not be read: {error}",
            target.display
        ))),
    }
}

/// A position in `snapshot`. Past the end it is `invalid` when the edit names a version and is
/// clamped otherwise (design.md §10); inside a character it is `invalid`.
fn point(
    snapshot: &BufferSnapshot,
    position: Position,
    strict: bool,
    display: &str,
) -> Result<Point, EditError> {
    let max = snapshot.max_point();
    let mut point = Point::new(position.line, position.column);
    let line_len = if point.row <= max.row {
        snapshot.line_len(point.row)
    } else {
        0
    };
    if point.row > max.row || point.column > line_len {
        if strict {
            return Err(EditError::Invalid(format!(
                "line {}, column {} is past the end of {display}",
                position.line, position.column
            )));
        }
        point = if point.row > max.row {
            max
        } else {
            Point::new(point.row, line_len)
        };
    }
    if snapshot.clip_point(point, Bias::Left) != point {
        return Err(EditError::Invalid(format!(
            "line {}, column {} of {display} is inside a character",
            position.line, position.column
        )));
    }
    Ok(point)
}

/// The synchronous step: checks versions and ranges of every file, then applies them all.
fn apply_resolved(
    targets: &[Target],
    buffers: &[Entity<Buffer>],
    canceled: &dyn Fn() -> bool,
    cx: &mut App,
) -> Result<ProjectTransaction, EditError> {
    if canceled() {
        return Err(EditError::Cancelled);
    }
    let mut plans: Vec<(Entity<Buffer>, Vec<(Range<Point>, String)>)> = Vec::new();
    for (target, buffer) in targets.iter().zip(buffers) {
        let buffer_ref = buffer.read(cx);
        if buffer_ref.len() as u64 > MAX_FILE_BYTES {
            return Err(not_allowed(&target.display, "it is larger than 16 MiB"));
        }
        if let Some(expected) = target.expected_version {
            let current = document_version(buffer_ref);
            if current != expected {
                return Err(EditError::Conflict(format!(
                    "{} changed since version {expected} (it is {current} now)",
                    target.display
                )));
            }
        }
        let snapshot = buffer_ref.snapshot();
        let strict = target.expected_version.is_some();
        let mut ranges = Vec::with_capacity(target.edits.len());
        for edit in &target.edits {
            let start = point(&snapshot, edit.range.start, strict, &target.display)?;
            let end = point(&snapshot, edit.range.end, strict, &target.display)?;
            ranges.push((start..end, edit.new_text.clone()));
        }
        plans.push((buffer.clone(), ranges));
    }
    let mut transaction = ProjectTransaction::default();
    for (buffer, ranges) in plans {
        let applied = buffer.update(cx, |buffer, cx| {
            // Its own undo step: neither the user's typing before nor after merges into it.
            buffer.finalize_last_transaction();
            buffer.start_transaction();
            buffer.edit(ranges, None, cx);
            let id = buffer.end_transaction(cx);
            buffer.finalize_last_transaction();
            id.and_then(|id| buffer.get_transaction(id).cloned())
        });
        if let Some(applied) = applied {
            transaction.0.insert(buffer, applied);
        }
    }
    keep_until_clean(&transaction, cx);
    Ok(transaction)
}

/// Edited buffers the host holds: the project holds buffers weakly, so an edit of a file no
/// editor shows would vanish with the last handle. Each is let go once it is clean again (saved,
/// or undone to its saved text).
#[derive(Default)]
struct KeptBuffers(HashMap<EntityId, (Entity<Buffer>, gpui::Subscription)>);

impl Global for KeptBuffers {}

fn keep_until_clean(transaction: &ProjectTransaction, cx: &mut App) {
    for buffer in transaction.0.keys() {
        keep_buffer_until_clean(buffer, cx);
    }
}

/// Holds an edited buffer until it is clean again (see [`KeptBuffers`]).
pub(crate) fn keep_buffer_until_clean(buffer: &Entity<Buffer>, cx: &mut App) {
    if !buffer.read(cx).is_dirty()
        || cx
            .default_global::<KeptBuffers>()
            .0
            .contains_key(&buffer.entity_id())
    {
        return;
    }
    let subscription = cx.subscribe(buffer, |buffer, event: &language::BufferEvent, cx| {
        let clean = matches!(
            event,
            language::BufferEvent::Saved | language::BufferEvent::DirtyChanged
        ) && !buffer.read(cx).is_dirty();
        if clean {
            let id = buffer.entity_id();
            cx.defer(move |cx| {
                cx.default_global::<KeptBuffers>().0.remove(&id);
            });
        }
    });
    cx.default_global::<KeptBuffers>()
        .0
        .insert(buffer.entity_id(), (buffer.clone(), subscription));
}

struct EditNotification;

/// Whether an edit gets the Show/Undo card (Q5): one that changed several files, or a file no
/// editor of the workspace shows. An edit of one open file is in plain sight, and undoing it there
/// undoes it all (one transaction).
pub(crate) fn needs_card(
    transaction: &ProjectTransaction,
    workspace: &Workspace,
    cx: &App,
) -> bool {
    let mut buffers = transaction.0.keys();
    let (Some(buffer), None) = (buffers.next(), buffers.next()) else {
        return transaction.0.len() > 1;
    };
    let buffer_id = buffer.read(cx).remote_id();
    !workspace.items_of_type::<Editor>(cx).any(|editor| {
        editor
            .read(cx)
            .buffer()
            .read(cx)
            .buffer(buffer_id)
            .is_some()
    })
}

/// The card: "<extension>: <label>, N files", with Show and Undo, when [`needs_card`]. It never
/// takes focus.
fn notify(extension_id: &str, label: &str, applied: Applied, cx: &mut App) {
    let Some(workspace) = applied.workspace.and_then(|workspace| workspace.upgrade()) else {
        return;
    };
    let transaction = applied.transaction;
    let files = transaction.0.len();
    if !needs_card(&transaction, workspace.read(cx), cx) {
        return;
    }
    let name = crate::UiExtensionRegistry::global(cx)
        .map(|registry| registry.read(cx).provenance_name(extension_id))
        .unwrap_or_else(|| crate::notifications::provenance_name(None, extension_id));
    let message = format!(
        "{name}: {label}, {files} {}",
        if files == 1 { "file" } else { "files" }
    );
    let title = SharedString::from(format!("{name}: {label}"));
    let weak_workspace = workspace.downgrade();
    let id = NotificationId::composite::<EditNotification>(SharedString::from(format!(
        "{extension_id}-{}",
        next_card()
    )));
    workspace.update(cx, |workspace, cx| {
        workspace.show_notification(id, cx, |cx| {
            let shown = transaction.clone();
            let undone = transaction.clone();
            cx.new(|cx| {
                MessageNotification::new(message, cx)
                    .primary_message("Show")
                    .primary_on_click(move |window, cx| {
                        show(&weak_workspace, &shown, title.clone(), window, cx)
                    })
                    .secondary_message("Undo")
                    .secondary_on_click(move |_, cx| undo(&undone, cx))
            })
        });
    });
}

fn next_card() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A multibuffer of the edited ranges, in which one undo reverts the whole edit.
fn show(
    workspace: &WeakEntity<Workspace>,
    transaction: &ProjectTransaction,
    title: SharedString,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    let Some(workspace) = workspace.upgrade() else {
        return;
    };
    let mut entries: Vec<_> = transaction.0.iter().collect();
    entries.sort_by_key(|(buffer, _)| buffer.read(cx).file().map(|file| file.path().clone()));
    let multibuffer = cx.new(|cx| {
        let mut multibuffer = MultiBuffer::new(Capability::ReadWrite).with_title(title.into());
        for (buffer, transaction) in &entries {
            let ranges: Vec<Range<Point>> = buffer
                .read(cx)
                .edited_ranges_for_transaction::<Point>(transaction)
                .collect();
            multibuffer.set_excerpts_for_path(
                PathKey::for_buffer(buffer, cx),
                (*buffer).clone(),
                ranges,
                editor::multibuffer_context_lines(cx),
                cx,
            );
        }
        multibuffer.push_transaction(entries.iter().copied(), cx);
        multibuffer
    });
    workspace.update(cx, |workspace, cx| {
        let project = workspace.project().clone();
        let editor = cx.new(|cx| Editor::for_multibuffer(multibuffer, Some(project), window, cx));
        workspace.add_item_to_active_pane(Box::new(editor), None, true, window, cx);
    });
}

/// Undoes each buffer's part of the edit.
pub(crate) fn undo(transaction: &ProjectTransaction, cx: &mut App) {
    for (buffer, transaction) in &transaction.0 {
        buffer.update(cx, |buffer, cx| {
            buffer.undo_transaction(transaction.id, cx);
        });
    }
}

#[cfg(test)]
#[path = "workspace_edit_tests.rs"]
mod tests;
