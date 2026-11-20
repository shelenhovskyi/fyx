//! The editor side of the marks an extension puts on the user's files (design wit-0.4 §E.2,
//! §F.3, plans 6.6 and 6.7): W2c's [`DiagnosticsPublisher`] (`diagnostics:publish`) and
//! [`FileDecorationsHost`] (`workspace:file-decorations`). The runtime has checked the capability,
//! the paths (worktree files, not private) and the limits.
//!
//! **Diagnostics** go into the project's language-server store as if a language server had sent
//! them, so the editor, the diagnostics panel and the status bar show them unchanged. Each load
//! gets its own [`LanguageServerId`] per project, never registered as a server (nothing sends it
//! requests); `source` is the extension's display name. Ranges arrive as 0-based lines with UTF-8
//! byte columns and are clamped to the file's text: the open buffer's, or the file on disk. The
//! load keeps the files it published, so clearing (an empty list, `clear-diagnostics`, the
//! instance's end) publishes empty lists for them. Local projects only.
//!
//! **Decorations** go into `project`'s [`FileDecorations`] global, which the project panel
//! draws; the load's decorations go away with it.
//!
//! Each load has its own host objects (keyed like its worktree ids). The extension thread waits
//! for the main thread (see [`MainThread`]).

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use collections::{HashMap, HashSet};
use gpui::{App, AsyncApp, Entity, EntityId, SharedString, WeakEntity};
use language::{Bias, DiagnosticEntry, LanguageServerId, Point, PointUtf16, Rope, Unclipped};
use project::{
    Project, ProjectPath,
    fyx_file_decorations::{FileDecoration as PanelDecoration, FileDecorationColor, FileDecorations},
    lsp_store::{DocumentDiagnostics, DocumentDiagnosticsUpdate},
};
use ui_ext::{
    ColorToken,
    context::{Diagnostic, DiagnosticSeverity},
    edit::Position,
    file_decorations::{FileDecoration, FileDecorationsHost},
    process::FileRef,
    published_diagnostics::DiagnosticsPublisher,
};

use crate::{context_host::locate, main_thread::MainThread};

/// How long an extension thread waits for a publish (a closed file is read from disk).
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);
const DECORATE_TIMEOUT: Duration = Duration::from_secs(5);
const PRIVATE_TIMEOUT: Duration = Duration::from_secs(2);

/// Group ids of published entries: each entry is a group of its own, far from the ids language
/// servers' entries get.
static NEXT_GROUP: AtomicUsize = AtomicUsize::new(1 << 30);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn is_private(main: &MainThread, absolute: &Path) -> bool {
    let absolute = absolute.to_path_buf();
    // Not answered in time: private, so nothing is marked.
    main.call(PRIVATE_TIMEOUT, move |cx, reply| {
        reply.send(crate::workspace_edit::is_private_path(&absolute, cx));
    })
    .unwrap_or(true)
}

/// The extension's display name, for `source` and tooltips.
fn display_name(extension_id: &str, cx: &App) -> SharedString {
    crate::UiExtensionRegistry::global(cx)
        .map(|registry| registry.read(cx).provenance_name(extension_id))
        .unwrap_or_else(|| extension_id.to_string().into())
}

/// The project and absolute path of a worktree file of the load keyed by `key`.
fn resolve(key: &str, file: &FileRef, cx: &mut App) -> Result<(Entity<Project>, PathBuf), String> {
    let (worktree, abs_path) = locate(key, file, cx)?;
    let (_, project) = crate::workspace_edit::registered(&worktree, cx)
        .ok_or("the file's project is not open")?;
    Ok((project, abs_path))
}

/// The [`DiagnosticsPublisher`] of one load. `key` is what
/// [`crate::worktrees::OpenWorktrees::for_extension`] was given.
pub fn publisher_for_extension(key: &str, cx: &mut App) -> Arc<dyn DiagnosticsPublisher> {
    Arc::new(ExtensionPublisher {
        key: key.into(),
        main: MainThread::global(cx),
        state: Arc::default(),
    })
}

#[derive(Default)]
struct Published {
    /// The load's server id in each project it published to.
    servers: HashMap<EntityId, (WeakEntity<Project>, LanguageServerId)>,
    /// The files that have entries now, with their project.
    files: HashMap<PathBuf, EntityId>,
}

struct ExtensionPublisher {
    key: Arc<str>,
    main: MainThread,
    state: Arc<Mutex<Published>>,
}

