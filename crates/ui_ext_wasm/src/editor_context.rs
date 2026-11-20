//! The fyx:ui 0.4 `context` import over [`ContextHost`] and [`DiagnosticsSource`] (design wit-0.4
//! §B.11, plan 5.13): `context:editor` (the feed and the path conversions), `workspace:open` and
//! `workspace:diagnostics`. This side checks the capabilities, the paths and the focus rule, and
//! keeps private files out whatever the host sends; the context host debounces its feed, the
//! diagnostics changes are debounced here.

use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use futures::{StreamExt as _, channel::mpsc};
use ui_ext::{
    context::{self, ContextHost, DiagnosticsSnapshot, DiagnosticsSource, EditorContext},
    edit::TextRange,
    process::FileRef,
    v0_2::{HostError, HostErrorCode},
};

use crate::{
    listeners::ListenersHandle,
    network::{IoEvent, Running},
    process::Workspace,
};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

/// Which instance of an extension watches the context, by host and extension id: a reloaded
/// instance's `watch` replaces the old one's sink, and the old one, dropped later, leaves it.
static WATCHING: LazyLock<Mutex<HashMap<(usize, Arc<str>), u64>>> = LazyLock::new(Default::default);
/// The same for `watch-diagnostics`, by diagnostics source.
static WATCHING_DIAGNOSTICS: LazyLock<Mutex<HashMap<(usize, Arc<str>), u64>>> =
    LazyLock::new(Default::default);
/// Distinct files one debounce window collects before it reports "anything may have changed".
const MAX_PENDING_DIAGNOSTIC_FILES: usize = 16 * context::MAX_CHANGED_DIAGNOSTIC_FILES;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// One instance's tie to the editor host.
pub(crate) struct ContextHandle {
    extension_id: Arc<str>,
    host: Option<Arc<dyn ContextHost>>,
    workspace: Option<Arc<dyn Workspace>>,
    listeners: Arc<ListenersHandle>,
    sender: mpsc::UnboundedSender<EditorContext>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<EditorContext>>>,
    instance: u64,
    watching: AtomicBool,
    diagnostics: Option<Arc<dyn DiagnosticsSource>>,
    diagnostics_sender: mpsc::UnboundedSender<Vec<FileRef>>,
    diagnostics_receiver: Mutex<Option<mpsc::UnboundedReceiver<Vec<FileRef>>>>,
    watching_diagnostics: Arc<AtomicBool>,
}

