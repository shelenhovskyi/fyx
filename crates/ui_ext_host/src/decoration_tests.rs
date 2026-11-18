//! Decorations in real editors: the block follows its line through edits above it, and every
//! editor on the buffer places it at the same logical line.

use std::path::Path;

use editor::{Editor, display_map::CustomBlockId};
use fs::{FakeFs, Fs as _};
use gpui::{Entity, Pixels, TestAppContext, VisualTestContext};
use language::{Buffer, Point};
use project::Project;
use ui_ext::TreeLimits;
use ui_ext::v0_2::{FlexDirection, FlexProps, Node, Patch, Props, TextProps, Update, tree::Tree};
use util::path;

use crate::{
    DecorationId, DecorationPlacement, DecorationTarget, Decorations,
    decorations::{estimate_lines, parse_dev_decorations, path_matches},
    text_tree,
};

/// The text node of [`text_tree`].
const TEXT_SELECTOR: &str = "fyx-node-2";

fn init(cx: &mut TestAppContext) -> Entity<Decorations> {
    crate::tests::init_test(cx);
    cx.update(|cx| {
        crate::decorations::init(cx);
        Decorations::global(cx).expect("initialized")
    })
}

/// `<root>/scratch/notes.txt` with this text, opened from a real (fake-fs) worktree, so the
/// buffer has the `File` a project gives it.
async fn scratch_buffer(text: &str, cx: &mut TestAppContext) -> (Entity<Project>, Entity<Buffer>) {
    let fs = FakeFs::new(cx.executor());
    fs.create_dir(Path::new(path!("/root/scratch")))
        .await
        .expect("scratch dir");
    fs.insert_file(path!("/root/scratch/notes.txt"), text.as_bytes().to_vec())
        .await;
    let project = Project::test(fs, [Path::new(path!("/root/scratch"))], cx).await;
    let buffer = project
        .update(cx, |project, cx| {
            project.open_local_buffer(path!("/root/scratch/notes.txt"), cx)
        })
        .await
        .expect("buffer opens");
    (project, buffer)
}