impl DiagnosticsPublisher for ExtensionPublisher {
    fn publish(
        &self,
        extension_id: &str,
        path: FileRef,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<(), String> {
        let key = self.key.clone();
        let extension_id = extension_id.to_string();
        let state = self.state.clone();
        self.main
            .call(PUBLISH_TIMEOUT, move |cx, reply| {
                cx.spawn(async move |cx| {
                    reply.send(publish(&key, &extension_id, path, diagnostics, &state, cx).await);
                })
                .detach();
            })
            .unwrap_or_else(|| Err("the editor did not answer in time".into()))
    }

    fn clear_all(&self, _extension_id: &str) {
        let state = self.state.clone();
        self.main.post(move |cx| clear_all(&state, cx));
    }

    fn is_private(&self, absolute: &Path) -> bool {
        is_private(&self.main, absolute)
    }
}

impl Drop for ExtensionPublisher {
    fn drop(&mut self) {
        let state = self.state.clone();
        self.main.post(move |cx| clear_all(&state, cx));
    }
}

/// Replaces the load's entries of one file; an empty list clears it.
async fn publish(
    key: &str,
    extension_id: &str,
    path: FileRef,
    diagnostics: Vec<Diagnostic>,
    state: &Arc<Mutex<Published>>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let (project, abs_path, open_text, name) = cx.update(|cx| {
        let (project, abs_path) = resolve(key, &path, cx)?;
        let open_text = project
            .read(cx)
            .find_project_path(&abs_path, cx)
            .and_then(|project_path| project.read(cx).get_open_buffer(&project_path, cx))
            .map(|buffer| buffer.read(cx).as_rope().clone());
        Ok::<_, String>((project, abs_path, open_text, display_name(extension_id, cx)))
    })?;
    let text = match open_text {
        Some(text) => text,
        None if diagnostics.is_empty() => Rope::new(),
        None => {
            let fs = project.read_with(cx, |project, _| project.fs().clone());
            let loaded = fs
                .load(&abs_path)
                .await
                .map_err(|error| format!("{} could not be read: {error:#}", path.path))?;
            Rope::from(loaded.as_str())
        }
    };
    let entries = diagnostics
        .into_iter()
        .map(|diagnostic| entry(diagnostic, &text, &name))
        .collect::<Vec<_>>();
    cx.update(|cx| {
        let server_id = server_for(state, &project, cx);
        merge(&project, server_id, abs_path.clone(), entries.clone(), cx)?;
        let mut state = lock(state);
        if entries.is_empty() {
            state.files.remove(&abs_path);
        } else {
            state.files.insert(abs_path, project.entity_id());
        }
        Ok(())
    })
}

/// The load's server id in `project`, made on first use.
fn server_for(state: &Mutex<Published>, project: &Entity<Project>, cx: &App) -> LanguageServerId {
    let mut state = lock(state);
    if let Some((_, server_id)) = state.servers.get(&project.entity_id()) {
        return *server_id;
    }
    let server_id = project.read(cx).languages().next_language_server_id();
    state
        .servers
        .insert(project.entity_id(), (project.downgrade(), server_id));
    server_id
}

fn merge(
    project: &Entity<Project>,
    server_id: LanguageServerId,
    abs_path: PathBuf,
    entries: Vec<DiagnosticEntry<Unclipped<PointUtf16>>>,
    cx: &mut App,
) -> Result<(), String> {
    let lsp_store = project.read(cx).lsp_store();
    lsp_store
        .update(cx, |lsp_store, cx| {
            lsp_store.merge_diagnostic_entries(
                vec![DocumentDiagnosticsUpdate {
                    diagnostics: DocumentDiagnostics::fyx_new(abs_path, entries),
                    result_id: None,
                    registration_id: None,
                    server_id,
                    disk_based_sources: Cow::Borrowed(&[]),
                }],
                // Nothing earlier of this load's server stays.
                |_, _, _| false,
                cx,
            )
        })
        .map_err(|error| format!("{error:#}"))
}

fn clear_all(state: &Mutex<Published>, cx: &mut App) {
    let (files, servers) = {
        let mut state = lock(state);
        (std::mem::take(&mut state.files), state.servers.clone())
    };
    for (abs_path, project_id) in files {
        let Some((project, server_id)) = servers
            .get(&project_id)
            .and_then(|(project, server_id)| Some((project.upgrade()?, *server_id)))
        else {
            continue;
        };
        if let Err(error) = merge(&project, server_id, abs_path, Vec::new(), cx) {
            log::warn!("fyx: clearing an extension's diagnostics failed: {error}");
        }
    }
}

/// An extension's entry as Zed keeps it: the range clamped to `text` and in UTF-16, `source` the
/// extension's name, a group of its own.
fn entry(
    diagnostic: Diagnostic,
    text: &Rope,
    name: &SharedString,
) -> DiagnosticEntry<Unclipped<PointUtf16>> {
    let start = point_utf16(diagnostic.range.start, text);
    let end = point_utf16(diagnostic.range.end, text).max(start);
    let defaults = language::Diagnostic::default();
    // lsp's severity and code are not reachable from here by name: their wire forms are.
    let severity = serde_json::from_value(serde_json::Value::from(match diagnostic.severity {
        DiagnosticSeverity::Error => 1,
        DiagnosticSeverity::Warning => 2,
        DiagnosticSeverity::Information => 3,
        DiagnosticSeverity::Hint => 4,
    }))
    .unwrap_or(defaults.severity);
    let code = diagnostic
        .code
        .and_then(|code| serde_json::from_value(serde_json::Value::String(code)).ok());
    DiagnosticEntry {
        range: Unclipped(start)..Unclipped(end),
        diagnostic: language::Diagnostic {
            source: Some(name.to_string()),
            code,
            severity,
            message: diagnostic.message.into(),
            group_id: NEXT_GROUP.fetch_add(1, Ordering::Relaxed),
            is_primary: true,
            ..defaults
        },
        related_information: None,
    }
}

/// A 0-based line and UTF-8 byte column, clamped to `text`: a line past the end is the end, a
/// column past the line is its end, inside a character its start.
fn point_utf16(position: Position, text: &Rope) -> PointUtf16 {
    let max = text.max_point();
    let point = if position.line > max.row {
        max
    } else {
        text.clip_point(Point::new(position.line, position.column), Bias::Left)
    };
    text.point_to_point_utf16(point)
}

/// The [`FileDecorationsHost`] of one load.
pub fn decorations_for_extension(key: &str, cx: &mut App) -> Arc<dyn FileDecorationsHost> {
    Arc::new(ExtensionDecorations {
        key: key.into(),
        main: MainThread::global(cx),
        decorated: Arc::default(),
    })
}

struct ExtensionDecorations {
    key: Arc<str>,
    main: MainThread,
    /// The extension ids this load decorated for (one, in practice).
    decorated: Arc<Mutex<HashSet<String>>>,
}

impl FileDecorationsHost for ExtensionDecorations {
    fn decorate(&self, extension_id: &str, decorations: Vec<FileDecoration>) -> Result<(), String> {
        let key = self.key.clone();
        let extension_id = extension_id.to_string();
        let decorated = self.decorated.clone();
        self.main
            .call(DECORATE_TIMEOUT, move |cx, reply| {
                reply.send(decorate(&key, &extension_id, decorations, &decorated, cx));
            })
            .unwrap_or_else(|| Err("the editor did not answer in time".into()))
    }

