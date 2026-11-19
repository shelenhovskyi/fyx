//! The language-server side of the 0.4 `context.diagnostics` and `watch-diagnostics` (W2c's
//! [`DiagnosticsSource`], design wit-0.4 §B.11, plan 5.13).
//!
//! What the projects' language servers report is kept in a cache that extension threads read
//! without the main thread. The cache starts filling the first time an extension asks (or
//! watches) and then follows every registered project: each `DiagnosticsUpdated` re-reads the
//! named files, and the end of a server's check re-reads every file (Zed sends no update for a
//! file whose errors and warnings stay at zero, so information and hints catch up then). An open buffer is read as it is now (its entries follow edits); a closed file's
//! entries come from the language-server store (a `fyx:` accessor in `LspStore`) with their
//! UTF-16 columns converted to UTF-8 bytes against the file on disk.
//!
//! Only primary entries are reported: Zed adds a server's related information as secondary
//! entries of the same group, which are not diagnostics of their own. `source` is the
//! diagnostic's own `source`, or the server's name when it gave none.
//!
//! A watching extension hears the files whose entries changed (its own `FileRef`s, files outside
//! every worktree left out), and an empty list when a language server stopped.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use collections::HashMap;
use futures::{FutureExt as _, future::Shared};
use gpui::{App, AppContext as _, Entity, EntityId, Global, Subscription, Task, WeakEntity};
use language::{
    BufferSnapshot, DiagnosticEntry, LanguageServerId, Point, PointUtf16, ToPoint as _, Unclipped,
};
use project::{Project, ProjectPath};
use ui_ext::{
    context::{
        Diagnostic, DiagnosticSeverity, DiagnosticsSink, DiagnosticsSource, FileDiagnostics,
    },
    edit::{Position, TextRange},
    process::FileRef,
};

use crate::{main_thread::MainThread, worktrees::OpenWorktrees};

/// How long an extension thread waits for the first fill of the cache.
const FILL_TIMEOUT: Duration = Duration::from_secs(10);
const PRIVATE_TIMEOUT: Duration = Duration::from_secs(2);

/// One file's entries, and which project reported them.
#[derive(Clone)]
struct Cached {
    project: EntityId,
    path: ProjectPath,
    diagnostics: Vec<Diagnostic>,
}

/// Every file with diagnostics, by absolute path; read from extension threads.
#[derive(Clone, Default)]
struct Cache(Arc<RwLock<HashMap<PathBuf, Cached>>>);

