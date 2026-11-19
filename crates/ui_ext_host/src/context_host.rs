//! The editor side of the 0.4 `context` import (W2c's [`ContextHost`], design wit-0.4 §B.11,
//! plan 5.13): what the user has open and selected, for `context:editor`, and opening a worktree
//! file for `workspace:open`.
//!
//! The context is the active workspace's (the window the user was in last): its active file, the
//! files its panes hold (most recently active first) and the newest selection of the active
//! editor. Every workspace reports changes here; they reach the watching extensions debounced by
//! 300 ms, and a new watcher gets the current context at once. Only worktree files appear, as each
//! extension's own `file-ref`s; a private file's selection has no text. The runtime filters again.
//!
//! Each load has its own host object (keyed like its worktree ids): a reloaded instance's watch
//! and the old instance's stop never touch each other's sink. The extension thread waits for the
//! main thread on `open` and `is_private` (see [`MainThread`]); sinks are called from background
//! tasks, because the runtime's filter calls `is_private` back.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Duration,
};

use collections::HashMap;
use editor::{Editor, EditorEvent, SelectionEffects, scroll::Autoscroll};
use gpui::{App, AppContext as _, Context, Entity, EntityId, Global, Subscription, Task, Window};
use language::Point;
use ui_ext::{
    context::{ContextHost, ContextSink, EditorContext},
    edit::{Position, TextRange},
    process::FileRef,
};
use workspace::Workspace;

use crate::{
    main_thread::MainThread,
    worktrees::{OpenWorktree, OpenWorktrees, opaque_id},
};

/// How long the context settles before it is sent (§B.11).
pub const DEBOUNCE: Duration = Duration::from_millis(300);
/// How long an extension thread waits for the editor.
const OPEN_TIMEOUT: Duration = Duration::from_secs(5);
const PRIVATE_TIMEOUT: Duration = Duration::from_secs(2);

/// One load's feed.
struct Watcher {
    /// What its worktree ids are keyed by (the extension id, or a dev extension's path).
    key: Arc<str>,
    sink: Mutex<Option<ContextSink>>,
}

impl Watcher {
    fn sink(&self) -> Option<ContextSink> {
        self.sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The watchers and the workspaces' subscriptions. Main thread only.
#[derive(Default)]
struct Feed {
    watchers: Vec<Weak<Watcher>>,
    /// The debounce timer of the next send.
    pending: Option<Task<()>>,
    /// The active editor's selection changes, per workspace.
    editors: HashMap<EntityId, Subscription>,
}

impl Global for Feed {}

/// The context as the editor has it, in absolute paths.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    pub active: Option<PathBuf>,
    pub open: Vec<PathBuf>,
    /// The newest selection of the active file; no text when the file is private.
    pub selection: Option<(TextRange, Option<String>)>,
}

/// The [`ContextHost`] of one load. `key` is what [`OpenWorktrees::for_extension`] was given.
pub(crate) fn for_extension(key: &str, cx: &mut App) -> Arc<dyn ContextHost> {
    let watcher = Arc::new(Watcher {
        key: key.into(),
        sink: Mutex::new(None),
    });
    let feed = cx.default_global::<Feed>();
    feed.watchers.retain(|watcher| watcher.strong_count() > 0);
    feed.watchers.push(Arc::downgrade(&watcher));
    Arc::new(ExtensionContext {
        main: MainThread::global(cx),
        watcher,
    })
}

struct ExtensionContext {
    main: MainThread,
    watcher: Arc<Watcher>,
}

impl ContextHost for ExtensionContext {
    fn watch(&self, _extension_id: &str, sink: Option<ContextSink>) {
        let watching = sink.is_some();
        *self
            .watcher
            .sink
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = sink;
        if watching {
            let watcher = self.watcher.clone();
            self.main.post(move |cx| {
                let snapshot = snapshot(cx);
                send(&watcher, &snapshot, cx);
            });
        }
    }

    fn open(
        &self,
        _extension_id: &str,
        path: FileRef,
        range: Option<TextRange>,
        focus: bool,
    ) -> Result<(), String> {
        let key = self.watcher.key.clone();
        self.main
            .call(OPEN_TIMEOUT, move |cx, reply| {
                match open_file(&key, &path, range, focus, cx) {
                    Ok(task) => {
                        cx.spawn(async move |_| {
                            reply.send(task.await);
                        })
                        .detach();
                    }
                    Err(why) => {
                        reply.send(Err(why));
                    }
                }
            })
            .unwrap_or_else(|| Err("the editor did not answer".into()))
    }

