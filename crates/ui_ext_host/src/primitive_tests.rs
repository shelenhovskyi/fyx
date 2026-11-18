//! Every primitive drawn from a hand-written tree (no WASM): each leaf must get a non-empty box,
//! tables must fetch and lay out their rows, disabled buttons must not send events.

use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use futures::{FutureExt as _, future::LocalBoxFuture};
use gpui::{AppContext as _, Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext};
use ui_ext::TreeLimits;
use ui_ext::v0_2::{
    ButtonProps, ButtonStyle, ColorToken, DiffViewProps, FlexDirection, FlexProps, IconProps,
    InputProps, Node, NodeId, Props, TableProps, TextProps, UiEvent, Update, tree::Tree,
};

use crate::{
    SurfaceState, SurfaceView,
    backend::{Reply, UiBackend},
};

const ROOT: NodeId = 1;
const COLORED_TEXT: NodeId = 2;
const DANGER: NodeId = 3;
const ICON_ONLY: NodeId = 4;
const DISABLED: NodeId = 5;
const UNKNOWN_ICON: NodeId = 6;
const KNOWN_ICON: NodeId = 7;
const MARKDOWN: NodeId = 8;
const DIFF: NodeId = 9;
const TABLE: NodeId = 10;
const INPUT: NodeId = 11;
const MONO_TEXT: NodeId = 12;

const TABLE_ROWS: u32 = 50;
const ROW_ROOT: NodeId = 0x8000_0000;

fn node(id: NodeId, parent: Option<NodeId>, props: Props) -> Node {
    Node { id, parent, props }
}

fn button(label: &str, icon: Option<&str>, style: ButtonStyle, disabled: bool) -> Props {
    Props::Button(ButtonProps {
        label: label.into(),
        icon: icon.map(|name| ui_ext::v0_2::IconRef::Upstream(name.into())),
        style,
        disabled,
        tooltip: None,
    })
}

fn text(content: impl Into<String>) -> Props {
    Props::Text(TextProps {
        content: content.into(),
        ..TextProps::default()
    })
}

fn nodes() -> Vec<Node> {
    let root = Some(ROOT);
    vec![
        node(
            ROOT,
            None,
            Props::Flex(FlexProps {
                direction: FlexDirection::Column,
                gap: 4,
                padding: 4,
                grow: true,
                background: Some(ColorToken::Surface),
                ..FlexProps::default()
            }),
        ),
        node(
            COLORED_TEXT,
            root,
            Props::Text(TextProps {
                content: "an error, in the error colour".into(),
                color: Some(ColorToken::Error),
                bold: true,
                monospace: false,
            }),
        ),
        node(
            DANGER,
            root,
            button("Delete", Some("trash"), ButtonStyle::Danger, false),
        ),
        node(
            ICON_ONLY,
            root,
            button("", Some("settings"), ButtonStyle::Subtle, false),
        ),
        node(
            DISABLED,
            root,
            button("Disabled", None, ButtonStyle::Filled, true),
        ),
        node(
            UNKNOWN_ICON,
            root,
            Props::Icon(IconProps {
                name: ui_ext::v0_2::IconRef::Upstream("fyx_no_such_icon".into()),
                color: None,
            }),
        ),
        node(
            KNOWN_ICON,
            root,
            Props::Icon(IconProps {
                name: ui_ext::v0_2::IconRef::Upstream("check".into()),
                color: Some(ColorToken::Success),
            }),
        ),
        node(
            MARKDOWN,
            root,
            Props::Markdown(
                "# Heading\n\nA paragraph with **bold** and `code`.\n\n- one\n- two\n- three"
                    .into(),
            ),
        ),
        node(
            DIFF,
            root,
            Props::DiffView(DiffViewProps {
                old_text: "fn a() {}\nfn b() {}\nfn c() {}\n".into(),
                new_text: "fn a() {}\nfn b2() {}\nfn c() {}\n".into(),
                language: Some("rust".into()),
            }),
        ),
        node(
            TABLE,
            root,
            Props::Table(TableProps {
                columns: vec!["Name".into(), "Kind".into(), "Lines".into()],
                row_count: TABLE_ROWS,
            }),
        ),
        node(
            INPUT,
            root,
            Props::Input(InputProps {
                value: "draft".into(),
                placeholder: "Filter".into(),
                secret: false,
                label: None,
            }),
        ),
        node(
            MONO_TEXT,
            root,
            Props::Text(TextProps {
                content: "monospace".into(),
                color: Some(ColorToken::TextMuted),
                bold: false,
                monospace: true,
            }),
        ),
    ]
}

