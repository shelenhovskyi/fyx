use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use anyhow::{Result, anyhow};
use futures::{FutureExt as _, future::LocalBoxFuture};
use gpui::{
    AppContext as _, Entity, Focusable as _, KeyBinding, Modifiers, TestAppContext,
    VisualTestContext,
};
use settings::SettingsStore;
use ui_ext::{
    ButtonProps, FlexDirection, FlexProps, InputProps, ListProps, Node, NodeId, Patch, Props,
    TextProps, Tree, TreeLimits, UiEvent, Update,
};

use crate::{
    SurfaceState, SurfaceView, TYPING_DEBOUNCE,
    backend::UiBackend,
    surface::fetch_range,
    view::{DiffLine, diff_lines, icon_name},
};

const LABEL: NodeId = 2;
const INCREMENT: NodeId = 5;
const LIST: NodeId = 7;
const INPUT: NodeId = 9;
const BROKEN: NodeId = 11;
const ROW_ROOT: NodeId = 0x8000_0000;

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Mount,
    Event(UiEvent),
    Rows {
        list: NodeId,
        start: u32,
        count: u32,
    },
}

/// A counter-like extension: a label, an increment button, a list with one row per count and a
/// filter input that the extension clears on submit. Clicking `BROKEN` answers with a patch for
/// an unknown node.
#[derive(Default)]
struct FakeBackend {
    calls: RefCell<Vec<Call>>,
    count: Cell<u32>,
    fail_rows: Cell<bool>,
    short_rows: Cell<bool>,
}

impl FakeBackend {
    fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    fn take_calls(&self) -> Vec<Call> {
        self.calls.take()
    }

    fn events(&self) -> Vec<UiEvent> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Event(event) => Some(event),
                Call::Mount | Call::Rows { .. } => None,
            })
            .collect()
    }
}

fn node(id: NodeId, parent: Option<NodeId>, props: impl Into<Props>) -> Node {
    Node {
        id,
        parent,
        props: props.into(),
    }
}

fn text(content: impl Into<String>) -> Props {
    Props::Text(TextProps {
        content: content.into(),
        ..TextProps::default()
    })
}

fn list(row_count: u32) -> Props {
    Props::List(ListProps {
        row_count,
        row_height: 24,
    })
}

fn input(value: &str) -> Props {
    Props::Input(InputProps {
        value: value.into(),
        placeholder: "Filter".into(),
    })
}

fn button(label: &str) -> Props {
    Props::Button(ButtonProps {
        label: label.into(),
        ..ButtonProps::default()
    })
}

impl UiBackend for FakeBackend {
    fn extension_id(&self) -> &str {
        "fake"
    }

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Tree>> {
        self.calls.borrow_mut().push(Call::Mount);
        let count = self.count.get();
        let nodes = vec![
            node(
                1,
                None,
                Props::Flex(FlexProps {
                    direction: FlexDirection::Column,
                    gap: 8,
                    padding: 8,
                    grow: true,
                    ..FlexProps::default()
                }),
            ),
            node(LABEL, Some(1), text(format!("Count: {count}"))),
            node(INCREMENT, Some(1), button("Increment")),
            node(BROKEN, Some(1), button("Broken")),
            node(INPUT, Some(1), input("")),
            node(LIST, Some(1), list(count)),
        ];
        let tree = Tree::from_nodes(nodes, TreeLimits::default()).map_err(anyhow::Error::from);
        async move { tree }.boxed_local()
    }

    fn handle_event(&self, _: &str, event: UiEvent) -> LocalBoxFuture<'static, Result<Update>> {
        self.calls.borrow_mut().push(Call::Event(event.clone()));
        let update = match event {
            UiEvent::Click(INCREMENT) => {
                let count = self.count.get() + 1;
                self.count.set(count);
                Update::Patches(vec![
                    Patch::SetProps(LABEL, text(format!("Count: {count}"))),
                    Patch::SetProps(LIST, list(count)),
                ])
            }
            UiEvent::Click(BROKEN) => Update::Patches(vec![Patch::SetProps(99, text("nope"))]),
            UiEvent::InputSubmitted(INPUT, _) => {
                Update::Patches(vec![Patch::SetProps(INPUT, input(""))])
            }
            _ => Update::Unchanged,
        };
        async move { Ok(update) }.boxed_local()
    }

    fn list_rows(
        &self,
        _: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Vec<(u32, Tree)>>> {
        self.calls
            .borrow_mut()
            .push(Call::Rows { list, start, count });
        let result = if self.fail_rows.get() {
            Err(anyhow!("rows unavailable"))
        } else {
            let total = self.count.get();
            let end = (start + count).min(total);
            let end = if self.short_rows.get() {
                end.min(start + 1)
            } else {
                end
            };
            Ok((start..end)
                .map(|index| {
                    let row = vec![node(
                        ROW_ROOT,
                        None,
                        text(format!("Row {} of {total}", index + 1)),
                    )];
                    (
                        index,
                        Tree::from_row(row, TreeLimits::default()).expect("valid row"),
                    )
                })
                .collect())
        };
        async move { result }.boxed_local()
    }
}

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings = SettingsStore::test(cx);
        cx.set_global(settings);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
        cx.bind_keys([KeyBinding::new("enter", menu::Confirm, None)]);
    });
}

