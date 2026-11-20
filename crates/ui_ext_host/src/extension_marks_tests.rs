use std::path::{Path, PathBuf};

use fs::FakeFs;
use gpui::{EntityId, TestAppContext};
use project::{DiagnosticSummary, Project};
use serde_json::json;
use ui_ext::edit::TextRange;
use util::{path, rel_path::rel_path};

use super::*;
use crate::worktrees::{OpenWorktree, OpenWorktrees};

const WORKSPACE: u64 = 1;
const KEY: &str = "acme.lint";

async fn setup(cx: &mut TestAppContext) -> Entity<Project> {
    crate::tests::init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/work"),
        json!({ "a.txt": "héllo world\nsecond line\n", "b.txt": "one\n" }),
    )
    .await;
    let project = Project::test(fs, [Path::new(path!("/work"))], cx).await;
    cx.update(|cx| {
        let worktree_id = project.read(cx).worktrees(cx).next().unwrap().read(cx).id();
        OpenWorktrees::global(cx).set(
            EntityId::from(WORKSPACE),
            vec![OpenWorktree {
                key: (WORKSPACE, worktree_id.to_proto()),
                root: PathBuf::from(path!("/work")),
                trusted: true,
            }],
        );
        crate::workspace_edit::register_workspace(
            EntityId::from(WORKSPACE),
            None,
            project.downgrade(),
            cx,
        );
    });
    cx.run_until_parked();
    project
}

fn file(path: &str) -> FileRef {
    FileRef {
        worktree: None,
        path: path.into(),
    }
}

fn at(line: u32, column: u32) -> Position {
    Position { line, column }
}

fn diagnostic(start: Position, end: Position, severity: DiagnosticSeverity) -> Diagnostic {
    Diagnostic {
        range: TextRange { start, end },
        severity,
        message: "TODO left".into(),
        source: Some("rust-analyzer".into()),
        code: Some("T1".into()),
    }
}

async fn publish_now(
    path: &str,
    diagnostics: Vec<Diagnostic>,
    state: &Arc<Mutex<Published>>,
    cx: &mut TestAppContext,
) -> Result<(), String> {
    let state = state.clone();
    let path = file(path);
    let result = cx
        .spawn(async move |mut cx: AsyncApp| {
            publish(KEY, KEY, path, diagnostics, &state, &mut cx).await
        })
        .await;
    cx.run_until_parked();
    result
}

fn summary(project: &Entity<Project>, cx: &mut TestAppContext) -> DiagnosticSummary {
    project.read_with(cx, |project, cx| project.diagnostic_summary(false, cx))
}

fn entries(
    project: &Entity<Project>,
    path: &str,
    cx: &mut TestAppContext,
) -> Vec<DiagnosticEntry<Unclipped<PointUtf16>>> {
    project.read_with(cx, |project, cx| {
        let worktree_id = project.worktrees(cx).next().unwrap().read(cx).id();
        let lsp_store = project.lsp_store().read(cx);
        lsp_store
            .as_local()
            .unwrap()
            .fyx_file_diagnostics(worktree_id, rel_path(path))
            .iter()
            .flat_map(|(_, entries)| entries.clone())
            .collect()
    })
}