impl ContextHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        host: Option<Arc<dyn ContextHost>>,
        workspace: Option<Arc<dyn Workspace>>,
        listeners: Arc<ListenersHandle>,
        diagnostics: Option<Arc<dyn DiagnosticsSource>>,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded();
        let (diagnostics_sender, diagnostics_receiver) = mpsc::unbounded();
        Self {
            extension_id,
            host,
            workspace,
            listeners,
            sender,
            receiver: Mutex::new(Some(receiver)),
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            watching: AtomicBool::new(false),
            diagnostics,
            diagnostics_sender,
            diagnostics_receiver: Mutex::new(Some(diagnostics_receiver)),
            watching_diagnostics: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.host.is_some()
    }

    pub(crate) fn diagnostics_available(&self) -> bool {
        self.diagnostics.is_some()
    }

    fn diagnostics_source(&self) -> Result<&Arc<dyn DiagnosticsSource>, HostError> {
        self.diagnostics.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "diagnostics are not available in this window",
            )
        })
    }

    fn host(&self) -> Result<&Arc<dyn ContextHost>, HostError> {
        self.host.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "the editor context is not available in this window",
            )
        })
    }

    fn key(&self, host: &Arc<dyn ContextHost>) -> (usize, Arc<str>) {
        (
            Arc::as_ptr(host) as *const () as usize,
            self.extension_id.clone(),
        )
    }

    /// Moves the host's contexts and the debounced diagnostics changes onto the io channel, on
    /// the current tokio runtime. Once.
    pub(crate) fn forward(&self, running: &Arc<Running>) {
        self.forward_diagnostics(running);
        let Some(mut receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let running = Arc::downgrade(running);
        tokio::spawn(async move {
            while let Some(context) = receiver.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::Context(context)).await;
            }
        });
    }

    /// `context.watch`.
    pub(crate) fn watch(&self, on: bool) -> Result<(), HostError> {
        let host = self.host()?;
        let key = self.key(host);
        if on {
            let sender = self.sender.clone();
            let filter = Arc::new(PrivacyFilter {
                host: host.clone(),
                workspace: self.workspace.clone(),
            });
            host.watch(
                &self.extension_id,
                Some(Arc::new(move |context| {
                    sender.unbounded_send(filter.apply(context)).ok();
                })),
            );
            WATCHING
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, self.instance);
            self.watching.store(true, Ordering::SeqCst);
        } else {
            self.stop();
        }
        Ok(())
    }

    /// Stops this instance's feed, if it is the one watching.
    pub(crate) fn stop(&self) {
        let Some(host) = &self.host else {
            return;
        };
        if !self.watching.swap(false, Ordering::SeqCst) {
            return;
        }
        let key = self.key(host);
        let mut watching = WATCHING.lock().unwrap_or_else(PoisonError::into_inner);
        if watching.get(&key) == Some(&self.instance) {
            watching.remove(&key);
            drop(watching);
            host.watch(&self.extension_id, None);
        }
    }

    fn forward_diagnostics(&self, running: &Arc<Running>) {
        let Some(source) = self.diagnostics.clone() else {
            return;
        };
        let Some(receiver) = self
            .diagnostics_receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let running = Arc::downgrade(running);
        let watching = self.watching_diagnostics.clone();
        let filter = ChangedFilter {
            source,
            workspace: self.workspace.clone(),
        };
        tokio::spawn(async move {
            let mut changes = std::pin::pin!(debounced_changes(receiver));
            while let Some(changed) = changes.next().await {
                if !watching.load(Ordering::SeqCst) {
                    continue;
                }
                let Some(files) = filter.apply(changed) else {
                    continue;
                };
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::DiagnosticsChanged(files)).await;
            }
        });
    }

    /// `context.diagnostics`: one worktree file's (`Some`), or those of every worktree file with
    /// any; a private file is `not-permitted` by name and left out of the whole answer.
    pub(crate) fn diagnostics(
        &self,
        path: Option<FileRef>,
    ) -> Result<DiagnosticsSnapshot, HostError> {
        let source = self.diagnostics_source()?;
        let roots = self.roots()?;
        let files = match path {
            Some(path) => {
                path.check()?;
                let (worktree, full) = resolve(&roots, &path)?;
                if hidden(&**source, &full) {
                    return Err(error(HostErrorCode::NotPermitted, "a private file"));
                }
                let path = FileRef {
                    worktree: Some(worktree),
                    path: path.path,
                };
                let mut files = source.diagnostics(&self.extension_id, Some(&path));
                files.retain(|file| file.path == path);
                files
            }
            None => {
                let mut files = source.diagnostics(&self.extension_id, None);
                files.retain_mut(|file| {
                    let Some((worktree, full)) = worktree_file(&roots, &file.path) else {
                        return false;
                    };
                    file.path.worktree = Some(worktree);
                    !hidden(&**source, &full)
                });
                files
            }
        };
        Ok(context::diagnostics_snapshot(files))
    }

    /// `context.watch-diagnostics`.
    pub(crate) fn watch_diagnostics(&self, on: bool) -> Result<(), HostError> {
        let source = self.diagnostics_source()?;
        if !on {
            self.stop_diagnostics();
            return Ok(());
        }
        let sender = self.diagnostics_sender.clone();
        source.watch(
            &self.extension_id,
            Some(Arc::new(move |files| {
                sender.unbounded_send(files).ok();
            })),
        );
        WATCHING_DIAGNOSTICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(source_key(source, &self.extension_id), self.instance);
        self.watching_diagnostics.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Stops this instance's diagnostics feed, if it is the one watching.
    pub(crate) fn stop_diagnostics(&self) {
        let Some(source) = &self.diagnostics else {
            return;
        };
        if !self.watching_diagnostics.swap(false, Ordering::SeqCst) {
            return;
        }
        let key = source_key(source, &self.extension_id);
        let mut watching = WATCHING_DIAGNOSTICS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if watching.get(&key) == Some(&self.instance) {
            watching.remove(&key);
            drop(watching);
            source.watch(&self.extension_id, None);
        }
    }

    /// `workspace.open`: a file of a worktree. `focus` is kept only with a gesture, or with a
    /// listener request less than 2 s old while one of the extension's terminals has focus;
    /// otherwise the file opens in the background.
    pub(crate) fn open(
        &self,
        path: FileRef,
        range: Option<TextRange>,
        focus: bool,
        gesture: bool,
    ) -> Result<(), HostError> {
        let host = self.host()?;
        path.check()?;
        let absolute = self.absolute(&path)?;
        if !absolute.is_file() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{:?} is not a file", path.path),
            ));
        }
        let focus = focus
            && (gesture
                || (self
                    .listeners
                    .requested_within(context::FOCUS_REQUEST_WINDOW)
                    && host.terminal_focused(&self.extension_id)));
        host.open(&self.extension_id, path, range, focus)
            .map_err(|why| error(HostErrorCode::Unavailable, why))
    }

    /// `context.to-file-ref`: a tool's absolute path or `file://` URI in one of the worktrees.
    pub(crate) fn to_file_ref(&self, absolute: &str) -> Result<FileRef, HostError> {
        let host = self.host()?;
        let parsed = context::parse_tool_path(absolute).ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                "not an absolute path or file URI",
            )
        })?;
        let roots = self.roots()?;
        let file_ref = context::file_ref_in(
            &parsed,
            roots.iter().map(|(id, root)| (*id, root.as_path())),
        )
        .ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                "the path is outside every open worktree",
            )
        })?;
        let full = self.absolute(&file_ref)?;
        if private(&**host, &full) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        Ok(file_ref)
    }

    /// `context.absolute-path`: a worktree file's absolute path, for a tool's reply.
    pub(crate) fn absolute_path(&self, path: &FileRef) -> Result<String, HostError> {
        let host = self.host()?;
        path.check()?;
        let absolute = self.absolute(path)?;
        if private(&**host, &absolute) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        Ok(absolute.to_string_lossy().into_owned())
    }

    fn roots(&self) -> Result<Vec<(u64, PathBuf)>, HostError> {
        roots(self.workspace.as_deref())
    }

    fn absolute(&self, path: &FileRef) -> Result<PathBuf, HostError> {
        absolute(&self.roots()?, path)
    }
}

