use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use editor::Editor;
use gpui::{EntityId, TestAppContext, VisualTestContext};
use project::Project;
use serde_json::json;
use util::path;

use super::*;
use crate::worktrees::{OpenWorktree, OpenWorktrees};

const WORKSPACE: u64 = 1;

fn empty_range() -> TextRange {
    let at = Position { line: 0, column: 0 };
    TextRange { start: at, end: at }
}

fn worktree(root: &str, trusted: bool) -> OpenWorktree {
    OpenWorktree {
        key: (WORKSPACE, 1),
        root: PathBuf::from(root),
        trusted,
    }
}

/// The extension sees its own `file-ref`s, worktree files only, and the active file's trust.
#[test]
fn test_context_for_maps_to_file_refs() {
    let open = OpenWorktrees::default();
    open.set(EntityId::from(WORKSPACE), vec![worktree("/work", false)]);
    let selection = Some((empty_range(), Some("two".to_string())));
    let snapshot = Snapshot {
        active: Some(PathBuf::from("/work/src/a.rs")),
        open: vec![
            PathBuf::from("/work/src/a.rs"),
            PathBuf::from("/elsewhere/b.rs"),
            PathBuf::from("/work/c.rs"),
        ],
        selection: selection.clone(),
    };
    let context = context_for("ext", &snapshot, &open);
    let id = Some(opaque_id("ext", (WORKSPACE, 1)));
    let file = |path: &str| FileRef {
        worktree: id,
        path: path.into(),
    };
    assert_eq!(context.active, Some(file("src/a.rs")));
    assert_eq!(context.open, vec![file("src/a.rs"), file("c.rs")]);
    assert_eq!(context.trusted, Some(false));
    assert_eq!(context.selection, selection);
    assert_ne!(
        context_for("other", &snapshot, &open).active,
        context.active,
        "ids are per extension"
    );

    let outside = Snapshot {
        active: Some(PathBuf::from("/elsewhere/b.rs")),
        selection: Some((empty_range(), Some("secret".into()))),
        ..Snapshot::default()
    };
    let context = context_for("ext", &outside, &open);
    assert_eq!(
        (context.active, context.selection, context.trusted),
        (None, None, None),
        "no file outside the worktrees, and no selection from one"
    );
}

/// `watch` sends the current context at once; a stopped watcher hears nothing more, and another
/// load's watcher is not touched by it.
#[gpui::test]
async fn test_watch_sends_at_once_and_stops(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = |name: &'static str| -> ContextSink {
        let received = received.clone();
        Arc::new(move |context| received.lock().unwrap().push((name, context)))
    };
    let first = cx.update(|cx| for_extension("ext", cx));
    let second = cx.update(|cx| for_extension("ext", cx));
    first.watch("ext", Some(sink("first")));
    second.watch("ext", Some(sink("second")));
    cx.run_until_parked();
    let mut names = received
        .lock()
        .unwrap()
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, ["first", "second"]);

    received.lock().unwrap().clear();
    first.watch("ext", None);
    cx.update(changed);
    cx.executor().advance_clock(DEBOUNCE);
    cx.run_until_parked();
    let names = received
        .lock()
        .unwrap()
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["second"],
        "debounced, once, to the one still watching"
    );
}

/// The workspace's context: the active file, the open files most recent first, the selection
/// with its text, and no text for a private file.
#[gpui::test]
async fn test_snapshot_of_a_workspace(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let fs = fs::FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/work"),
        json!({ "a.txt": "one\ntwo\nthree\n", "b.txt": "b\n", ".env": "SECRET=1\n" }),
    )
    .await;
    let project = Project::test(fs, [Path::new(path!("/work"))], cx).await;
    let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());
    cx.update(|_, cx| {
        crate::workspace_edit::register_workspace(
            EntityId::from(WORKSPACE),
            None,
            project.downgrade(),
            cx,
        )
    });
    let worktree_id =
        cx.update(|_, cx| project.read(cx).worktrees(cx).next().unwrap().read(cx).id());
    let open = |name: &'static str, cx: &mut VisualTestContext| {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_path(
                (worktree_id, util::rel_path::rel_path(name)),
                None,
                true,
                window,
                cx,
            )
        })
    };
    open("b.txt", cx).await.unwrap();
    let item = open("a.txt", cx).await.unwrap();
    let editor = item.downcast::<Editor>().unwrap();
    editor.update_in(cx, |editor, window, cx| {
        editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
            selections.select_ranges([Point::new(1, 0)..Point::new(1, 3)])
        })
    });
    let snapshot = cx.update(|_, cx| snapshot_of(&workspace, cx));
    assert_eq!(snapshot.active, Some(PathBuf::from(path!("/work/a.txt"))));
    assert_eq!(
        snapshot.open,
        vec![
            PathBuf::from(path!("/work/a.txt")),
            PathBuf::from(path!("/work/b.txt"))
        ]
    );
    let range = TextRange {
        start: Position { line: 1, column: 0 },
        end: Position { line: 1, column: 3 },
    };
    assert_eq!(snapshot.selection, Some((range, Some("two".into()))));

    open(".env", cx).await.unwrap();
    let snapshot = cx.update(|_, cx| snapshot_of(&workspace, cx));
    assert_eq!(snapshot.active, Some(PathBuf::from(path!("/work/.env"))));
    assert_eq!(
        snapshot.selection.map(|(_, text)| text),
        Some(None),
        "private"
    );

    // workspace:open brings a.txt back with the range selected.
    let window = cx.windows()[0];
    cx.update(|_, cx| {
        crate::workspace_edit::register_workspace(
            EntityId::from(WORKSPACE),
            Some(workspace.downgrade()),
            project.downgrade(),
            cx,
        );
        crate::workspace_edit::set_workspace_window(EntityId::from(WORKSPACE), window, cx);
        OpenWorktrees::global(cx).set(
            EntityId::from(WORKSPACE),
            vec![OpenWorktree {
                key: (WORKSPACE, worktree_id.to_proto()),
                root: PathBuf::from(path!("/work")),
                trusted: true,
            }],
        );
    });
    let range = TextRange {
        start: Position { line: 2, column: 0 },
        end: Position { line: 2, column: 5 },
    };
    let file = FileRef {
        worktree: None,
        path: "a.txt".into(),
    };
    let opening = cx
        .cx
        .update(|cx| open_file("ext", &file, Some(range), true, cx))
        .unwrap();
    opening.await.unwrap();
    let snapshot = cx.update(|_, cx| snapshot_of(&workspace, cx));
    assert_eq!(snapshot.active, Some(PathBuf::from(path!("/work/a.txt"))));
    assert_eq!(snapshot.selection, Some((range, Some("three".into()))));
    let missing = FileRef {
        worktree: Some(1),
        path: "a.txt".into(),
    };
    assert!(
        cx.update(|_, cx| open_file("ext", &missing, None, false, cx))
            .is_err()
    );
}