/// A table row: a row flex with one text per column.
fn table_row(index: u32) -> Tree {
    let mut nodes = vec![node(
        ROW_ROOT,
        None,
        Props::Flex(FlexProps {
            direction: FlexDirection::Row,
            ..FlexProps::default()
        }),
    )];
    for (column, content) in [
        format!("file_{index}.rs"),
        "source".into(),
        format!("{}", index * 7),
    ]
    .into_iter()
    .enumerate()
    {
        nodes.push(node(
            ROW_ROOT + 1 + column as u32,
            Some(ROW_ROOT),
            text(content),
        ));
    }
    Tree::from_row(nodes, TreeLimits::default()).expect("valid row")
}

#[derive(Default)]
struct StaticBackend {
    events: RefCell<Vec<UiEvent>>,
}

impl UiBackend for StaticBackend {
    fn extension_id(&self) -> &str {
        "static"
    }

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        let tree = Tree::from_nodes(nodes(), TreeLimits::default()).map_err(anyhow::Error::from);
        async move {
            tree.map(|value| Reply {
                effects: Vec::new(),
                value,
            })
        }
        .boxed_local()
    }

    fn handle_event(
        &self,
        _: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        self.events.borrow_mut().push(event);
        async move {
            Ok(Reply {
                effects: Vec::new(),
                value: Update::Unchanged,
            })
        }
        .boxed_local()
    }

    fn list_rows(
        &self,
        _: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<crate::backend::Row>>>> {
        let rows = if list == TABLE {
            (start..(start + count).min(TABLE_ROWS))
                .map(|index| crate::backend::Row {
                    index,
                    indent: 0,
                    tree: table_row(index),
                })
                .collect()
        } else {
            Vec::new()
        };
        async move {
            Ok(Reply {
                effects: Vec::new(),
                value: rows,
            })
        }
        .boxed_local()
    }
}

fn open(
    cx: &mut TestAppContext,
) -> (
    Rc<StaticBackend>,
    Entity<SurfaceState>,
    Entity<SurfaceView>,
    &mut VisualTestContext,
) {
    crate::tests::init_test(cx);
    let backend = Rc::new(StaticBackend::default());
    let surface = ui_ext::v0_2::Surface {
        id: "static".into(),
        title: "Static".into(),
        kind: ui_ext::v0_2::SurfaceKind::Panel(ui_ext::v0_2::PanelDescriptor {
            position: ui_ext::v0_2::DockPosition::Right,
            icon: ui_ext::v0_2::IconRef::Upstream("hash".into()),
            default_size: 320,
        }),
    };
    let state = {
        let backend: Rc<dyn UiBackend> = backend.clone();
        cx.new(|cx| SurfaceState::new(backend, surface, None, cx))
    };
    cx.run_until_parked();
    let (view, vcx) =
        cx.add_window_view(|window, cx| SurfaceView::new(state.clone(), false, window, cx));
    vcx.run_until_parked();
    // The table's rows arrive after the first layout asked for them; draw again.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    (backend, state, view, vcx)
}

fn bounds(selector: String, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    let selector: &'static str = selector.leak();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"));
    assert!(
        bounds.size.width > Pixels::ZERO && bounds.size.height > Pixels::ZERO,
        "{selector} has an empty box: {bounds:?}"
    );
    bounds
}

