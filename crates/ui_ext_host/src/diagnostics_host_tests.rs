use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use gpui::{EntityId, TestAppContext};
use project::Project;
use serde_json::json;
use util::path;

use super::*;
use crate::worktrees::{OpenWorktree, OpenWorktrees, opaque_id};

const WORKSPACE: u64 = 1;
const SERVER: LanguageServerId = LanguageServerId(0);

async fn setup(cx: &mut TestAppContext) -> Entity<Project> {
    crate::tests::init_test(cx);
    let fs = fs::FakeFs::new(cx.executor());
    // "é" is one UTF-16 unit and two UTF-8 bytes, "😀" two units and four bytes.
    fs.insert_tree(
        path!("/work"),
        json!({ "a.rs": "let é = 😀x;\n", "b.rs": "fn b() {}\n" }),
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
        watch_project(&project, cx);
    });
    project
}

fn worktree_id(project: &Entity<Project>, cx: &mut TestAppContext) -> u64 {
    cx.update(|cx| {
        project
            .read(cx)
            .worktrees(cx)
            .next()
            .unwrap()
            .read(cx)
            .id()
            .to_proto()
    })
}

/// A primary error at UTF-16 columns `start..end` of line 0, with a numeric code and no source.
fn entry(
    start: u32,
    end: u32,
    severity: i64,
    message: &str,
) -> DiagnosticEntry<Unclipped<PointUtf16>> {
    DiagnosticEntry::new(
        Unclipped(PointUtf16::new(0, start))..Unclipped(PointUtf16::new(0, end)),
        language::Diagnostic {
            severity: serde_json::from_value(json!(severity)).unwrap(),
            code: Some(serde_json::from_value(json!(42)).unwrap()),
            message: message.to_string().into(),
            is_primary: true,
            ..Default::default()
        },
    )
}

fn report(
    project: &Entity<Project>,
    file: &str,
    entries: Vec<DiagnosticEntry<Unclipped<PointUtf16>>>,
    cx: &mut TestAppContext,
) {
    let abs_path = PathBuf::from(path!("/work")).join(file);
    project.update(cx, |project, cx| {
        project.lsp_store().update(cx, |lsp_store, cx| {
            lsp_store
                .update_diagnostic_entries(SERVER, abs_path, None, None, entries, cx)
                .unwrap()
        })
    });
    cx.run_until_parked();
}

fn source(cx: &mut TestAppContext) -> (Cache, OpenWorktrees) {
    cx.update(|cx| {
        let _fill = activate(cx);
        (
            cx.default_global::<Feed>().cache.clone(),
            OpenWorktrees::global(cx),
        )
    })
}

fn range(start: u32, end: u32) -> TextRange {
    TextRange {
        start: Position {
            line: 0,
            column: start,
        },
        end: Position {
            line: 0,
            column: end,
        },
    }
}

/// A closed file's entries: UTF-16 columns become UTF-8 bytes, severity and code map, and the
/// file is named the extension's way; one file or all.
#[gpui::test]
async fn test_closed_file_diagnostics(cx: &mut TestAppContext) {
    let project = setup(cx).await;
    // Reported before activation, a hint only (no summary): the first fill reads it.
    report(&project, "a.rs", vec![entry(4, 5, 4, "hint")], cx);
    let (cache, open) = source(cx);
    cx.run_until_parked();
    let filled = read_cache(&cache, &open, "ext", None);
    assert_eq!(filled[0].diagnostics[0].severity, DiagnosticSeverity::Hint);
    // "let é = 😀x;": é at UTF-16 4..5 is bytes 4..6; 😀 at 8..10 is bytes 9..13.
    report(
        &project,
        "a.rs",
        vec![entry(4, 5, 1, "unused"), entry(8, 10, 2, "emoji")],
        cx,
    );
    let id = Some(opaque_id("ext", (WORKSPACE, worktree_id(&project, cx))));
    let file = FileRef {
        worktree: id,
        path: "a.rs".into(),
    };
    let all = read_cache(&cache, &open, "ext", None);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].path, file);
    let diagnostics = &all[0].diagnostics;
    let mut seen = diagnostics
        .iter()
        .map(|d| (d.range, d.severity, d.message.as_str(), d.code.as_deref()))
        .collect::<Vec<_>>();
    seen.sort_by_key(|(range, ..)| range.start.column);
    assert_eq!(
        seen,
        [
            (range(4, 6), DiagnosticSeverity::Error, "unused", Some("42")),
            (
                range(9, 13),
                DiagnosticSeverity::Warning,
                "emoji",
                Some("42")
            ),
        ]
    );
    assert_eq!(read_cache(&cache, &open, "ext", Some(&file)), all);
    let other = FileRef {
        worktree: id,
        path: "b.rs".into(),
    };
    assert!(read_cache(&cache, &open, "ext", Some(&other)).is_empty());

    report(&project, "a.rs", Vec::new(), cx);
    assert!(read_cache(&cache, &open, "ext", None).is_empty(), "cleared");
}

/// An open buffer's entries follow its edits; a watcher hears the changed file, a stopped one
/// nothing.
#[gpui::test]
async fn test_open_buffer_and_watch(cx: &mut TestAppContext) {
    let project = setup(cx).await;
    let host = cx.update(|cx| for_extension("ext", cx));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink: DiagnosticsSink = {
        let heard = heard.clone();
        Arc::new(move |files| heard.lock().unwrap().push(files))
    };
    host.watch("ext", Some(sink));
    cx.run_until_parked();
    let (cache, open) = source(cx);
    let buffer = project
        .update(cx, |project, cx| {
            project.open_local_buffer(path!("/work/b.rs"), cx)
        })
        .await
        .unwrap();
    report(&project, "b.rs", vec![entry(3, 4, 2, "name")], cx);
    let file = FileRef {
        worktree: Some(opaque_id("ext", (WORKSPACE, worktree_id(&project, cx)))),
        path: "b.rs".into(),
    };
    assert_eq!(heard.lock().unwrap().pop(), Some(vec![file.clone()]));
    let all = read_cache(&cache, &open, "ext", None);
    assert_eq!(all[0].diagnostics[0].range, range(3, 4));
    assert_eq!(all[0].diagnostics[0].severity, DiagnosticSeverity::Warning);

    // Typing before the entry moves it; the next report reads the buffer as it is.
    buffer.update(cx, |buffer, cx| buffer.edit([(0..0, "pub ")], None, cx));
    let names = HashMap::default();
    let moved = buffer.read_with(cx, |buffer, _| {
        buffer_diagnostics(&buffer.snapshot(), &names)
    });
    assert_eq!(moved[0].range, range(7, 8));

    heard.lock().unwrap().clear();
    host.watch("ext", None);
    report(&project, "b.rs", Vec::new(), cx);
    assert!(heard.lock().unwrap().is_empty(), "stopped");
    assert!(read_cache(&cache, &open, "ext", None).is_empty());
}

#[test]
fn test_convert_clips() {
    let at = |row, column| Unclipped(PointUtf16::new(row, column));
    let blank = Diagnostic {
        range: empty_range(),
        severity: DiagnosticSeverity::Hint,
        message: String::new(),
        source: None,
        code: None,
    };
    let converted = convert(
        "ab\r\nc",
        vec![
            (at(0, 1)..at(0, 99), blank.clone()),
            (at(1, 0)..at(5, 0), blank),
        ],
    );
    let position = |line, column| Position { line, column };
    assert_eq!(converted[0].range.start, position(0, 1));
    assert_eq!(converted[0].range.end, position(0, 3), "to the line's end");
    assert_eq!(converted[1].range.end, position(1, 1), "to the text's end");
}