fn open_editor(
    project: &Entity<Project>,
    buffer: &Entity<Buffer>,
    cx: &mut TestAppContext,
) -> (Entity<Editor>, VisualTestContext) {
    let window = cx.add_window(|window, cx| {
        Editor::for_buffer(buffer.clone(), Some(project.clone()), window, cx)
    });
    let editor = window.root(cx).expect("editor window");
    let mut vcx = VisualTestContext::from_window(*window, cx);
    redraw(&mut vcx);
    (editor, vcx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn only_block(
    decorations: &Entity<Decorations>,
    editor: &Entity<Editor>,
    id: DecorationId,
    cx: &mut VisualTestContext,
) -> CustomBlockId {
    let blocks = decorations.read_with(cx, |decorations, _| {
        decorations.blocks_in(editor.entity_id(), id)
    });
    assert_eq!(blocks.len(), 1, "one block per matching buffer");
    blocks[0]
}

fn block_row(editor: &Entity<Editor>, block: CustomBlockId, cx: &mut VisualTestContext) -> u32 {
    editor
        .update(cx, |editor, cx| editor.row_for_block(block, cx))
        .expect("block is in the display map")
        .0
}

fn text_top(cx: &mut VisualTestContext) -> Pixels {
    cx.debug_bounds(TEXT_SELECTOR)
        .expect("the decoration's text is drawn")
        .origin
        .y
}

fn insert(buffer: &Entity<Buffer>, at: Point, text: &str, cx: &mut VisualTestContext) {
    buffer.update(cx, |buffer, cx| buffer.edit([(at..at, text)], None, cx));
    redraw(cx);
}

fn below(path: &str, row: u32) -> DecorationTarget {
    DecorationTarget {
        path: path.into(),
        row,
        placement: DecorationPlacement::Below,
        lines: None,
    }
}

#[gpui::test]
async fn test_block_follows_its_line_through_edits_above(cx: &mut TestAppContext) {
    let decorations = init(cx);
    let (project, buffer) = scratch_buffer("zero\none\ntwo\nthree\n", cx).await;
    let id = decorations
        .update(cx, |decorations, cx| {
            decorations.add_tree(below("scratch/notes.txt", 2), text_tree("note on two"), cx)
        })
        .expect("valid tree");

    let (editor, mut vcx) = open_editor(&project, &buffer, cx);
    let vcx = &mut vcx;
    let block = only_block(&decorations, &editor, id, vcx);
    // Display rows: zero, one, two, then the block.
    assert_eq!(block_row(&editor, block, vcx), 3);
    let top_before = text_top(vcx);

    // Two lines inserted at the top of the file.
    insert(&buffer, Point::new(0, 0), "new a\nnew b\n", vcx);
    assert_eq!(block_row(&editor, block, vcx), 5);
    let top_after_two = text_top(vcx);

    // A line typed directly above the decorated line, at its start ("two" is now row 4).
    insert(&buffer, Point::new(4, 0), "typed above\n", vcx);
    assert_eq!(block_row(&editor, block, vcx), 6);
    let top_after_three = text_top(vcx);

    // The drawn block moved with it: by two lines, then by one.
    let one_line = top_after_three - top_after_two;
    assert!(one_line > Pixels::ZERO, "block moved down: {one_line:?}");
    assert_eq!(top_after_two - top_before, one_line * 2.0);

    // Edits below the line and on the line itself leave the block where it is.
    insert(&buffer, Point::new(6, 0), "after\n", vcx);
    insert(&buffer, Point::new(5, 3), " (edited)", vcx);
    assert_eq!(block_row(&editor, block, vcx), 6);
    assert_eq!(
        buffer.read_with(vcx, |buffer, _| buffer.text()),
        "new a\nnew b\nzero\none\ntyped above\ntwo (edited)\nafter\nthree\n"
    );

    // An editor opened after the edits puts the block under the same logical line, not under
    // row 2 of the edited buffer.
    let (second, mut second_cx) = open_editor(&project, &buffer, cx);
    let second_block = only_block(&decorations, &second, id, &mut second_cx);
    assert_eq!(block_row(&second, second_block, &mut second_cx), 6);

    // Removing the decoration removes its blocks from every editor.
    decorations.update(vcx, |decorations, cx| decorations.remove(id, cx));
    redraw(vcx);
    redraw(&mut second_cx);
    for editor in [&editor, &second] {
        assert!(
            decorations
                .read_with(vcx, |decorations, _| decorations
                    .blocks_in(editor.entity_id(), id))
                .is_empty()
        );
    }
    assert!(vcx.debug_bounds(TEXT_SELECTOR).is_none());
}

#[gpui::test]
async fn test_updates_repaint_and_resize_the_block(cx: &mut TestAppContext) {
    let decorations = init(cx);
    let (project, buffer) = scratch_buffer("a\nb\nc\n", cx).await;
    let id = decorations
        .update(cx, |decorations, cx| {
            decorations.add_tree(
                DecorationTarget {
                    placement: DecorationPlacement::Above,
                    ..below("notes.txt", 1)
                },
                text_tree("first"),
                cx,
            )
        })
        .expect("valid tree");
    let (editor, mut vcx) = open_editor(&project, &buffer, cx);
    let vcx = &mut vcx;
    let block = only_block(&decorations, &editor, id, vcx);
    // Above row 1: a, then the block.
    assert_eq!(block_row(&editor, block, vcx), 1);
    // The row after the block is "b", one display row below it.
    let b_row = editor.update_in(vcx, |editor, window, cx| {
        let snapshot = editor.snapshot(window, cx);
        snapshot
            .display_snapshot
            .point_to_display_point(Point::new(1, 0), editor::Bias::Left)
            .row()
            .0
    });
    assert_eq!(b_row, 2);

    // A two-line text makes the block two lines tall.
    let applied = decorations.update(vcx, |decorations, cx| {
        decorations.update(
            id,
            Update::Patches(vec![Patch::SetProps(
                2,
                Props::Text(TextProps {
                    content: "first\nsecond".into(),
                    ..TextProps::default()
                }),
            )]),
            cx,
        )
    });
    assert!(applied);
    redraw(vcx);
    let b_row = editor.update_in(vcx, |editor, window, cx| {
        editor
            .snapshot(window, cx)
            .display_snapshot
            .point_to_display_point(Point::new(1, 0), editor::Bias::Left)
            .row()
            .0
    });
    assert_eq!(b_row, 3);
    let text = decorations.read_with(vcx, |decorations, cx| {
        let tree = decorations
            .state(id)
            .expect("decoration")
            .read(cx)
            .tree()
            .cloned();
        tree.and_then(|tree| match tree.get(2).map(|node| node.props().clone()) {
            Some(Props::Text(props)) => Some(props.content),
            _ => None,
        })
    });
    assert_eq!(text.as_deref(), Some("first\nsecond"));
}

#[gpui::test]
async fn test_other_files_get_no_block(cx: &mut TestAppContext) {
    let decorations = init(cx);
    let (project, buffer) = scratch_buffer("a\nb\n", cx).await;
    let id = decorations
        .update(cx, |decorations, cx| {
            decorations.add_tree(below("other/notes.txt", 0), text_tree("elsewhere"), cx)
        })
        .expect("valid tree");
    let (editor, mut vcx) = open_editor(&project, &buffer, cx);
    assert!(
        decorations
            .read_with(&mut vcx, |decorations, _| decorations
                .blocks_in(editor.entity_id(), id))
            .is_empty()
    );
}

#[test]
fn test_path_matching() {
    let full = Path::new("scratch/src/notes.txt");
    assert!(path_matches(Path::new("notes.txt"), full, None));
    assert!(path_matches(Path::new("src/notes.txt"), full, None));
    assert!(path_matches(Path::new("scratch/src/notes.txt"), full, None));
    // Whole components only.
    assert!(!path_matches(Path::new("tes.txt"), full, None));
    assert!(!path_matches(Path::new(""), full, None));
    // Absolute paths compare with the file's absolute path.
    let abs = std::env::temp_dir().join("scratch").join("notes.txt");
    assert!(path_matches(&abs, full, Some(&abs)));
    assert!(!path_matches(&abs, full, None));
}

#[test]
fn test_estimated_lines() {
    let text = |id, parent, content: &str| Node {
        id,
        parent: Some(parent),
        props: Props::Text(TextProps {
            content: content.into(),
            ..TextProps::default()
        }),
    };
    let flex = |id, parent, direction| Node {
        id,
        parent,
        props: Props::Flex(FlexProps {
            direction,
            ..FlexProps::default()
        }),
    };
    let tree = Tree::from_nodes(
        vec![
            flex(1, None, FlexDirection::Column),
            text(2, 1, "title"),
            flex(3, Some(1), FlexDirection::Row),
            text(4, 3, "left\nleft"),
            text(5, 3, "right"),
        ],
        TreeLimits::default(),
    )
    .expect("valid tree");
    assert_eq!(estimate_lines(&tree), 3);
    let tree = Tree::from_nodes(text_tree("one line"), TreeLimits::default()).expect("valid");
    assert_eq!(estimate_lines(&tree), 1);
}

#[test]
fn test_dev_file_format() {
    let entries = parse_dev_decorations(
        r#"[
            // A comment.
            { "path": "scratch/notes.txt", "line": 3, "text": "hello" },
            {
                "path": "notes.txt",
                "line": 1,
                "placement": "above",
                "lines": 2,
                "nodes": [
                    { "id": 1, "parent": null, "props": { "text": {
                        "content": "tree", "color": "warning", "bold": true, "monospace": false
                    } } }
                ]
            }
        ]"#,
    )
    .expect("parses");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, below("scratch/notes.txt", 2));
    assert_eq!(entries[0].1, text_tree("hello"));
    assert_eq!(
        entries[1].0,
        DecorationTarget {
            path: "notes.txt".into(),
            row: 0,
            placement: DecorationPlacement::Above,
            lines: Some(2),
        }
    );
    assert_eq!(entries[1].1.len(), 1);

    assert!(
        parse_dev_decorations(r#"[{ "path": "a", "line": 1 }]"#).is_err(),
        "neither text nor nodes"
    );
    assert!(
        parse_dev_decorations(r#"[{ "path": "a", "line": 1, "text": "x", "colour": 1 }]"#).is_err(),
        "unknown field"
    );
}