impl Cache {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<PathBuf, Cached>> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<PathBuf, Cached>> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One load's change feed.
struct Watcher {
    /// What its worktree ids are keyed by (the extension id, or a dev extension's path).
    key: Arc<str>,
    sink: Mutex<Option<DiagnosticsSink>>,
}

impl Watcher {
    fn sink(&self) -> Option<DiagnosticsSink> {
        self.sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Feed {
    cache: Cache,
    /// Set once the cache follows the projects.
    active: Arc<AtomicBool>,
    /// The first fill, which callers wait for.
    fill: Option<Shared<Task<()>>>,
    watchers: Vec<Weak<Watcher>>,
    projects: HashMap<EntityId, (WeakEntity<Project>, Subscription)>,
    /// The latest read of each file: a read that finishes after a newer one is dropped.
    generations: HashMap<PathBuf, u64>,
    next_generation: u64,
}

impl Global for Feed {}

/// The [`DiagnosticsSource`] of one load. `key` is what
/// [`crate::worktrees::OpenWorktrees::for_extension`] was given.
pub(crate) fn for_extension(key: &str, cx: &mut App) -> Arc<dyn DiagnosticsSource> {
    let watcher = Arc::new(Watcher {
        key: key.into(),
        sink: Mutex::new(None),
    });
    let open = OpenWorktrees::global(cx);
    let main = MainThread::global(cx);
    let feed = cx.default_global::<Feed>();
    feed.watchers.retain(|watcher| watcher.strong_count() > 0);
    feed.watchers.push(Arc::downgrade(&watcher));
    Arc::new(ExtensionDiagnostics {
        main,
        open,
        cache: feed.cache.clone(),
        active: feed.active.clone(),
        watcher,
    })
}

struct ExtensionDiagnostics {
    main: MainThread,
    open: OpenWorktrees,
    cache: Cache,
    active: Arc<AtomicBool>,
    watcher: Arc<Watcher>,
}

impl ExtensionDiagnostics {
    /// Starts following the projects and waits for the first fill (off the main thread only).
    fn ensure_active(&self) {
        if self.active.load(Ordering::Acquire) {
            return;
        }
        self.main.call(FILL_TIMEOUT, |cx, reply| {
            let fill = activate(cx);
            cx.spawn(async move |_| {
                fill.await;
                reply.send(());
            })
            .detach();
        });
    }
}

impl DiagnosticsSource for ExtensionDiagnostics {
    fn diagnostics(&self, _extension_id: &str, path: Option<&FileRef>) -> Vec<FileDiagnostics> {
        self.ensure_active();
        read_cache(&self.cache, &self.open, &self.watcher.key, path)
    }

    fn watch(&self, _extension_id: &str, sink: Option<DiagnosticsSink>) {
        let start = sink.is_some();
        *self
            .watcher
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = sink;
        if start && !self.active.load(Ordering::Acquire) {
            // The feed keeps the fill running.
            self.main.post(|cx| {
                let _fill = activate(cx);
            });
        }
    }

    fn is_private(&self, absolute: &Path) -> bool {
        let absolute = absolute.to_path_buf();
        // Not answered in time: private, so nothing of it leaves.
        self.main
            .call(PRIVATE_TIMEOUT, move |cx, reply| {
                reply.send(crate::workspace_edit::is_private_path(&absolute, cx));
            })
            .unwrap_or(true)
    }
}

/// The cache as the extension keyed by `key` sees it: one file, or every file in a worktree.
fn read_cache(
    cache: &Cache,
    open: &OpenWorktrees,
    key: &str,
    path: Option<&FileRef>,
) -> Vec<FileDiagnostics> {
    let cache = cache.read();
    match path {
        Some(file) => {
            let Ok((_, abs_path)) = crate::context_host::locate_in(open, key, file) else {
                return Vec::new();
            };
            let Some(cached) = cache.get(&abs_path) else {
                return Vec::new();
            };
            let Some(path) = open.file_ref(key, &abs_path) else {
                return Vec::new();
            };
            vec![FileDiagnostics {
                path,
                diagnostics: cached.diagnostics.clone(),
            }]
        }
        None => cache
            .iter()
            .filter_map(|(abs_path, cached)| {
                Some(FileDiagnostics {
                    path: open.file_ref(key, abs_path)?,
                    diagnostics: cached.diagnostics.clone(),
                })
            })
            .collect(),
    }
}

/// Follows a workspace's project (cheap until an extension asks for diagnostics).
pub(crate) fn watch_project(project: &Entity<Project>, cx: &mut App) {
    let id = project.entity_id();
    if cx.default_global::<Feed>().projects.contains_key(&id) {
        return;
    }
    let subscription = cx.subscribe(project, |project, event, cx| {
        if !cx.default_global::<Feed>().active.load(Ordering::Acquire) {
            return;
        }
        match event {
            project::Event::DiagnosticsUpdated { paths, .. } => {
                refresh(&project, paths.clone(), false, cx).detach();
            }
            // Zed sends no `DiagnosticsUpdated` for a file whose errors and warnings stay at
            // zero: information and hints catch up when a server finishes a check.
            project::Event::DiskBasedDiagnosticsFinished { .. } => {
                let paths = all_paths(&project, cx);
                refresh(&project, paths, false, cx).detach();
            }
            project::Event::LanguageServerRemoved(_) => {
                let paths = all_paths(&project, cx);
                refresh(&project, paths, true, cx).detach();
            }
            _ => {}
        }
    });
    cx.observe_release(project, move |_, cx| {
        let feed = cx.default_global::<Feed>();
        feed.projects.remove(&id);
        feed.cache.write().retain(|_, cached| cached.project != id);
    })
    .detach();
    let feed = cx.default_global::<Feed>();
    feed.projects
        .insert(id, (project.downgrade(), subscription));
    if feed.active.load(Ordering::Acquire) {
        let paths = reported_paths(project, cx);
        refresh(project, paths, false, cx).detach();
    }
}

/// Starts following every project; the returned task ends with the first fill.
fn activate(cx: &mut App) -> Shared<Task<()>> {
    let feed = cx.default_global::<Feed>();
    if let Some(fill) = &feed.fill {
        return fill.clone();
    }
    feed.active.store(true, Ordering::Release);
    let projects = feed
        .projects
        .values()
        .filter_map(|(project, _)| project.upgrade())
        .collect::<Vec<_>>();
    let reads = projects
        .iter()
        .map(|project| {
            let paths = reported_paths(project, cx);
            refresh(project, paths, false, cx)
        })
        .collect::<Vec<_>>();
    let fill = cx
        .foreground_executor()
        .spawn(futures::future::join_all(reads).map(|_| ()))
        .shared();
    cx.default_global::<Feed>().fill = Some(fill.clone());
    fill
}

/// The files the project's language servers report diagnostics for (only those with errors or
/// warnings in a remote project).
fn reported_paths(project: &Entity<Project>, cx: &App) -> Vec<ProjectPath> {
    let project = project.read(cx);
    let mut paths = match project.lsp_store().read(cx).as_local() {
        Some(local) => local.fyx_files_with_diagnostics().collect::<Vec<_>>(),
        None => project
            .diagnostic_summaries(false, cx)
            .map(|(path, _, _)| path)
            .collect(),
    };
    paths.sort();
    paths.dedup();
    paths
}

/// The files reported now and those in the cache (whose entries may be gone).
fn all_paths(project: &Entity<Project>, cx: &mut App) -> Vec<ProjectPath> {
    let mut paths = known_paths(project, cx);
    paths.extend(reported_paths(project, cx));
    paths.sort();
    paths.dedup();
    paths
}

/// The files of `project` in the cache.
fn known_paths(project: &Entity<Project>, cx: &mut App) -> Vec<ProjectPath> {
    let id = project.entity_id();
    cx.default_global::<Feed>()
        .cache
        .read()
        .values()
        .filter(|cached| cached.project == id)
        .map(|cached| cached.path.clone())
        .collect()
}

/// A file's entries as read on the main thread: final, or still to be converted to UTF-8
/// columns against the file's text.
enum Read {
    Ready(Vec<Diagnostic>),
    Stored(Vec<(Range<Unclipped<PointUtf16>>, Diagnostic)>),
}

/// Re-reads `paths` into the cache and tells the watchers which files changed (with `restarted`,
/// that anything may have).
fn refresh(
    project: &Entity<Project>,
    paths: Vec<ProjectPath>,
    restarted: bool,
    cx: &mut App,
) -> Task<()> {
    let project_id = project.entity_id();
    let fs = project.read(cx).fs().clone();
    let names = server_names(project, cx);
    let reads = paths
        .into_iter()
        .filter_map(|path| {
            let abs_path = project.read(cx).absolute_path(&path, cx)?;
            let read = read_file(project, &path, &names, cx);
            Some((path, abs_path, read))
        })
        .collect::<Vec<_>>();
    let feed = cx.default_global::<Feed>();
    let generations = reads
        .iter()
        .map(|(_, abs_path, _)| {
            feed.next_generation += 1;
            feed.generations
                .insert(abs_path.clone(), feed.next_generation);
            feed.next_generation
        })
        .collect::<Vec<_>>();
    cx.spawn(async move |cx| {
        let mut read = Vec::with_capacity(reads.len());
        for (path, abs_path, entries) in reads {
            let diagnostics = match entries {
                Read::Ready(diagnostics) => diagnostics,
                Read::Stored(entries) if entries.is_empty() => Vec::new(),
                Read::Stored(entries) => {
                    // Unreadable: the columns stay as they are (right for ASCII lines).
                    let text = fs.load(&abs_path).await.unwrap_or_default();
                    convert(&text, entries)
                }
            };
            read.push((path, abs_path, diagnostics));
        }
        cx.update(|cx| {
            let feed = cx.default_global::<Feed>();
            let mut changed = Vec::new();
            {
                let mut cache = feed.cache.write();
                for ((path, abs_path, diagnostics), generation) in read.into_iter().zip(generations)
                {
                    if feed.generations.get(&abs_path) != Some(&generation) {
                        continue;
                    }
                    feed.generations.remove(&abs_path);
                    let before = cache.get(&abs_path).map(|cached| &cached.diagnostics);
                    if before.map_or(diagnostics.is_empty(), |before| *before == diagnostics) {
                        continue;
                    }
                    if diagnostics.is_empty() {
                        cache.remove(&abs_path);
                    } else {
                        cache.insert(
                            abs_path.clone(),
                            Cached {
                                project: project_id,
                                path,
                                diagnostics,
                            },
                        );
                    }
                    changed.push(abs_path);
                }
            }
            if restarted {
                notify(None, cx);
            } else if !changed.is_empty() {
                notify(Some(changed), cx);
            }
        });
    })
}

fn server_names(project: &Entity<Project>, cx: &App) -> HashMap<LanguageServerId, String> {
    project
        .read(cx)
        .lsp_store()
        .read(cx)
        .language_server_statuses()
        .map(|(id, status)| (id, status.name.to_string()))
        .collect()
}

/// One file's primary entries: from its open buffer, else from the language-server store.
fn read_file(
    project: &Entity<Project>,
    path: &ProjectPath,
    names: &HashMap<LanguageServerId, String>,
    cx: &App,
) -> Read {
    if let Some(buffer) = project.read(cx).get_open_buffer(path, cx) {
        return Read::Ready(buffer_diagnostics(&buffer.read(cx).snapshot(), names));
    }
    let lsp_store = project.read(cx).lsp_store();
    let Some(local) = lsp_store.read(cx).as_local() else {
        return Read::Stored(Vec::new());
    };
    let entries = local
        .fyx_file_diagnostics(path.worktree_id, &path.path)
        .iter()
        .flat_map(|(server, entries)| {
            entries
                .iter()
                .filter(|entry| entry.diagnostic.is_primary)
                .map(move |entry| (*server, entry))
        })
        .map(|(server, entry): (LanguageServerId, &DiagnosticEntry<_>)| {
            let diagnostic = diagnostic(&entry.diagnostic, server, names, empty_range());
            (entry.range.clone(), diagnostic)
        })
        .collect();
    Read::Stored(entries)
}

/// An open buffer's primary entries, at their current positions.
pub(crate) fn buffer_diagnostics(
    snapshot: &BufferSnapshot,
    names: &HashMap<LanguageServerId, String>,
) -> Vec<Diagnostic> {
    snapshot
        .diagnostic_groups(None)
        .into_iter()
        .filter_map(|(server, group)| {
            let entry = group.entries.get(group.primary_ix)?;
            let start = entry.range.start.to_point(snapshot);
            let end = entry.range.end.to_point(snapshot);
            let range = TextRange {
                start: position(start),
                end: position(end),
            };
            Some(diagnostic(entry.diagnostic, server, names, range))
        })
        .collect()
}

fn position(point: Point) -> Position {
    Position {
        line: point.row,
        column: point.column,
    }
}

fn empty_range() -> TextRange {
    let at = Position { line: 0, column: 0 };
    TextRange { start: at, end: at }
}

/// Zed's diagnostic as the extension sees it (the range set by the caller).
fn diagnostic(
    diagnostic: &language::Diagnostic,
    server: LanguageServerId,
    names: &HashMap<LanguageServerId, String>,
    range: TextRange,
) -> Diagnostic {
    // lsp's severity and code are not reachable from here by name: their wire form is.
    let severity = match serde_json::to_value(diagnostic.severity)
        .ok()
        .and_then(|value| value.as_i64())
    {
        Some(2) => DiagnosticSeverity::Warning,
        Some(3) => DiagnosticSeverity::Information,
        Some(4) => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Error,
    };
    let code = diagnostic
        .code
        .as_ref()
        .and_then(|code| serde_json::to_value(code).ok())
        .and_then(|code| match code {
            serde_json::Value::String(code) => Some(code),
            serde_json::Value::Number(code) => Some(code.to_string()),
            _ => None,
        });
    Diagnostic {
        range,
        severity,
        message: diagnostic.message.as_str().to_string(),
        source: diagnostic
            .source
            .clone()
            .or_else(|| names.get(&server).cloned()),
        code,
    }
}

/// Stored entries with their UTF-16 columns converted to UTF-8 bytes of `text`'s lines (a
/// position past a line's end is clipped to it, one past the last line to the text's end).
pub(crate) fn convert(
    text: &str,
    entries: Vec<(Range<Unclipped<PointUtf16>>, Diagnostic)>,
) -> Vec<Diagnostic> {
    let lines = text.split('\n').collect::<Vec<_>>();
    let to_utf8 = |point: Unclipped<PointUtf16>| -> Position {
        let point = point.0;
        let Some(line) = lines.get(point.row as usize) else {
            let last = lines.len().saturating_sub(1);
            return Position {
                line: last as u32,
                column: lines.get(last).map_or(0, |line| line.len()) as u32,
            };
        };
        let mut units = 0;
        let mut column = line.len();
        for (index, character) in line.char_indices() {
            if units >= point.column as usize {
                column = index;
                break;
            }
            units += character.len_utf16();
        }
        Position {
            line: point.row,
            column: column as u32,
        }
    };
    entries
        .into_iter()
        .map(|(range, mut diagnostic)| {
            diagnostic.range = TextRange {
                start: to_utf8(range.start),
                end: to_utf8(range.end),
            };
            diagnostic
        })
        .collect()
}

/// Tells every watcher which files changed (`None`: anything may have).
fn notify(changed: Option<Vec<PathBuf>>, cx: &mut App) {
    let open = OpenWorktrees::global(cx);
    let feed = cx.default_global::<Feed>();
    feed.watchers.retain(|watcher| watcher.strong_count() > 0);
    let watchers = feed
        .watchers
        .iter()
        .filter_map(Weak::upgrade)
        .collect::<Vec<_>>();
    for watcher in watchers {
        let Some(sink) = watcher.sink() else {
            continue;
        };
        let files = match &changed {
            None => Vec::new(),
            Some(changed) => {
                let files = changed
                    .iter()
                    .filter_map(|abs_path| open.file_ref(&watcher.key, abs_path))
                    .collect::<Vec<_>>();
                if files.is_empty() {
                    continue;
                }
                files
            }
        };
        cx.background_spawn(async move { sink(files) }).detach();
    }
}

#[cfg(test)]
#[path = "diagnostics_host_tests.rs"]
mod tests;