#[gpui::test]
async fn test_every_leaf_primitive_gets_a_box(cx: &mut TestAppContext) {
    let (_backend, _state, _view, vcx) = open(cx);
    for id in [
        COLORED_TEXT,
        DANGER,
        ICON_ONLY,
        DISABLED,
        UNKNOWN_ICON,
        KNOWN_ICON,
        MARKDOWN,
        DIFF,
        INPUT,
        MONO_TEXT,
    ] {
        bounds(format!("fyx-node-{id}"), vcx);
    }

    // The unknown icon falls back to a real icon of the same size as a known one.
    let unknown = bounds(format!("fyx-node-{UNKNOWN_ICON}"), vcx);
    let known = bounds(format!("fyx-node-{KNOWN_ICON}"), vcx);
    assert_eq!(unknown.size, known.size);

    // Markdown and the diff placeholder lay out several lines, not a single label.
    let line = bounds(format!("fyx-node-{MONO_TEXT}"), vcx).size.height;
    let markdown = bounds(format!("fyx-node-{MARKDOWN}"), vcx).size.height;
    assert!(
        markdown > line * 4.0,
        "markdown is {markdown:?}, one line {line:?}"
    );
    // Three context/removed/added lines at least: a, -b, +b2, c.
    let diff = bounds(format!("fyx-node-{DIFF}"), vcx).size.height;
    assert!(
        diff > line * 3.0,
        "diff view is {diff:?}, one line {line:?}"
    );
}

#[gpui::test]
async fn test_table_fetches_rows_and_lays_out_cells(cx: &mut TestAppContext) {
    let (_backend, state, _view, vcx) = open(cx);

    let row = state.read_with(vcx, |state, _| state.row(TABLE, 0).cloned());
    let row = row.expect("the visible rows were fetched");
    assert_eq!(row.children(Some(ROW_ROOT)).len(), 3);

    // Rows span the table's width and stack at the default row height.
    let first = bounds(format!("fyx-row-{TABLE}-0"), vcx);
    let second = bounds(format!("fyx-row-{TABLE}-1"), vcx);
    assert_eq!(first.size.width, second.size.width);
    assert!(second.origin.y > first.origin.y);
    assert_eq!(first.size.height, gpui::px(24.));
    // A row is not drawn past what is visible.
    let visible_rows = state.read_with(vcx, |state, _| {
        (0..TABLE_ROWS)
            .filter(|index| state.row(TABLE, *index).is_some())
            .count()
    });
    assert!(
        visible_rows < TABLE_ROWS as usize,
        "{visible_rows} rows fetched"
    );
}

#[gpui::test]
async fn test_buttons_route_clicks_except_disabled(cx: &mut TestAppContext) {
    let (backend, _state, _view, vcx) = open(cx);
    for id in [DISABLED, ICON_ONLY, DANGER] {
        let bounds = bounds(format!("fyx-node-{id}"), vcx);
        vcx.simulate_click(bounds.center(), Modifiers::none());
        vcx.run_until_parked();
    }
    assert_eq!(
        backend.events.borrow().as_slice(),
        [UiEvent::Click(ICON_ONLY), UiEvent::Click(DANGER)],
        "a disabled button sends nothing"
    );
}