impl Drop for ContextHandle {
    /// The instance is gone: its feeds stop, unless a newer instance took them over.
    fn drop(&mut self) {
        self.stop();
        self.stop_diagnostics();
    }
}

pub(crate) fn roots(workspace: Option<&dyn Workspace>) -> Result<Vec<(u64, PathBuf)>, HostError> {
    let roots: Vec<(u64, PathBuf)> = workspace
        .map(|workspace| {
            workspace
                .worktrees()
                .into_iter()
                .map(|worktree| (worktree.id, worktree.root))
                .collect()
        })
        .unwrap_or_default();
    if roots.is_empty() {
        return Err(error(HostErrorCode::NoWorktree, "no worktree is open"));
    }
    Ok(roots)
}

fn source_key(source: &Arc<dyn DiagnosticsSource>, extension_id: &Arc<str>) -> (usize, Arc<str>) {
    (
        Arc::as_ptr(source) as *const () as usize,
        extension_id.clone(),
    )
}

/// A checked `file-ref`'s absolute path: in its worktree, or in the one worktree that has it.
pub(crate) fn absolute(roots: &[(u64, PathBuf)], path: &FileRef) -> Result<PathBuf, HostError> {
    resolve(roots, path).map(|(_, full)| full)
}

/// A host's `file-ref`, if it is a valid path in a known worktree: that worktree and the
/// absolute path.
fn worktree_file(roots: &[(u64, PathBuf)], path: &FileRef) -> Option<(u64, PathBuf)> {
    path.check().ok()?;
    resolve(roots, path).ok()
}