    fn clear(&self, extension_id: &str, paths: Option<Vec<FileRef>>) {
        let key = self.key.clone();
        let extension_id = extension_id.to_string();
        let decorated = self.decorated.clone();
        self.main
            .post(move |cx| clear(&key, &extension_id, paths, &decorated, cx));
    }

    fn is_private(&self, absolute: &Path) -> bool {
        is_private(&self.main, absolute)
    }
}

impl Drop for ExtensionDecorations {
    fn drop(&mut self) {
        let decorated = std::mem::take(&mut *lock(&self.decorated));
        if decorated.is_empty() {
            return;
        }
        self.main.post(move |cx| {
            if let Some(decorations) = FileDecorations::try_global(cx) {
                decorations.update(cx, |decorations, cx| {
                    for extension_id in &decorated {
                        decorations.clear(extension_id, None, cx);
                    }
                });
            }
        });
    }
}

fn decorate(
    key: &str,
    extension_id: &str,
    decorations: Vec<FileDecoration>,
    decorated: &Mutex<HashSet<String>>,
    cx: &mut App,
) -> Result<(), String> {
    let marks = decorations
        .into_iter()
        .map(|decoration| Ok((project_path(key, &decoration.path, cx)?, panel(decoration))))
        .collect::<Result<Vec<_>, String>>()?;
    let name = display_name(extension_id, cx);
    lock(decorated).insert(extension_id.to_string());
    FileDecorations::global(cx).update(cx, |decorations, cx| {
        decorations.decorate(extension_id, name, marks, cx);
    });
    Ok(())
}

fn clear(
    key: &str,
    extension_id: &str,
    paths: Option<Vec<FileRef>>,
    decorated: &Mutex<HashSet<String>>,
    cx: &mut App,
) {
    let paths = paths.map(|paths| {
        paths
            .iter()
            .filter_map(|path| project_path(key, path, cx).ok())
            .collect::<Vec<_>>()
    });
    if paths.is_none() {
        lock(decorated).remove(extension_id);
    }
    if let Some(decorations) = FileDecorations::try_global(cx) {
        decorations.update(cx, |decorations, cx| {
            decorations.clear(extension_id, paths.as_deref(), cx);
        });
    }
}

fn project_path(key: &str, file: &FileRef, cx: &mut App) -> Result<ProjectPath, String> {
    let (project, abs_path) = resolve(key, file, cx)?;
    project
        .read(cx)
        .find_project_path(&abs_path, cx)
        .ok_or_else(|| format!("{} is in no worktree", file.path))
}

fn panel(decoration: FileDecoration) -> PanelDecoration {
    PanelDecoration {
        badge: decoration.badge.map(Into::into),
        color: decoration.color.and_then(|color| match color {
            ColorToken::TextAccent => Some(FileDecorationColor::Accent),
            ColorToken::Warning => Some(FileDecorationColor::Warning),
            ColorToken::Error => Some(FileDecorationColor::Error),
            ColorToken::Success => Some(FileDecorationColor::Success),
            ColorToken::TextMuted => Some(FileDecorationColor::Muted),
            // Refused by the runtime's check.
            ColorToken::Text
            | ColorToken::Border
            | ColorToken::Surface
            | ColorToken::ElevatedSurface => None,
        }),
        tooltip: decoration.tooltip.map(Into::into),
    }
}

#[cfg(test)]
#[path = "extension_marks_tests.rs"]
mod tests;