    fn is_private(&self, absolute: &Path) -> bool {
        let absolute = absolute.to_path_buf();
        // Not answered in time: private, so no text leaves.
        self.main
            .call(PRIVATE_TIMEOUT, move |cx, reply| {
                reply.send(crate::workspace_edit::is_private_path(&absolute, cx));
            })
            .unwrap_or(true)
    }
}

/// The worktree `file` names for `key`, and its absolute path.
pub(crate) fn locate(
    key: &str,
    file: &FileRef,
    cx: &mut App,
) -> Result<(OpenWorktree, PathBuf), String> {
    let open = OpenWorktrees::global(cx).all();
    let mut candidates = open.into_iter().filter(|worktree| {
        file.worktree
            .is_none_or(|id| opaque_id(key, worktree.key) == id)
    });
    let worktree = match (candidates.next(), candidates.next(), file.worktree) {
        (Some(worktree), None, _) | (Some(worktree), Some(_), Some(_)) => worktree,
        (Some(_), Some(_), None) => return Err("several worktrees are open; name one".into()),
        (None, ..) => return Err("no such worktree".into()),
    };
    let abs_path = file
        .path
        .split('/')
        .fold(worktree.root.clone(), |path, component| {
            path.join(component)
        });
    Ok((worktree, abs_path))
}

fn point(position: Position) -> Point {
    Point::new(position.line, position.column)
}

/// Starts opening `file` in the window of the workspace that has its worktree: in a background
/// tab unless `focus`, with `range` selected.
fn open_file(
    key: &str,
    file: &FileRef,
    range: Option<TextRange>,
    focus: bool,
    cx: &mut App,
) -> Result<Task<Result<(), String>>, String> {
    let (worktree, abs_path) = locate(key, file, cx)?;
    let (workspace, window) =
        crate::workspace_edit::workspace_and_window(EntityId::from(worktree.key.0), cx)
            .ok_or("the window of that worktree is closed")?;
    let project_path = workspace
        .read(cx)
        .project()
        .read(cx)
        .find_project_path(&abs_path, cx)
        .filter(|path| path.worktree_id.to_proto() == worktree.key.1)
        .ok_or("the file is not in that worktree")?;
    let opening = window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.open_path_preview(project_path, None, focus, false, focus, window, cx)
            })
        })
        .map_err(|error| format!("{error:#}"))?;
    Ok(cx.spawn(async move |cx| {
        let item = opening.await.map_err(|error| format!("{error:#}"))?;
        let (Some(range), Some(editor)) = (range, item.downcast::<Editor>()) else {
            return Ok(());
        };
        window
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    let Some(buffer) = editor.buffer().read(cx).as_singleton() else {
                        return;
                    };
                    let snapshot = buffer.read(cx).snapshot();
                    let start = snapshot.clip_point(point(range.start), language::Bias::Left);
                    let end = snapshot.clip_point(point(range.end), language::Bias::Left);
                    editor.change_selections(
                        SelectionEffects::scroll(Autoscroll::center()),
                        window,
                        cx,
                        |selections| selections.select_ranges([start..end]),
                    );
                })
            })
            .map_err(|error| format!("{error:#}"))
    }))
}

/// The active workspace's context now.
pub(crate) fn snapshot(cx: &mut App) -> Snapshot {
    let workspace = crate::UiExtensionRegistry::global(cx)
        .and_then(|registry| registry.read(cx).active_workspace())
        .and_then(|id| crate::workspace_edit::workspace_and_window(id, cx))
        .map(|(workspace, _)| workspace);
    match workspace {
        Some(workspace) => snapshot_of(&workspace, cx),
        None => Snapshot::default(),
    }
}

/// `workspace`'s context: its active file, the files its panes hold, most recently active first,
/// and the active editor's newest selection.
pub(crate) fn snapshot_of(workspace: &Entity<Workspace>, cx: &mut App) -> Snapshot {
    let project = workspace.read(cx).project().clone();
    let abs_of = |item: &dyn workspace::ItemHandle, cx: &App| {
        let project_path = item.project_path(cx)?;
        project.read(cx).absolute_path(&project_path, cx)
    };
    let active_item = workspace.read(cx).active_item(cx);
    let active = active_item.as_deref().and_then(|item| abs_of(item, cx));
    let mut recent: Vec<(usize, PathBuf)> = Vec::new();
    for pane in workspace.read(cx).panes() {
        let pane = pane.read(cx);
        let history = pane.activation_history();
        for item in pane.items() {
            let Some(abs_path) = abs_of(item.as_ref(), cx) else {
                continue;
            };
            let stamp = history
                .iter()
                .find(|entry| entry.entity_id == item.item_id())
                .map_or(0, |entry| entry.timestamp);
            recent.push((stamp, abs_path));
        }
    }
    recent.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    let mut open: Vec<PathBuf> = Vec::new();
    for abs_path in active
        .iter()
        .cloned()
        .chain(recent.into_iter().map(|(_, path)| path))
    {
        if !open.contains(&abs_path) {
            open.push(abs_path);
        }
    }
    let selection = active.as_ref().and_then(|abs_path| {
        let editor = active_item.as_ref()?.act_as::<Editor>(cx)?;
        let (range, text) = editor.update(cx, |editor, cx| {
            let buffer = editor.buffer().read(cx).as_singleton()?;
            let selection = editor
                .selections
                .newest::<Point>(&editor.display_snapshot(cx));
            let range = TextRange {
                start: Position {
                    line: selection.start.row,
                    column: selection.start.column,
                },
                end: Position {
                    line: selection.end.row,
                    column: selection.end.column,
                },
            };
            let text = buffer
                .read(cx)
                .text_for_range(selection.start..selection.end)
                .collect::<String>();
            Some((range, text))
        })?;
        let private = crate::workspace_edit::is_private_path(abs_path, cx);
        Some((range, (!private).then_some(text)))
    });
    Snapshot {
        active,
        open,
        selection,
    }
}