/// [`absolute`], with the worktree that holds the file.
/// A file the extension may not see through the editor context: private, or in an account
/// profile's dir (§C.6).
fn private(host: &dyn ContextHost, full: &std::path::Path) -> bool {
    host.is_private(full) || crate::accounts::in_account_dir(full)
}

/// [`private`] for diagnostics.
fn hidden(source: &dyn DiagnosticsSource, full: &std::path::Path) -> bool {
    source.is_private(full) || crate::accounts::in_account_dir(full)
}

pub(crate) fn resolve(
    roots: &[(u64, PathBuf)],
    path: &FileRef,
) -> Result<(u64, PathBuf), HostError> {
    let join = |root: &PathBuf| {
        path.path
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(root.clone(), |full, part| full.join(part))
    };
    match path.worktree {
        Some(id) => roots
            .iter()
            .find(|(root_id, _)| *root_id == id)
            .map(|(id, root)| (*id, join(root)))
            .ok_or_else(|| error(HostErrorCode::InvalidArgument, "an unknown worktree")),
        None => {
            let mut found = roots
                .iter()
                .map(|(id, root)| (*id, join(root)))
                .filter(|(_, full)| full.exists());
            match (found.next(), found.next()) {
                (Some(found), None) => Ok(found),
                (None, _) => Err(error(
                    HostErrorCode::InvalidArgument,
                    format!("no worktree has {:?}", path.path),
                )),
                (Some(_), Some(_)) => Err(error(
                    HostErrorCode::InvalidArgument,
                    format!("more than one worktree has {:?}; name one", path.path),
                )),
            }
        }
    }
}

/// What every context passes before the extension sees it: worktree files only, no text of a
/// private active file, and the text capped.
struct PrivacyFilter {
    host: Arc<dyn ContextHost>,
    workspace: Option<Arc<dyn Workspace>>,
}

impl PrivacyFilter {
    fn apply(&self, mut context: EditorContext) -> EditorContext {
        let roots = roots(self.workspace.as_deref()).unwrap_or_default();
        let in_worktree = |file: &FileRef| file.check().is_ok() && absolute(&roots, file).is_ok();
        context.open.retain(|file| in_worktree(file));
        if context
            .active
            .as_ref()
            .is_some_and(|file| !in_worktree(file))
        {
            context.active = None;
            context.selection = None;
            context.trusted = None;
        }
        let private = context.active.as_ref().is_none_or(|file| {
            absolute(&roots, file).map_or(true, |full| private(&*self.host, &full))
        });
        if let Some((_, text)) = &mut context.selection {
            if private {
                *text = None;
            } else if let Some(text) = text {
                context::cap_selection(text);
            }
        }
        context
    }
}

/// One debounce window's changes: the distinct files, or anything.
#[derive(Debug, Default, PartialEq, Eq)]
struct Changed {
    anything: bool,
    files: BTreeSet<(Option<u64>, String)>,
}

impl Changed {
    fn add(&mut self, files: Vec<FileRef>) {
        if files.is_empty() {
            self.anything = true;
        }
        if self.anything {
            self.files.clear();
            return;
        }
        self.files
            .extend(files.into_iter().map(|file| (file.worktree, file.path)));
        if self.files.len() > MAX_PENDING_DIAGNOSTIC_FILES {
            self.anything = true;
            self.files.clear();
        }
    }
}