#[gpui::test]
async fn test_input_shows_its_value_and_user_text_stands(cx: &mut TestAppContext) {
    let (backend, state, view, vcx) = open(cx);
    let editor = view.read_with(vcx, |view, _| {
        view.inputs
            .get(&INPUT)
            .map(|input| input.editor.clone())
            .expect("input has an editor")
    });
    assert_eq!(editor.read_with(vcx, |editor, cx| editor.text(cx)), "draft");

    // An update that does not set the input leaves the user's text alone, focused or not.
    vcx.update(|window, cx| {
        use gpui::Focusable as _;
        editor.focus_handle(cx).focus(window, cx)
    });
    vcx.run_until_parked();
    vcx.simulate_input("ed");
    state.update(vcx, |state, cx| {
        state.apply_update(
            Update::Patches(vec![ui_ext::v0_2::Patch::SetProps(
                MONO_TEXT,
                text("changed"),
            )]),
            cx,
        );
    });
    vcx.run_until_parked();
    assert_eq!(
        editor.read_with(vcx, |editor, cx| editor.text(cx)),
        "drafted"
    );

    // The typed text reaches the extension once, after the pause.
    vcx.executor().advance_clock(crate::TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(
        backend.events.borrow().as_slice(),
        [UiEvent::InputChanged(INPUT, "drafted".into())]
    );
}

/// `diff-view` is Zed's diff editor (the agent panel's `acp_thread::Diff`): read-only, showing the
/// new text with the changed hunk expanded.
#[gpui::test]
async fn test_diff_view_is_a_read_only_diff_editor(cx: &mut TestAppContext) {
    let (_backend, _state, view, vcx) = open(cx);
    let editor = view.read_with(vcx, |view, _| view.diffs[&DIFF].editor.clone());
    vcx.run_until_parked();
    editor.read_with(vcx, |editor, cx| {
        assert!(editor.read_only(cx));
        // The expanded hunk shows the removed line above its replacement.
        assert_eq!(
            editor.text(cx),
            "fn a() {}\nfn b() {}\nfn b2() {}\nfn c() {}\n"
        );
        let hunks = editor.buffer().read(cx).snapshot(cx).diff_hunks().count();
        assert_eq!(hunks, 1, "one changed line, one hunk");
    });
}

/// The language name picks the file suffix the diff's syntax highlighting comes from.
#[gpui::test]
async fn test_diff_view_language_from_name(cx: &mut TestAppContext) {
    let languages = std::sync::Arc::new(language::LanguageRegistry::test(cx.executor()));
    languages.add(std::sync::Arc::new(language::Language::new(
        language::LanguageConfig {
            name: "Rust".into(),
            matcher: std::sync::Arc::new(language::LanguageMatcher {
                path_suffixes: vec!["rs".into()],
                ..Default::default()
            }),
            ..Default::default()
        },
        None,
    )));
    assert_eq!(crate::view::diff_path(Some("rust"), &languages), "diff.rs");
    assert_eq!(crate::view::diff_path(Some("Rust"), &languages), "diff.rs");
    assert_eq!(
        crate::view::diff_path(Some("klingon"), &languages),
        "diff.txt"
    );
    assert_eq!(crate::view::diff_path(None, &languages), "diff.txt");
}

/// Triage-3 #5 (PERF 1): how long a redraw takes when nothing in the surface changed (a caret
/// blink elsewhere), for a tree at the node cap. Prints the timing; run with `--ignored`.
#[gpui::test]
#[ignore]
async fn bench_redraw_of_a_large_tree(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let flex = |direction| {
        Props::Flex(FlexProps {
            direction,
            ..FlexProps::default()
        })
    };
    let mut nodes = vec![node(1, None, flex(FlexDirection::Column))];
    let mut next = 2;
    for _ in 0..128 {
        let group = next;
        next += 1;
        nodes.push(node(group, Some(1), flex(FlexDirection::Row)));
        for i in 0..126 {
            nodes.push(node(next, Some(group), text(format!("cell {i}"))));
            next += 1;
        }
    }
    let count = nodes.len();
    let tree = Tree::from_nodes(nodes, TreeLimits::default()).expect("valid tree");
    let backend: Rc<dyn UiBackend> = Rc::new(StaticBackend::default());
    let surface = ui_ext::v0_2::Surface {
        id: "static".into(),
        title: "Static".into(),
        kind: ui_ext::v0_2::SurfaceKind::Panel(ui_ext::v0_2::PanelDescriptor {
            position: ui_ext::v0_2::DockPosition::Right,
            icon: ui_ext::v0_2::IconRef::Upstream("hash".into()),
            default_size: 320,
        }),
    };
    let state = cx.new(|cx| SurfaceState::with_tree(backend, surface, None, tree, cx));
    // How a panel hosts it: as a cached view (`AnyView::cached`), so a redraw caused by
    // something else (a caret blinking in another editor) reuses its last frame.
    struct CachedHost(Entity<SurfaceView>);
    impl gpui::Render for CachedHost {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement as _, Styled as _};
            gpui::div().size_full().child(
                gpui::AnyView::from(self.0.clone())
                    .cached(gpui::StyleRefinement::default().size_full()),
            )
        }
    }
    let (host, vcx) = cx.add_window_view(|window, cx| {
        CachedHost(cx.new(|cx| SurfaceView::new(state, false, window, cx)))
    });
    vcx.run_until_parked();
    for round in 0..3 {
        let started = std::time::Instant::now();
        // As a caret blink elsewhere does: one other view notifies, the window redraws.
        host.update(vcx, |_, cx| cx.notify());
        vcx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        eprintln!("redraw {round} of {count} nodes: {:?}", started.elapsed());
    }
}