fn new_state(backend: &Rc<FakeBackend>, cx: &mut TestAppContext) -> Entity<SurfaceState> {
    let surface = ui_ext::Surface {
        id: "counter".into(),
        title: "Counter".into(),
        kind: ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
            position: ui_ext::DockPosition::Right,
            icon: "hash".into(),
            default_size: 320,
        }),
    };
    let backend: Rc<dyn UiBackend> = backend.clone();
    let state = cx.new(|cx| SurfaceState::new(backend, surface, cx));
    cx.run_until_parked();
    state
}

fn props_of(state: &Entity<SurfaceState>, id: NodeId, cx: &mut TestAppContext) -> Props {
    state.read_with(cx, |state, _| {
        state
            .tree()
            .and_then(|tree| tree.get(id))
            .map(|node| node.props().clone())
            .expect("node exists")
    })
}

fn row_text(
    state: &Entity<SurfaceState>,
    list: NodeId,
    index: u32,
    cx: &mut TestAppContext,
) -> Option<Props> {
    state.read_with(cx, |state, _| {
        let tree = state.row(list, index)?;
        tree.get(tree.roots()[0]).map(|node| node.props().clone())
    })
}

fn open_view<'a>(
    state: &Entity<SurfaceState>,
    cx: &'a mut TestAppContext,
) -> (Entity<SurfaceView>, &'a mut VisualTestContext) {
    let (view, cx) =
        cx.add_window_view(|window, cx| SurfaceView::new(state.clone(), false, window, cx));
    cx.run_until_parked();
    (view, cx)
}

#[gpui::test]
async fn test_click_makes_one_call_and_repaints(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (_view, vcx) = open_view(&state, cx);
    assert_eq!(backend.take_calls(), [Call::Mount]);

    let bounds = vcx
        .debug_bounds("fyx-node-5")
        .expect("increment button is drawn");
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();

    let calls = backend.take_calls();
    assert_eq!(calls[0], Call::Event(UiEvent::Click(INCREMENT)));
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, Call::Event(_)))
            .count(),
        1,
        "exactly one handle-event call per click: {calls:?}"
    );
    assert_eq!(props_of(&state, LABEL, vcx), text("Count: 1"));
    // The list's props changed, so the view fetched its visible rows after the repaint.
    assert!(
        calls.contains(&Call::Rows {
            list: LIST,
            start: 0,
            count: 1
        }),
        "{calls:?}"
    );
    assert_eq!(row_text(&state, LIST, 0, vcx), Some(text("Row 1 of 1")));

    // A second click refreshes the rows that are already cached.
    let bounds = vcx.debug_bounds("fyx-node-5").expect("still drawn");
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(props_of(&state, LABEL, vcx), text("Count: 2"));
    assert_eq!(row_text(&state, LIST, 0, vcx), Some(text("Row 1 of 2")));
    assert_eq!(row_text(&state, LIST, 1, vcx), Some(text("Row 2 of 2")));

    // Repainting with everything cached makes no calls.
    backend.take_calls();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(backend.calls(), []);
}

#[gpui::test]
async fn test_events_are_serialized_and_typing_is_coalesced(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    backend.take_calls();

    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx);
        state.dispatch(UiEvent::InputChanged(INPUT, "a".into()), cx);
        state.dispatch(UiEvent::InputChanged(INPUT, "ab".into()), cx);
        state.dispatch(UiEvent::Click(INCREMENT), cx);
        state.dispatch(UiEvent::Click(INCREMENT), cx);
        // Events for nodes that are not in the tree never reach the extension.
        state.dispatch(UiEvent::Click(1234), cx);
    });
    cx.run_until_parked();

    assert_eq!(
        backend.events(),
        [
            UiEvent::Click(INCREMENT),
            UiEvent::InputChanged(INPUT, "ab".into()),
            UiEvent::Click(INCREMENT),
            UiEvent::Click(INCREMENT),
        ]
    );
    assert_eq!(props_of(&state, LABEL, cx), text("Count: 3"));
}