/// The host's change reports, collected for [`context::DIAGNOSTICS_DEBOUNCE`] from the first
/// report of each window.
fn debounced_changes(
    receiver: mpsc::UnboundedReceiver<Vec<FileRef>>,
) -> impl futures::Stream<Item = Changed> {
    futures::stream::unfold(receiver, |mut receiver| async move {
        let first = receiver.next().await?;
        let mut changed = Changed::default();
        changed.add(first);
        let deadline = tokio::time::Instant::now() + context::DIAGNOSTICS_DEBOUNCE;
        while let Ok(Some(more)) = tokio::time::timeout_at(deadline, receiver.next()).await {
            changed.add(more);
        }
        Some((changed, receiver))
    })
}

/// What a `diagnostics-changed` names: worktree files that are not private, at most
/// [`context::MAX_CHANGED_DIAGNOSTIC_FILES`] (more, or anything, is the empty list). `None` when
/// nothing is left to tell.
struct ChangedFilter {
    source: Arc<dyn DiagnosticsSource>,
    workspace: Option<Arc<dyn Workspace>>,
}

impl ChangedFilter {
    fn apply(&self, changed: Changed) -> Option<Vec<FileRef>> {
        if changed.anything {
            return Some(Vec::new());
        }
        let roots = roots(self.workspace.as_deref()).ok()?;
        let mut files: Vec<FileRef> = Vec::new();
        for (worktree, path) in changed.files {
            let Some((worktree, full)) = worktree_file(
                &roots,
                &FileRef {
                    worktree,
                    path: path.clone(),
                },
            ) else {
                continue;
            };
            let file = FileRef {
                worktree: Some(worktree),
                path,
            };
            if !hidden(&*self.source, &full) && !files.contains(&file) {
                files.push(file);
            }
        }
        if files.len() > context::MAX_CHANGED_DIAGNOSTIC_FILES {
            return Some(Vec::new());
        }
        (!files.is_empty()).then_some(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileRef {
        FileRef {
            worktree: Some(1),
            path: path.into(),
        }
    }

    #[test]
    fn changes_collect_distinct_files_or_anything() {
        let mut changed = Changed::default();
        changed.add(vec![file("a.rs"), file("b.rs")]);
        changed.add(vec![file("a.rs")]);
        assert_eq!(changed.files.len(), 2);
        assert!(!changed.anything);
        changed.add(Vec::new());
        changed.add(vec![file("c.rs")]);
        assert!(changed.anything && changed.files.is_empty());

        let mut many = Changed::default();
        many.add(
            (0..=MAX_PENDING_DIAGNOSTIC_FILES)
                .map(|index| file(&format!("{index}.rs")))
                .collect(),
        );
        assert!(many.anything && many.files.is_empty());
    }

    #[tokio::test]
    async fn changes_are_debounced_from_the_first_report() {
        let (sender, receiver) = mpsc::unbounded();
        let mut changes = std::pin::pin!(debounced_changes(receiver));
        let started = std::time::Instant::now();
        sender.unbounded_send(vec![file("a.rs")]).unwrap();
        let later = sender.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            later.unbounded_send(vec![file("b.rs")]).unwrap();
        });
        let first = changes.next().await.unwrap();
        let waited = started.elapsed();
        assert!(waited >= context::DIAGNOSTICS_DEBOUNCE, "{waited:?}");
        assert_eq!(
            first
                .files
                .into_iter()
                .map(|(_, path)| path)
                .collect::<Vec<_>>(),
            ["a.rs", "b.rs"]
        );
        sender.unbounded_send(vec![file("c.rs")]).unwrap();
        drop(sender);
        let second = changes.next().await.unwrap();
        assert_eq!(second.files.len(), 1);
        assert!(changes.next().await.is_none());
    }
}