/// §E.4: published entries count like a language server's, `source` is the extension, columns
/// in UTF-8 bytes land on the same characters, an empty list clears the file and the load's end
/// clears every file.
#[gpui::test]
async fn test_published_diagnostics(cx: &mut TestAppContext) {
    let project = setup(cx).await;
    let state = Arc::<Mutex<Published>>::default();
    // "héllo": `é` is two bytes, so "world" starts at byte 7, UTF-16 column 6.
    publish_now(
        "a.txt",
        vec![
            diagnostic(at(0, 7), at(0, 12), DiagnosticSeverity::Error),
            diagnostic(at(1, 0), at(1, 99), DiagnosticSeverity::Warning),
        ],
        &state,
        cx,
    )
    .await
    .unwrap();
    publish_now(
        "b.txt",
        vec![diagnostic(at(9, 0), at(9, 1), DiagnosticSeverity::Error)],
        &state,
        cx,
    )
    .await
    .unwrap();
    let counts = summary(&project, cx);
    assert_eq!((counts.error_count, counts.warning_count), (2, 1));

    let a = entries(&project, "a.txt", cx);
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].range.start.0, PointUtf16::new(0, 6));
    assert_eq!(a[0].range.end.0, PointUtf16::new(0, 11));
    assert_eq!(
        a[1].range.end.0,
        PointUtf16::new(1, 11),
        "clamped to the line"
    );
    assert_eq!(
        a[0].diagnostic.source.as_deref(),
        Some(KEY),
        "never rust-analyzer"
    );
    assert!(a[0].diagnostic.is_primary);
    assert_ne!(a[0].diagnostic.group_id, a[1].diagnostic.group_id);
    let b = entries(&project, "b.txt", cx);
    assert_eq!(
        b[0].range.start.0,
        PointUtf16::new(1, 0),
        "a line past the end is the end"
    );

    publish_now("a.txt", Vec::new(), &state, cx).await.unwrap();
    assert!(entries(&project, "a.txt", cx).is_empty());
    assert_eq!(summary(&project, cx).error_count, 1);

    cx.update(|cx| clear_all(&state, cx));
    cx.run_until_parked();
    assert_eq!(summary(&project, cx), DiagnosticSummary::default());
}

/// A file in no worktree publishes nothing.
#[gpui::test]
async fn test_publish_outside_the_worktrees(cx: &mut TestAppContext) {
    let project = setup(cx).await;
    let state = Arc::<Mutex<Published>>::default();
    let mut outside = file("x.txt");
    outside.worktree = Some(12345);
    let state_clone = state.clone();
    let result = cx
        .spawn(async move |mut cx: AsyncApp| {
            publish(
                KEY,
                KEY,
                outside,
                vec![diagnostic(at(0, 0), at(0, 1), DiagnosticSeverity::Error)],
                &state_clone,
                &mut cx,
            )
            .await
        })
        .await;
    assert!(result.is_err());
    assert_eq!(summary(&project, cx), DiagnosticSummary::default());
}

/// §F.5: decorations reach the panel's global with the extension's name; clearing some, then all.
#[gpui::test]
async fn test_file_decorations(cx: &mut TestAppContext) {
    let project = setup(cx).await;
    let decorated = Mutex::default();
    let mark = |path: &str, badge: &str| FileDecoration {
        path: file(path),
        badge: Some(badge.into()),
        color: Some(ColorToken::Warning),
        tooltip: Some("bookmarked".into()),
    };
    cx.update(|cx| {
        decorate(
            KEY,
            KEY,
            vec![mark("a.txt", "★"), mark("b.txt", "B")],
            &decorated,
            cx,
        )
    })
    .unwrap();
    let project_path = |path: &str, cx: &mut TestAppContext| {
        project.read_with(cx, |project, cx| {
            project
                .find_project_path(Path::new(&format!("{}/{path}", path!("/work"))), cx)
                .unwrap()
        })
    };
    let a = project_path("a.txt", cx);
    let b = project_path("b.txt", cx);
    let shown = cx.update(|cx| FileDecorations::global(cx).read(cx).for_path(&a));
    assert_eq!(shown.badges, vec![SharedString::from("★")]);
    assert_eq!(shown.color, Some(FileDecorationColor::Warning));
    assert_eq!(
        shown.tooltips,
        vec![(SharedString::from(KEY), SharedString::from("bookmarked"))]
    );

    cx.update(|cx| clear(KEY, KEY, Some(vec![file("a.txt")]), &decorated, cx));
    let (a_shown, b_shown) = cx.update(|cx| {
        let decorations = FileDecorations::global(cx).read(cx);
        (decorations.for_path(&a), decorations.for_path(&b))
    });
    assert!(a_shown.is_empty());
    assert_eq!(b_shown.badges, vec![SharedString::from("B")]);

    cx.update(|cx| clear(KEY, KEY, None, &decorated, cx));
    assert!(cx.update(|cx| FileDecorations::global(cx).read(cx).is_empty()));
    assert!(decorated.lock().unwrap().is_empty());
}