/// `snapshot` as the extension keyed by `key` sees it: its own `file-ref`s, worktree files only.
pub(crate) fn context_for(key: &str, snapshot: &Snapshot, open: &OpenWorktrees) -> EditorContext {
    let active = snapshot
        .active
        .as_deref()
        .and_then(|path| open.file_ref(key, path));
    let trusted = active.as_ref().and_then(|file| {
        open.all()
            .into_iter()
            .find(|worktree| Some(opaque_id(key, worktree.key)) == file.worktree)
            .map(|worktree| worktree.trusted)
    });
    EditorContext {
        selection: active.as_ref().and(snapshot.selection.clone()),
        open: snapshot
            .open
            .iter()
            .filter_map(|path| open.file_ref(key, path))
            .collect(),
        active,
        trusted,
    }
}

fn send(watcher: &Watcher, snapshot: &Snapshot, cx: &mut App) {
    let Some(sink) = watcher.sink() else {
        return;
    };
    let context = context_for(&watcher.key, snapshot, &OpenWorktrees::global(cx));
    cx.background_spawn(async move { sink(context) }).detach();
}

/// Something in a workspace changed: the watchers get the context once it settled.
pub(crate) fn changed(cx: &mut App) {
    let Some(feed) = cx.try_global::<Feed>() else {
        return;
    };
    let watching = feed
        .watchers
        .iter()
        .filter_map(Weak::upgrade)
        .any(|watcher| watcher.sink().is_some());
    if !watching {
        return;
    }
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(DEBOUNCE).await;
        cx.update(|cx| {
            let watchers = cx
                .try_global::<Feed>()
                .map(|feed| {
                    feed.watchers
                        .iter()
                        .filter_map(Weak::upgrade)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let snapshot = snapshot(cx);
            for watcher in watchers {
                send(&watcher, &snapshot, cx);
            }
        });
    });
    cx.default_global::<Feed>().pending = Some(task);
}

/// Reports `workspace`'s changes to the feed: its items, its active editor's selection, its
/// window's activation. From the registry's workspace hook.
pub(crate) fn watch_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace_id = cx.entity_id();
    crate::workspace_edit::set_workspace_window(workspace_id, window.window_handle(), cx);
    cx.subscribe_in(
        &cx.entity(),
        window,
        move |workspace, _, event: &workspace::Event, _, cx| match event {
            workspace::Event::ActiveItemChanged => {
                follow_active_editor(workspace_id, workspace, cx);
                changed(cx);
            }
            workspace::Event::ItemAdded { .. }
            | workspace::Event::ItemRemoved { .. }
            | workspace::Event::PaneAdded(_)
            | workspace::Event::PaneRemoved => changed(cx),
            _ => {}
        },
    )
    .detach();
    cx.observe_window_activation(window, |_, _, cx| changed(cx))
        .detach();
    follow_active_editor(workspace_id, workspace, cx);
    cx.on_release(move |_, cx| {
        if cx.has_global::<Feed>() {
            cx.global_mut::<Feed>().editors.remove(&workspace_id);
        }
    })
    .detach();
}

fn follow_active_editor(workspace_id: EntityId, workspace: &Workspace, cx: &mut App) {
    let editor = workspace
        .active_item(cx)
        .and_then(|item| item.act_as::<Editor>(cx));
    let subscription = editor.map(|editor| {
        cx.subscribe(&editor, |_, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::SelectionsChanged { .. }) {
                changed(cx);
            }
        })
    });
    let feed = cx.default_global::<Feed>();
    match subscription {
        Some(subscription) => {
            feed.editors.insert(workspace_id, subscription);
        }
        None => {
            feed.editors.remove(&workspace_id);
        }
    }
}

#[cfg(test)]
#[path = "context_host_tests.rs"]
mod tests;