#[gpui::test]
async fn test_invalid_update_mounts_again(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx)
    });
    cx.run_until_parked();
    backend.take_calls();

    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(BROKEN), cx));
    cx.run_until_parked();

    assert_eq!(
        backend.take_calls(),
        [Call::Event(UiEvent::Click(BROKEN)), Call::Mount]
    );
    // The fresh mount reflects the extension's own state.
    assert_eq!(props_of(&state, LABEL, cx), text("Count: 1"));
    state.read_with(cx, |state, _| {
        let error = state.last_error().expect("error is shown");
        assert!(error.contains("unknown node id 99"), "{error}");
    });
}

#[gpui::test]
async fn test_rows_are_fetched_once_per_change(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(50);
    let state = new_state(&backend, cx);
    backend.take_calls();

    let request = |state: &Entity<SurfaceState>, range, cx: &mut TestAppContext| {
        state.update(cx, |state, cx| state.request_rows(LIST, range, cx));
        cx.run_until_parked();
    };

    request(&state, 0..10, cx);
    request(&state, 0..10, cx);
    request(&state, 2..8, cx);
    assert_eq!(
        backend.take_calls(),
        [Call::Rows {
            list: LIST,
            start: 0,
            count: 10
        }]
    );

    // An extension that answers with fewer rows is not asked again on every frame.
    backend.short_rows.set(true);
    request(&state, 20..30, cx);
    request(&state, 20..30, cx);
    assert_eq!(backend.take_calls().len(), 1);
    backend.short_rows.set(false);

    // A failing fetch is not retried until the list changes.
    backend.fail_rows.set(true);
    request(&state, 40..50, cx);
    request(&state, 41..50, cx);
    assert_eq!(backend.take_calls().len(), 1);
    backend.fail_rows.set(false);
    state.update(cx, |state, cx| state.dismiss_error(cx));

    // Changing the list's props invalidates its rows, but stale rows stay visible meanwhile.
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx)
    });
    cx.run_until_parked();
    assert_eq!(row_text(&state, LIST, 0, cx), Some(text("Row 1 of 50")));
    backend.take_calls();
    request(&state, 0..10, cx);
    assert_eq!(
        backend.take_calls(),
        [Call::Rows {
            list: LIST,
            start: 0,
            count: 10
        }]
    );
    assert_eq!(row_text(&state, LIST, 0, cx), Some(text("Row 1 of 51")));
}

#[gpui::test]
async fn test_typing_is_debounced_and_submit_clears(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    backend.take_calls();

    let editor = view.read_with(vcx, |view, _| {
        view.inputs
            .get(&INPUT)
            .map(|input| input.editor.clone())
            .expect("input has an editor")
    });
    vcx.update(|window, cx| editor.focus_handle(cx).focus(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("h");
    vcx.simulate_input("i");
    vcx.run_until_parked();
    assert_eq!(backend.events(), [], "nothing is sent while typing");

    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(
        backend.take_calls(),
        [Call::Event(UiEvent::InputChanged(INPUT, "hi".into()))]
    );

    vcx.simulate_input("!");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    // The submit carries the text, so the pending change is not sent separately.
    assert_eq!(
        backend.take_calls(),
        [Call::Event(UiEvent::InputSubmitted(INPUT, "hi!".into()))]
    );
    // The extension cleared the field in its answer to the submit; that wins while focused.
    assert_eq!(editor.read_with(vcx, |editor, cx| editor.text(cx)), "");
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(backend.calls(), [], "clearing is not echoed back");
}

#[test]
fn test_diff_lines() {
    assert_eq!(
        diff_lines("a\nb\nc\nd", "a\nx\ny\nd"),
        [
            (DiffLine::Context, "a"),
            (DiffLine::Removed, "b"),
            (DiffLine::Removed, "c"),
            (DiffLine::Added, "x"),
            (DiffLine::Added, "y"),
            (DiffLine::Context, "d"),
        ]
    );
    assert_eq!(diff_lines("same", "same"), [(DiffLine::Context, "same")]);
    assert_eq!(diff_lines("", "new"), [(DiffLine::Added, "new")]);
    assert_eq!(
        diff_lines("a\na", "a"),
        [(DiffLine::Context, "a"), (DiffLine::Removed, "a")]
    );
}

#[test]
fn test_icon_names_and_fetch_range() {
    assert_eq!(icon_name("list_todo"), ui::IconName::ListTodo);
    assert_eq!(icon_name("rotate_cw"), ui::IconName::RotateCw);
    assert_eq!(icon_name("no-such-icon"), ui::IconName::Blocks);

    assert_eq!(fetch_range(0..5, 100), 0..13);
    assert_eq!(fetch_range(50..60, 100), 42..68);
    assert_eq!(fetch_range(95..100, 100), 87..100);
    assert_eq!(fetch_range(0..0, 0), 0..0);
}
