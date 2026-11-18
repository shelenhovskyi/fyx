use std::time::Duration;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use anyhow::{Result, anyhow};
use futures::{FutureExt as _, future::LocalBoxFuture};
use gpui::{
    App, AppContext as _, Entity, Focusable as _, KeyBinding, Modifiers, TestAppContext,
    VisualTestContext,
};
use settings::SettingsStore;
use ui_ext::TreeLimits;
use ui_ext::v0_2::{
    ButtonProps, FlexDirection, FlexProps, InputProps, ListProps, Node, NodeId, Patch, Props,
    TextProps, UiEvent, Update, tree::Tree,
};

use crate::{
    SurfaceState, SurfaceView, TYPING_DEBOUNCE,
    backend::{HostEffect, Reply, ReplyError, UiBackend},
    icons::upstream_icon as icon_name,
    surface::fetch_range,
    view::{DiffLine, diff_lines},
};

const LABEL: NodeId = 2;
const INCREMENT: NodeId = 5;
const LIST: NodeId = 7;
const INPUT: NodeId = 9;
const BROKEN: NodeId = 11;
/// Answers with a push made during the call, then the reply.
const PUSHER: NodeId = 13;
/// Sets the input's value without a submit.
const SET_INPUT: NodeId = 15;
/// Pushes, then fails with an extension-reported error.
const PUSH_THEN_FAIL: NodeId = 17;
/// Pushes an own patch that does not fit, then a push for another surface, then replies.
const BAD_OWN_THEN_OTHER: NodeId = 19;
/// Fails like a trapped or disabled instance: an error without a `ReplyError`.
const FATAL: NodeId = 21;
/// Its reply removes it.
const REMOVER: NodeId = 23;
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
    Tick,
}

/// A counter-like extension: a label, an increment button, a list with one row per count and a
/// filter input that the extension clears on submit. Clicking `BROKEN` answers with a patch for
/// an unknown node.
#[derive(Default)]
struct FakeBackend {
    calls: RefCell<Vec<Call>>,
    count: Cell<u32>,
    fail_rows: Cell<bool>,
    /// Surfaces that pushes for other surfaces were routed to.
    routed: RefCell<Vec<String>>,
    short_rows: Cell<bool>,
    /// `list-rows` pushes an own patch that does not fit, every time.
    bad_rows_push: Cell<bool>,
    /// Surfaces that reported the whole extension as failed.
    failed_from: RefCell<Vec<String>>,
    /// The input's id in the next mount, as an SDK remount hands out new ids.
    renumbered_input: Cell<Option<NodeId>>,
    /// Restart requests that reached the link.
    restarts: Cell<u32>,
    /// Ticks answered so far; each sets the label to `Tick {n}`.
    ticks: Cell<u32>,
    /// Each tick asks for the next one, this long after.
    tick_again: Cell<Option<Duration>>,
    /// Each tick fails with the extension's own error.
    fail_ticks: Cell<bool>,
    /// Modals the renderer reported as not shown.
    modals_closed: RefCell<Vec<String>>,
}

struct FakeLink(Rc<FakeBackend>);

impl crate::surface::SurfaceLink for FakeLink {
    fn route_effect(&self, effect: HostEffect, _: &mut App) {
        if let HostEffect::Push { surface_id, .. } = effect {
            self.0.routed.borrow_mut().push(surface_id);
        }
    }

    fn extension_failed(&self, from_surface: &str, _: gpui::SharedString, _: &mut App) {
        self.0
            .failed_from
            .borrow_mut()
            .push(from_surface.to_string());
    }

    fn restart_extension(&self, _: &mut App) {
        self.0.restarts.set(self.0.restarts.get() + 1);
    }
}

fn unfit_push() -> HostEffect {
    HostEffect::Push {
        surface_id: "counter".into(),
        update: Update::Patches(vec![Patch::SetProps(99, text("no such node"))]),
    }
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
                Call::Mount | Call::Rows { .. } | Call::Tick => None,
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
        selected: None,
    })
}

fn input(value: &str) -> Props {
    Props::Input(InputProps {
        value: value.into(),
        placeholder: "Filter".into(),
        secret: false,
        label: None,
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

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
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
            node(PUSHER, Some(1), button("Push")),
            node(SET_INPUT, Some(1), button("Set input")),
            node(PUSH_THEN_FAIL, Some(1), button("Push then fail")),
            node(BAD_OWN_THEN_OTHER, Some(1), button("Bad push")),
            node(FATAL, Some(1), button("Trap")),
            node(REMOVER, Some(1), button("Remove me")),
            node(
                self.renumbered_input.get().unwrap_or(INPUT),
                Some(1),
                input(""),
            ),
            node(LIST, Some(1), list(count)),
        ];
        let tree = Tree::from_nodes(nodes, TreeLimits::default()).map_err(anyhow::Error::from);
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
        self.calls.borrow_mut().push(Call::Event(event.clone()));
        if event == UiEvent::Click(BAD_OWN_THEN_OTHER) {
            let reply = Reply {
                effects: vec![
                    unfit_push(),
                    HostEffect::Push {
                        surface_id: "other".into(),
                        update: Update::Unchanged,
                    },
                ],
                value: Update::Patches(vec![Patch::SetProps(LABEL, text("never applied"))]),
            };
            return async move { Ok(reply) }.boxed_local();
        }
        if event == UiEvent::Click(FATAL) {
            return async move { Err(anyhow!("wasm trap: interrupt")) }.boxed_local();
        }
        if event == UiEvent::Click(REMOVER) {
            let reply = Reply {
                effects: Vec::new(),
                value: Update::Patches(vec![Patch::Remove(REMOVER)]),
            };
            return async move { Ok(reply) }.boxed_local();
        }
        if event == UiEvent::Click(PUSH_THEN_FAIL) {
            let error = anyhow::Error::new(ReplyError {
                effects: vec![HostEffect::Push {
                    surface_id: "counter".into(),
                    update: Update::Patches(vec![Patch::SetProps(
                        LABEL,
                        text("pushed, then failed"),
                    )]),
                }],
                message: "the extension gave up".into(),
            })
            .context("from fyx:ui extension fake");
            return async move { Err(error) }.boxed_local();
        }
        if event == UiEvent::Click(PUSHER) {
            let reply = Reply {
                effects: vec![
                    HostEffect::Push {
                        surface_id: "counter".into(),
                        update: Update::Patches(vec![Patch::SetProps(LABEL, text("pushed"))]),
                    },
                    HostEffect::Push {
                        surface_id: "other".into(),
                        update: Update::Unchanged,
                    },
                ],
                value: Update::Patches(vec![Patch::SetProps(LABEL, text("replied"))]),
            };
            return async move { Ok(reply) }.boxed_local();
        }
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
            UiEvent::Click(SET_INPUT) => {
                Update::Patches(vec![Patch::SetProps(INPUT, input("from extension"))])
            }
            UiEvent::InputSubmitted(INPUT, _) => {
                Update::Patches(vec![Patch::SetProps(INPUT, input(""))])
            }
            _ => Update::Unchanged,
        };
        async move {
            Ok(Reply {
                effects: Vec::new(),
                value: update,
            })
        }
        .boxed_local()
    }

    fn modal_closed(&self, surface_id: &str) {
        self.modals_closed.borrow_mut().push(surface_id.to_string());
    }

    fn tick(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        self.calls.borrow_mut().push(Call::Tick);
        if self.fail_ticks.get() {
            let error = anyhow::Error::new(ReplyError {
                effects: Vec::new(),
                message: "tick failed".into(),
            });
            return async move { Err(error) }.boxed_local();
        }
        let n = self.ticks.get() + 1;
        self.ticks.set(n);
        let effects = self
            .tick_again
            .get()
            .map(|delay| HostEffect::RequestTick {
                surface_id: "counter".into(),
                delay,
            })
            .into_iter()
            .collect();
        let reply = Reply {
            effects,
            value: Update::Patches(vec![Patch::SetProps(LABEL, text(format!("Tick {n}")))]),
        };
        async move { Ok(reply) }.boxed_local()
    }

    fn list_rows(
        &self,
        _: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<crate::backend::Row>>>> {
        self.calls
            .borrow_mut()
            .push(Call::Rows { list, start, count });
        let result = if self.fail_rows.get() {
            // An extension-reported error: the instance lives on.
            Err(anyhow::Error::new(ReplyError {
                effects: Vec::new(),
                message: "rows unavailable".into(),
            }))
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
                    crate::backend::Row {
                        index,
                        indent: 0,
                        tree: Tree::from_row(row, TreeLimits::default()).expect("valid row"),
                    }
                })
                .collect())
        };
        let effects = if self.bad_rows_push.get() {
            vec![unfit_push()]
        } else {
            Vec::new()
        };
        async move { result.map(|value| Reply { effects, value }) }.boxed_local()
    }
}

pub(crate) fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings = SettingsStore::test(cx);
        cx.set_global(settings);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
        // The context-free part of the default keymap that surfaces rely on: Tab and Down both
        // map to `menu::SelectNext` there, which is why the keyboard module intercepts Tab.
        cx.bind_keys([
            KeyBinding::new("enter", menu::Confirm, None),
            KeyBinding::new("escape", menu::Cancel, None),
            KeyBinding::new("tab", menu::SelectNext, None),
            KeyBinding::new("shift-tab", menu::SelectPrevious, None),
            KeyBinding::new("down", menu::SelectNext, None),
            KeyBinding::new("up", menu::SelectPrevious, None),
            KeyBinding::new("home", menu::SelectFirst, None),
            KeyBinding::new("pageup", menu::SelectFirst, None),
            KeyBinding::new("end", menu::SelectLast, None),
            KeyBinding::new("pagedown", menu::SelectLast, None),
        ]);
        crate::keyboard::intercept_keys(cx);
    });
}

fn new_state(backend: &Rc<FakeBackend>, cx: &mut TestAppContext) -> Entity<SurfaceState> {
    let surface = ui_ext::v0_2::Surface {
        id: "counter".into(),
        title: "Counter".into(),
        kind: ui_ext::v0_2::SurfaceKind::Panel(ui_ext::v0_2::PanelDescriptor {
            position: ui_ext::v0_2::DockPosition::Right,
            icon: ui_ext::v0_2::IconRef::Upstream("hash".into()),
            default_size: 320,
        }),
    };
    let link: Rc<dyn crate::surface::SurfaceLink> = Rc::new(FakeLink(backend.clone()));
    let backend: Rc<dyn UiBackend> = backend.clone();
    let state = cx.new(|cx| SurfaceState::new(backend, surface, Some(link), cx));
    cx.run_until_parked();
    state
}

/// Plan 1.3: a dead instance is no longer final: the failed surface can ask the registry for a
/// restart (the "Restart extension" button), and a failed reload is a footer error with Retry,
/// cleared when the surface mounts again.
#[gpui::test]
async fn test_dead_instance_can_restart(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(FATAL), cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert!(matches!(
            state.phase(),
            crate::Phase::Failed {
                retryable: false,
                ..
            }
        ));
        assert!(state.can_restart());
    });
    state.update(cx, |state, cx| state.restart_extension(cx));
    cx.run_until_parked();
    assert_eq!(backend.restarts.get(), 1);

    // The registry answers a restart with a new instance (here the same fake).
    let surface = state.read_with(cx, |state, _| state.surface().clone());
    state.update(cx, |state, cx| {
        state.replace_backend(backend.clone(), surface, cx)
    });
    cx.run_until_parked();
    state.update(cx, |state, cx| {
        state.show_reload_error("Reload failed: broken".into(), cx)
    });
    state.read_with(cx, |state, _| {
        assert!(state.reload_failed());
        assert!(state.tree().is_some(), "the old instance keeps the surface");
    });
    state.update(cx, |state, cx| state.retry(cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert!(!state.reload_failed());
        assert_eq!(state.last_error(), None);
    });
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
    // The host tree follows the typed text (integrator decision 6).
    assert_eq!(props_of(&state, INPUT, cx), input("ab"));

    // Pushes made during a call are applied before the reply.
    backend.take_calls();
    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(PUSHER), cx));
    cx.run_until_parked();
    assert_eq!(backend.events(), [UiEvent::Click(PUSHER)]);
    assert_eq!(props_of(&state, LABEL, cx), text("replied"));
    assert_eq!(backend.routed.borrow().as_slice(), ["other"]);
}

#[gpui::test]
async fn test_pushes_before_a_failure_still_apply(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    backend.take_calls();

    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(PUSH_THEN_FAIL), cx)
    });
    cx.run_until_parked();

    // The guest counts its pushes as applied even though the call failed, so the host applies
    // them too, then shows the error; no remount is needed.
    assert_eq!(props_of(&state, LABEL, cx), text("pushed, then failed"));
    assert_eq!(
        backend.take_calls(),
        [Call::Event(UiEvent::Click(PUSH_THEN_FAIL))]
    );
    state.read_with(cx, |state, _| {
        let error = state.last_error().expect("error is shown");
        assert!(error.contains("the extension gave up"), "{error}");
    });
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

    // While focused, the user's text stands until the extension sets a value...
    vcx.simulate_input("mine");
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(editor.read_with(vcx, |editor, cx| editor.text(cx)), "mine");
    backend.take_calls();
    // ...and a set-props is written even though the field is focused.
    state.update(vcx, |state, cx| {
        state.dispatch(UiEvent::Click(SET_INPUT), cx)
    });
    vcx.run_until_parked();
    assert!(vcx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
    assert_eq!(
        editor.read_with(vcx, |editor, cx| editor.text(cx)),
        "from extension"
    );
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(
        backend.events(),
        [UiEvent::Click(SET_INPUT)],
        "the written value is not echoed back"
    );
}

#[gpui::test]
async fn test_extension_panel_renders_surface(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let info = Rc::new(crate::SurfaceInfo::new(
        "fake",
        &state.read_with(cx, |state, _| state.surface().clone()),
        100,
    ));
    let (panel, vcx) = cx.add_window_view(|window, cx| {
        crate::ExtensionPanel::<0>::new(info.clone(), state.clone(), window, cx)
    });
    vcx.run_until_parked();

    vcx.update(|window, cx| {
        use workspace::Panel as _;
        let panel = panel.read(cx);
        assert_eq!(panel.key().as_ref(), "fake/counter");
        assert_eq!(panel.icon(window, cx), Some(ui::IconName::Hash));
        assert_eq!(panel.icon_tooltip(window, cx), Some("Counter"));
        assert_eq!(
            panel.position(window, cx),
            workspace::dock::DockPosition::Right
        );
        assert_eq!(panel.default_size(window, cx), gpui::px(320.));
        assert_eq!(panel.activation_priority(), 100);
        let slot = info.slot.expect("a panel slot");
        assert_eq!(slot, 0, "the first panel surface of the process");
        assert_eq!(
            <crate::ExtensionPanel<0> as workspace::Panel>::persistent_name(),
            "fyx-ui:fake/counter"
        );
    });

    let bounds = vcx
        .debug_bounds("fyx-node-5")
        .expect("the surface is drawn inside the panel");
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(props_of(&state, LABEL, vcx), text("Count: 1"));

    // Triage-3, ASYNC 8: a panel that goes away (its workspace closed) stops showing the
    // surface, so its ticks wait.
    panel.update_in(vcx, |panel, window, cx| {
        use workspace::Panel as _;
        panel.set_active(true, window, cx)
    });
    assert!(state.read_with(vcx, |state, _| state.is_visible()));
    drop(panel);
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    assert!(!state.read_with(vcx, |state, _| state.is_visible()));
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

// Judge review w3-deep-1 (tasks/reviews/w3-deep-1.md), one test per finding still open at
// cd82d83856. Finding 1 is `test_pushes_before_a_failure_still_apply`.

fn mounts(backend: &FakeBackend) -> usize {
    backend
        .calls()
        .iter()
        .filter(|call| **call == Call::Mount)
        .count()
}

fn retryable(state: &Entity<SurfaceState>, cx: &mut TestAppContext) -> bool {
    state.read_with(cx, |state, _| {
        matches!(
            state.phase(),
            crate::Phase::Failed {
                retryable: true,
                ..
            }
        )
    })
}

fn failure(state: &Entity<SurfaceState>, cx: &mut TestAppContext) -> Option<String> {
    state.read_with(cx, |state, _| match state.phase() {
        crate::Phase::Failed { message, .. } => Some(message.to_string()),
        crate::Phase::Closed | crate::Phase::Mounting | crate::Phase::Ready(_) => None,
    })
}

/// Finding 2: typing still waiting for its pause reaches the extension before a later click.
#[gpui::test]
async fn test_pending_typing_is_sent_before_a_click(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    backend.take_calls();

    let editor = view.read_with(vcx, |view, _| view.inputs[&INPUT].editor.clone());
    vcx.update(|window, cx| editor.focus_handle(cx).focus(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("foo");
    let bounds = vcx.debug_bounds("fyx-node-5").expect("increment is drawn");
    vcx.simulate_click(bounds.center(), Modifiers::none());
    vcx.run_until_parked();

    assert_eq!(
        backend.events(),
        [
            UiEvent::InputChanged(INPUT, "foo".into()),
            UiEvent::Click(INCREMENT)
        ]
    );
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(backend.events().len(), 2, "the typing is not sent twice");
}

/// Finding 3: an own push that does not fit stops the own updates, not the routing of the others.
#[gpui::test]
async fn test_pushes_for_other_surfaces_survive_an_own_failure(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    backend.take_calls();

    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(BAD_OWN_THEN_OTHER), cx)
    });
    cx.run_until_parked();

    assert_eq!(backend.routed.borrow().as_slice(), ["other"]);
    assert_eq!(mounts(&backend), 1, "the bad own push remounts the surface");
    assert_eq!(
        props_of(&state, LABEL, cx),
        text("Count: 0"),
        "the reply was skipped"
    );
}

/// Finding 4: an extension that keeps sending unfit updates without user input is stopped.
#[gpui::test]
async fn test_remounts_without_input_are_bounded(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(5);
    backend.bad_rows_push.set(true);
    let state = new_state(&backend, cx);
    backend.take_calls();

    // Every layout asks for the visible rows again; every answer pushes an unfit patch.
    for _ in 0..10 {
        state.update(cx, |state, cx| state.request_rows(LIST, 0..5, cx));
        cx.run_until_parked();
    }
    assert_eq!(mounts(&backend), 3);
    let message = failure(&state, cx).expect("the surface gave up");
    assert!(message.contains("keeps sending updates"), "{message}");
    assert!(
        retryable(&state, cx),
        "the instance lives, so Retry is offered"
    );

    // A retry by the user starts over.
    backend.bad_rows_push.set(false);
    state.update(cx, |state, cx| state.retry(cx));
    cx.run_until_parked();
    assert_eq!(failure(&state, cx), None);
    assert_eq!(props_of(&state, LABEL, cx), text("Count: 5"));
}

/// integration-5 M1: good pushes between the bad updates do not reset the bound; only a user
/// event's applied reply does.
#[gpui::test]
async fn test_pushes_do_not_reset_the_remount_bound(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(5);
    backend.bad_rows_push.set(true);
    let state = new_state(&backend, cx);
    backend.take_calls();
    for round in 0..10 {
        state.update(cx, |state, cx| state.request_rows(LIST, 0..5, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            state.apply_pushed_update(
                Update::Patches(vec![Patch::SetProps(
                    LABEL,
                    text(&format!("pushed {round}")),
                )]),
                cx,
            )
        });
        cx.run_until_parked();
    }
    let message = failure(&state, cx).expect("the surface gave up");
    assert!(message.contains("keeps sending updates"), "{message}");
}

/// Finding 5: an error without a `ReplyError` (trap, budget, disabled) stops every surface of the
/// extension; nothing is dispatched any more until a retry.
#[gpui::test]
async fn test_a_dead_instance_stops_the_extension(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    backend.take_calls();

    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(FATAL), cx));
    cx.run_until_parked();
    let message = failure(&state, cx).expect("the surface stopped");
    // Triage-3 #7: plain words and the next step; the raw error is the detail.
    assert_eq!(
        message,
        "fake took too long to answer and was stopped. Restart the extension to try again."
    );
    assert_eq!(
        state
            .read_with(cx, |state, _| state.failure_detail().cloned())
            .as_deref(),
        Some("wasm trap: interrupt")
    );
    assert_eq!(backend.failed_from.borrow().as_slice(), ["counter"]);
    assert!(
        !retryable(&state, cx),
        "a dead instance refuses every call, so no Retry (integration-3 finding 9)"
    );

    backend.take_calls();
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx);
        state.request_rows(LIST, 0..1, cx);
    });
    cx.run_until_parked();
    assert_eq!(backend.calls(), [], "a stopped surface makes no calls");
    // Triage-3, ASYNC 7: the surface let the instance go; only the test's handles remain.
    assert_eq!(Rc::strong_count(&backend), 2, "the test and its link");
    state.update(cx, |state, cx| state.retry(cx));
    cx.run_until_parked();
    assert_eq!(backend.calls(), [], "even a forced retry does not reach it");
    assert_eq!(
        failure(&state, cx).as_deref(),
        Some("fake crashed. Restart the extension to try again.")
    );

    // A restart hands over a new instance.
    let surface = state.read_with(cx, |state, _| state.surface().clone());
    state.update(cx, |state, cx| {
        state.replace_backend(backend.clone(), surface, cx)
    });
    cx.run_until_parked();
    assert_eq!(backend.take_calls(), [Call::Mount]);
    assert_eq!(failure(&state, cx), None);
}

/// Triage-3, ASYNC 12: typing still in its pause is sent when the view goes away.
#[gpui::test]
async fn test_pending_typing_is_sent_when_the_view_closes(cx: &mut TestAppContext) {
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
    vcx.simulate_input("hi");
    vcx.run_until_parked();
    assert_eq!(backend.events(), []);

    drop(view);
    drop(editor);
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    assert_eq!(
        backend.events(),
        [UiEvent::InputChanged(INPUT, "hi".into())]
    );
}

/// Finding 6: short answers for two ranges (measuring and visible, or two windows) do not make
/// the ranges refetch each other forever.
#[gpui::test]
async fn test_short_answers_for_two_ranges_are_fetched_once(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(50);
    backend.short_rows.set(true);
    let state = new_state(&backend, cx);
    backend.take_calls();

    for _ in 0..3 {
        for range in [0..9, 20..29] {
            state.update(cx, |state, cx| state.request_rows(LIST, range, cx));
            cx.run_until_parked();
        }
    }
    assert_eq!(
        backend.take_calls(),
        [
            Call::Rows {
                list: LIST,
                start: 0,
                count: 9
            },
            Call::Rows {
                list: LIST,
                start: 20,
                count: 9
            },
        ]
    );
    // The rows it left out are empty, not stale.
    assert!(row_text(&state, LIST, 0, cx).is_some());
    assert!(row_text(&state, LIST, 1, cx).is_none());
}

/// Finding 7: a queued event is checked again when its turn comes.
#[gpui::test]
async fn test_queued_events_are_checked_before_sending(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(5);
    let state = new_state(&backend, cx);
    state.update(cx, |state, cx| state.request_rows(LIST, 0..5, cx));
    cx.run_until_parked();
    backend.take_calls();

    // The increment's reply changes the list, so row 2 is no longer the row the user clicked.
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx);
        state.dispatch(UiEvent::RowClick(LIST, 2), cx);
    });
    cx.run_until_parked();
    assert_eq!(backend.events(), [UiEvent::Click(INCREMENT)]);

    // A double click on a button whose reply removes it sends one click.
    backend.take_calls();
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(REMOVER), cx);
        state.dispatch(UiEvent::Click(REMOVER), cx);
    });
    cx.run_until_parked();
    assert_eq!(backend.events(), [UiEvent::Click(REMOVER)]);

    // The increment changed the list and nothing has fetched its rows again, so row 1 on screen
    // is stale: a click on it is not sent (integration-3 finding 9)...
    backend.take_calls();
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::RowClick(LIST, 1), cx)
    });
    cx.run_until_parked();
    assert_eq!(backend.events(), []);
    // ...while a click on a fresh row goes through.
    state.update(cx, |state, cx| state.request_rows(LIST, 0..6, cx));
    cx.run_until_parked();
    backend.take_calls();
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::RowClick(LIST, 1), cx)
    });
    cx.run_until_parked();
    assert_eq!(backend.events(), [UiEvent::RowClick(LIST, 1)]);
}

/// Finding 8: a remount (which sets every input) does not overwrite typing not yet sent; the
/// typing is sent at once instead.
#[gpui::test]
async fn test_a_remount_keeps_unsent_typing(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    backend.take_calls();

    let editor = view.read_with(vcx, |view, _| view.inputs[&INPUT].editor.clone());
    vcx.update(|window, cx| editor.focus_handle(cx).focus(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("abc");
    state.update(vcx, |state, cx| state.mount(cx));
    vcx.run_until_parked();

    assert_eq!(editor.read_with(vcx, |editor, cx| editor.text(cx)), "abc");
    assert_eq!(
        backend.take_calls(),
        [
            Call::Mount,
            Call::Event(UiEvent::InputChanged(INPUT, "abc".into()))
        ]
    );
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(backend.calls(), [], "nothing is sent twice");
}

// Keyboard support: every key path makes exactly the extension calls a mouse would.

fn focused(view: &Entity<SurfaceView>, cx: &mut VisualTestContext) -> Option<NodeId> {
    cx.update(|window, cx| view.read(cx).focused_node(window, cx))
}

fn focus_surface(view: &Entity<SurfaceView>, cx: &mut VisualTestContext) {
    let handle = view.read_with(cx, |view, _| view.focus_handle.clone());
    cx.update(|window, cx| handle.focus(window, cx));
    cx.run_until_parked();
}

fn key(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

/// Triage-3 #5: the caret of a focused input does not blink, so the surface is not repainted
/// while nothing happens.
#[gpui::test]
async fn test_focused_input_does_not_repaint_by_itself(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    let editor = view.read_with(vcx, |view, _| {
        view.inputs
            .get(&INPUT)
            .map(|input| input.editor.clone())
            .expect("input has an editor")
    });
    vcx.update(|window, cx| {
        window.activate_window();
        editor.focus_handle(cx).focus(window, cx)
    });
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.simulate_input("x");
    // Past what focusing and typing schedule (the blink pause).
    vcx.executor().advance_clock(Duration::from_secs(2));
    vcx.run_until_parked();
    let notified = Rc::new(Cell::new(0));
    let _observe = vcx.update(|_, cx| {
        let notified = notified.clone();
        cx.observe(&editor, move |_, _| notified.set(notified.get() + 1))
    });
    let mut shown = Vec::new();
    for _ in 0..8 {
        vcx.executor().advance_clock(Duration::from_millis(300));
        vcx.run_until_parked();
        shown.push(vcx.update(|window, cx| {
            editor.update(cx, |editor, cx| editor.show_local_cursors(window, cx))
        }));
    }
    assert_eq!(shown, [true; 8], "the caret stays");
    assert_eq!(notified.get(), 0, "no blink repaints");
    assert!(vcx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
}

/// Triage-3, UX F6 and F21: the host's own buttons are in the Tab order and take Enter and
/// Space; the error footer has a Dismiss.
#[gpui::test]
async fn test_host_controls_take_the_keyboard(cx: &mut TestAppContext) {
    use crate::view::HostControl;
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    focus_surface(&view, vcx);
    let host_focused = |vcx: &mut VisualTestContext| {
        vcx.update(|window, cx| view.read(cx).focused_host_control(window))
    };

    // A footer error: after the surface's nodes come Retry and Dismiss.
    state.update(vcx, |state, cx| {
        state.show_reload_error("Reload failed: broken".into(), cx)
    });
    vcx.run_until_parked();
    key("shift-tab", vcx);
    assert_eq!(host_focused(vcx), Some(HostControl::DismissError));
    key("shift-tab", vcx);
    assert_eq!(host_focused(vcx), Some(HostControl::ReloadRetry));
    key("shift-tab", vcx);
    assert_eq!(focused(&view, vcx), Some(LIST), "back into the surface");
    key("tab", vcx);
    key("tab", vcx);
    assert_eq!(host_focused(vcx), Some(HostControl::DismissError));
    key("space", vcx);
    assert_eq!(
        state.read_with(vcx, |state, _| state.last_error().cloned()),
        None
    );

    // A failed surface: Retry, reached by Tab, retries on Enter.
    state.update(vcx, |state, cx| state.fail("broken".into(), true, cx));
    vcx.run_until_parked();
    backend.take_calls();
    key("tab", vcx);
    assert_eq!(host_focused(vcx), Some(HostControl::Retry));
    key("enter", vcx);
    assert_eq!(backend.take_calls(), [Call::Mount]);
}

#[gpui::test]
async fn test_tab_moves_focus_in_tree_order(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    backend.take_calls();
    focus_surface(&view, vcx);

    // Buttons, then the input, then the list, as the tree lists them.
    let order = [
        INCREMENT,
        BROKEN,
        PUSHER,
        SET_INPUT,
        PUSH_THEN_FAIL,
        BAD_OWN_THEN_OTHER,
        FATAL,
        REMOVER,
        INPUT,
        LIST,
    ];
    for expected in order {
        key("tab", vcx);
        assert_eq!(focused(&view, vcx), Some(expected));
    }
    // Around the end, and back.
    key("tab", vcx);
    assert_eq!(focused(&view, vcx), Some(INCREMENT));
    key("shift-tab", vcx);
    assert_eq!(focused(&view, vcx), Some(LIST));
    key("shift-tab", vcx);
    assert_eq!(
        focused(&view, vcx),
        Some(INPUT),
        "Shift-Tab leaves the list"
    );
    key("shift-tab", vcx);
    assert_eq!(
        focused(&view, vcx),
        Some(REMOVER),
        "Tab keys leave an input too"
    );
    assert_eq!(backend.calls(), [], "moving focus calls nothing");
}

#[gpui::test]
async fn test_enter_and_space_click_the_focused_button(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    backend.take_calls();
    focus_surface(&view, vcx);
    key("tab", vcx);
    assert_eq!(focused(&view, vcx), Some(INCREMENT));

    key("enter", vcx);
    assert_eq!(backend.events(), [UiEvent::Click(INCREMENT)]);
    backend.take_calls();
    key("space", vcx);
    assert_eq!(backend.events(), [UiEvent::Click(INCREMENT)]);
    assert_eq!(props_of(&state, LABEL, vcx), text("Count: 2"));
    // The patches did not remove the button, so it keeps focus.
    assert_eq!(focused(&view, vcx), Some(INCREMENT));
}

#[gpui::test]
async fn test_list_selection_keys_and_enter(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(300);
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    let list_focus = view.read_with(vcx, |view, _| view.list_focus[&LIST].clone());
    vcx.update(|window, cx| list_focus.focus(window, cx));
    vcx.run_until_parked();
    assert_eq!(focused(&view, vcx), Some(LIST));
    backend.take_calls();

    let selected = |vcx: &mut VisualTestContext| {
        view.read_with(vcx, |view, _| view.selection.get(&LIST).copied())
    };
    key("down", vcx);
    assert_eq!(selected(vcx), Some(0));
    key("down", vcx);
    assert_eq!(selected(vcx), Some(1));
    key("up", vcx);
    key("up", vcx);
    assert_eq!(selected(vcx), Some(0), "Up stops at the first row");
    key("end", vcx);
    assert_eq!(selected(vcx), Some(299));
    key("home", vcx);
    assert_eq!(selected(vcx), Some(0));
    // Page keys move by a page, not to the ends (upstream maps them to SelectFirst/Last).
    key("pagedown", vcx);
    let page = selected(vcx).expect("selected");
    assert!(page > 1 && page < 299, "a page is {page} rows");
    key("pageup", vcx);
    assert_eq!(selected(vcx), Some(0));
    key("down", vcx);
    key("down", vcx);
    assert_eq!(focused(&view, vcx), Some(LIST), "Down stays in the list");
    assert!(
        !backend
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Event(_))),
        "moving the selection sends no event"
    );

    key("enter", vcx);
    assert_eq!(backend.events(), [UiEvent::RowClick(LIST, 2)]);
}

#[gpui::test]
async fn test_focus_moves_on_when_its_node_is_removed(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    let remover = view.read_with(vcx, |view, _| view.button_focus[&REMOVER].clone());
    vcx.update(|window, cx| remover.focus(window, cx));
    vcx.run_until_parked();
    backend.take_calls();

    key("enter", vcx);
    assert_eq!(backend.events(), [UiEvent::Click(REMOVER)]);
    // The reply removed the button; the next node in the Tab order takes focus.
    assert_eq!(focused(&view, vcx), Some(INPUT));
}

#[gpui::test]
async fn test_escape_returns_to_the_editor(cx: &mut TestAppContext) {
    init_test(cx);
    let returned = Rc::new(Cell::new(0));
    cx.update(|cx| {
        let returned = returned.clone();
        cx.on_action(move |_: &workspace::FocusCenterPane, _| returned.set(returned.get() + 1));
    });
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    focus_surface(&view, vcx);
    key("tab", vcx);
    backend.take_calls();

    key("escape", vcx);
    assert_eq!(returned.get(), 1);
    assert_eq!(backend.calls(), [], "Escape calls nothing");
}

// Judge review integration-3 (tasks/reviews/integration-3.md), W3 findings 8, 9 and 10.

/// Finding 8: an answer to a fetch made before a remount cannot pose as the new list's rows.
#[gpui::test]
async fn test_a_stale_row_answer_after_a_remount_is_dropped(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(5);
    let state = new_state(&backend, cx);
    state.update(cx, |state, cx| state.request_rows(LIST, 0..5, cx));
    cx.run_until_parked();
    let old_generation = state.read_with(cx, |state, _| state.current_row_generation(LIST));

    // A remount (the list keeps its fixed id), then the new list asks for rows...
    state.update(cx, |state, cx| state.mount(cx));
    cx.run_until_parked();
    backend.take_calls();
    state.update(cx, |state, cx| state.request_rows(LIST, 0..5, cx));
    // ...and the answer to the fetch from before the remount arrives first.
    state.update(cx, |state, cx| {
        let rows = (0..5)
            .map(|index| {
                let row = vec![node(ROW_ROOT, None, text("stale"))];
                crate::backend::Row {
                    index,
                    indent: 0,
                    tree: Tree::from_row(row, TreeLimits::default()).expect("valid row"),
                }
            })
            .collect();
        let reply = Reply {
            effects: Vec::new(),
            value: rows,
        };
        state.receive_rows(LIST, old_generation, 0..5, Ok(reply), cx);
    });
    cx.run_until_parked();
    assert_eq!(row_text(&state, LIST, 0, cx), Some(text("Row 1 of 5")));

    // Layout keeps asking; nothing is missing, so nothing is fetched again.
    for _ in 0..5 {
        state.update(cx, |state, cx| state.request_rows(LIST, 0..5, cx));
        cx.run_until_parked();
    }
    assert_eq!(
        backend.take_calls(),
        [Call::Rows {
            list: LIST,
            start: 0,
            count: 5
        }]
    );
}

/// w3-deep-1 finding 6, the rest of it: the measuring request and the visible range of one
/// layout make one `list-rows` call.
#[gpui::test]
async fn test_one_layout_makes_one_row_call(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    backend.count.set(100);
    let state = new_state(&backend, cx);
    backend.take_calls();

    state.update(cx, |state, cx| {
        state.request_rows(LIST, 40..49, cx);
        state.request_rows(LIST, 32..60, cx);
        state.request_rows(LIST, 40..49, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.take_calls(),
        [Call::Rows {
            list: LIST,
            start: 32,
            count: 28
        }]
    );
}

/// Finding 9: a surface fed only by pushes does not fail after a few recovered desyncs over a
/// session: a desync a minute after the last remount starts the count again (pushes themselves
/// no longer reset it, integration-5 M1).
#[gpui::test]
async fn test_occasional_desyncs_recover(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    backend.take_calls();

    for round in 0..6 {
        state.update(cx, |state, cx| {
            state.apply_pushed_update(
                match unfit_push() {
                    HostEffect::Push { update, .. } => update,
                    _ => unreachable!(),
                },
                cx,
            )
        });
        cx.run_until_parked();
        state.update(cx, |state, cx| {
            state.apply_pushed_update(
                Update::Patches(vec![Patch::SetProps(LABEL, text(format!("round {round}")))]),
                cx,
            )
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_secs(61));
        cx.run_until_parked();
    }
    assert_eq!(failure(&state, cx), None);
    assert_eq!(mounts(&backend), 6, "every desync remounted and recovered");
}

/// Finding 9: typing pending in one view of an extension is sent before a click in another
/// view of the same extension (another surface, another window).
#[gpui::test]
async fn test_typing_in_one_view_precedes_a_click_in_another(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let typing_state = new_state(&backend, cx);
    let clicking_state = new_state(&backend, cx);
    let (typing_view, typing_cx) = open_view(&typing_state, cx);
    let editor = typing_view.read_with(typing_cx, |view, _| view.inputs[&INPUT].editor.clone());
    typing_cx.update(|window, cx| editor.focus_handle(cx).focus(window, cx));
    typing_cx.run_until_parked();
    typing_cx.simulate_input("abc");
    let (clicking_view, clicking_cx) = open_view(&clicking_state, cx);
    backend.take_calls();

    clicking_view.update(clicking_cx, |view, cx| {
        view.dispatch(UiEvent::Click(INCREMENT), cx)
    });
    // Before either queue runs, the typing is already queued: it goes out first.
    assert_eq!(
        typing_state.read_with(clicking_cx, |state, _| state.queued_events()),
        [UiEvent::InputChanged(INPUT, "abc".into())]
    );
    clicking_cx.run_until_parked();
    assert_eq!(
        backend.events(),
        [
            UiEvent::InputChanged(INPUT, "abc".into()),
            UiEvent::Click(INCREMENT)
        ]
    );
}

/// Finding 10: a remount that gives the input a new id keeps the user's unsent typing and
/// focus, and sends the typing under the new id.
#[gpui::test]
async fn test_typing_survives_a_remount_that_renumbers_the_input(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    let editor = view.read_with(vcx, |view, _| view.inputs[&INPUT].editor.clone());
    vcx.update(|window, cx| editor.focus_handle(cx).focus(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("abc");
    backend.take_calls();

    const RENUMBERED: NodeId = 31;
    backend.renumbered_input.set(Some(RENUMBERED));
    state.update(vcx, |state, cx| state.mount(cx));
    vcx.run_until_parked();

    let editor = view.read_with(vcx, |view, _| view.inputs[&RENUMBERED].editor.clone());
    assert_eq!(editor.read_with(vcx, |editor, cx| editor.text(cx)), "abc");
    assert_eq!(focused(&view, vcx), Some(RENUMBERED));
    assert_eq!(
        backend.take_calls(),
        [
            Call::Mount,
            Call::Event(UiEvent::InputChanged(RENUMBERED, "abc".into()))
        ]
    );
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert_eq!(backend.calls(), [], "nothing is sent twice");
}

/// Integration-4 finding 8: when the extension itself swaps the input for a new, empty one (a
/// form cleared after a submit), the user's old text is not written into it or sent again.
#[gpui::test]
async fn test_own_rebuild_does_not_carry_typing(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (view, vcx) = open_view(&state, cx);
    let editor = view.read_with(vcx, |view, _| view.inputs[&INPUT].editor.clone());
    vcx.update(|window, cx| editor.focus_handle(cx).focus(window, cx));
    vcx.run_until_parked();
    vcx.simulate_input("hello");
    backend.take_calls();

    const CLEARED: NodeId = 41;
    let applied = state.update(vcx, |state, cx| {
        state.apply_update(
            Update::Patches(vec![
                Patch::Remove(INPUT),
                Patch::Insert(ui_ext::v0_2::InsertOp {
                    node: node(CLEARED, Some(1), input("")),
                    before: None,
                }),
            ]),
            cx,
        )
    });
    assert!(applied);
    vcx.run_until_parked();

    let editor = view.read_with(vcx, |view, _| view.inputs[&CLEARED].editor.clone());
    assert_eq!(editor.read_with(vcx, |editor, cx| editor.text(cx)), "");
    vcx.executor().advance_clock(TYPING_DEBOUNCE);
    vcx.run_until_parked();
    assert!(
        !backend
            .take_calls()
            .iter()
            .any(|call| matches!(call, Call::Event(UiEvent::InputChanged(CLEARED, _)))),
        "the old text is not sent for the new input"
    );
}

// (c) Names, titles and status item order.

fn panel_surface(id: &str, title: &str) -> ui_ext::v0_2::Surface {
    ui_ext::v0_2::Surface {
        id: id.into(),
        title: title.into(),
        kind: ui_ext::v0_2::SurfaceKind::Panel(ui_ext::v0_2::PanelDescriptor {
            position: ui_ext::v0_2::DockPosition::Left,
            icon: ui_ext::v0_2::IconRef::Upstream("hash".into()),
            default_size: 300,
        }),
    }
}

/// Every panel surface gets a type slot of its own, so its `persistent_name` and `panel_key`
/// (sizes, the restored open panel) are its own: `fyx-ui:<extension>/<surface>`. The same surface
/// keeps its slot for the process (a reload does not take another), and titles are interned
/// rather than leaked per load.
#[test]
fn test_panels_get_names_of_their_own() {
    use workspace::Panel as _;
    let first = crate::SurfaceInfo::new("names-test", &panel_surface("a", "Alpha"), 100);
    let second = crate::SurfaceInfo::new("names-test", &panel_surface("b", "Beta"), 101);
    let status = crate::SurfaceInfo::new(
        "names-test",
        &ui_ext::v0_2::Surface {
            id: "s".into(),
            title: "Status".into(),
            kind: ui_ext::v0_2::SurfaceKind::StatusItem(ui_ext::v0_2::StatusItemDescriptor {
                side: ui_ext::v0_2::StatusBarSide::Right,
                priority: 0,
            }),
        },
        102,
    );
    let (first_slot, second_slot) = (first.slot.expect("a slot"), second.slot.expect("a slot"));
    assert_ne!(first_slot, second_slot);
    assert_eq!(status.slot, None, "status items are not dock panels");
    assert_eq!(
        crate::panel::slot_name(first_slot),
        Some("fyx-ui:names-test/a")
    );
    assert_eq!(
        crate::panel::slot_name(second_slot),
        Some("fyx-ui:names-test/b")
    );
    assert_eq!(
        <crate::ExtensionPanel<{ crate::SHARED_SLOT }>>::persistent_name(),
        "Extension Panel",
        "panels past the slots share one name"
    );

    let reloaded = crate::SurfaceInfo::new("names-test", &panel_surface("a", "Alpha"), 103);
    assert_eq!(reloaded.slot, Some(first_slot));
    assert!(
        std::ptr::eq(reloaded.title, first.title),
        "the title is interned"
    );
}

#[gpui::test]
async fn test_status_items_follow_their_priority(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (_, vcx) = open_view(&state, cx);
    for (side, expected) in [
        (ui_ext::v0_2::StatusBarSide::Left, [10, 5, 5, 1]),
        (ui_ext::v0_2::StatusBarSide::Right, [1, 5, 5, 10]),
    ] {
        let group = vcx.update(|_, cx| cx.new(|_| crate::ExtensionStatusGroup::new(side)));
        for priority in [5, 10, 1, 5] {
            let item = vcx.update(|window, cx| {
                cx.new(|cx| crate::ExtensionStatusItem::new(state.clone(), window, cx))
            });
            group.update(vcx, |group, cx| group.add(priority, item, cx));
        }
        assert_eq!(
            group.read_with(vcx, |group, _| group.priorities()),
            expected,
            "higher priority sits closer to the {side:?} edge"
        );
    }
}

/// Triage-3, UX F22: `fyx.extensions.hidden_status_items` hides a status item; the group
/// re-renders when it changes.
#[gpui::test]
async fn test_status_items_can_be_hidden(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let (_, vcx) = open_view(&state, cx);
    let group = vcx.update(|_, cx| {
        cx.new(|_| crate::ExtensionStatusGroup::new(ui_ext::v0_2::StatusBarSide::Left))
    });
    let item = vcx.update(|window, cx| {
        cx.new(|cx| crate::ExtensionStatusItem::new(state.clone(), window, cx))
    });
    group.update(vcx, |group, cx| group.add(0, item.clone(), cx));
    let group_notified = Rc::new(Cell::new(0));
    let _observe = vcx.update(|_, cx| {
        let group_notified = group_notified.clone();
        cx.observe(&group, move |_, _| {
            group_notified.set(group_notified.get() + 1)
        })
    });
    assert!(!item.read_with(vcx, |item, cx| item.is_hidden(cx)));

    let set_hidden = |hidden: Vec<String>, vcx: &mut VisualTestContext| {
        vcx.update(|_, cx| {
            gpui::BorrowAppContext::update_global::<SettingsStore, _>(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings
                        .fyx
                        .get_or_insert_default()
                        .extensions
                        .get_or_insert_default()
                        .hidden_status_items = Some(hidden)
                })
            })
        });
        vcx.run_until_parked();
    };
    set_hidden(vec!["fake/counter".into()], vcx);
    assert!(item.read_with(vcx, |item, cx| item.is_hidden(cx)));
    assert!(group_notified.get() > 0, "the group lays out again");
    set_hidden(Vec::new(), vcx);
    assert!(!item.read_with(vcx, |item, cx| item.is_hidden(cx)));
}

/// The extensions page shows a loaded UI extension's capabilities, from the surfaces it declared.
#[gpui::test]
async fn test_capabilities_for_the_extensions_page(cx: &mut TestAppContext) {
    init_test(cx);
    let backend: Rc<dyn UiBackend> = Rc::new(FakeBackend::default());
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let panel = panel_surface("main", "Main");
    let status = ui_ext::v0_2::Surface {
        id: "status".into(),
        title: "Status".into(),
        kind: ui_ext::v0_2::SurfaceKind::StatusItem(ui_ext::v0_2::StatusItemDescriptor {
            side: ui_ext::v0_2::StatusBarSide::Right,
            priority: 0,
        }),
    };
    registry.update(cx, |registry, cx| {
        registry.register_backend(backend.clone(), std::slice::from_ref(&status), cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            registry.read(cx).capabilities("fake", cx),
            ["ui:status-bar"]
        );
    });
    // A reload that adds a panel shows both.
    registry.update(cx, |registry, cx| {
        registry.register_backend(backend.clone(), &[panel, status], cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let registry = registry.read(cx);
        assert_eq!(
            registry.capabilities("fake", cx),
            ["ui:panel", "ui:status-bar"]
        );
        assert!(
            registry.capabilities("not-a-ui-extension", cx).is_empty(),
            "a regular Zed extension gets no chips"
        );
    });
}

/// Sandbox: an extension id belongs to the source that first loaded it. The same source may load
/// it again (a reinstall); another source declaring the same id is refused until it is released
/// (uninstalled). Lookups use the structured `(extension, surface)` key.
#[gpui::test]
async fn test_extension_ids_belong_to_their_source(cx: &mut TestAppContext) {
    init_test(cx);
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let id: std::sync::Arc<str> = "fake".into();
    let first = crate::ExtensionSource::Directory("/ext/fake".into());
    let impostor = crate::ExtensionSource::Directory("/elsewhere/fake".into());
    registry.update(cx, |registry, _| {
        registry
            .claim_extension_id(&id, first.clone())
            .expect("first claim");
        registry
            .claim_extension_id(&id, first.clone())
            .expect("a reinstall from the same source");
        let error = registry
            .claim_extension_id(&id, impostor.clone())
            .expect_err("another source");
        assert!(format!("{error:#}").contains("already used"), "{error:#}");
    });

    let backend: Rc<dyn UiBackend> = Rc::new(FakeBackend::default());
    registry.update(cx, |registry, cx| {
        registry.register_backend(backend, &[panel_surface("main", "Main")], cx)
    });
    cx.update(|cx| {
        let registry = registry.read(cx);
        assert!(registry.surface_of("fake", "main").is_some());
        assert!(registry.surface_of("fak", "e/main").is_none());
        assert_eq!(registry.surfaces_of("fake").count(), 1);
    });
}

/// Plan 2.3, design E: `add-surface` adds and mounts a surface at runtime, `remove-surface` takes
/// it away, and `show-surface` / `hide-surface` of a panel reach the workspaces as events.
#[gpui::test]
async fn test_runtime_surfaces(cx: &mut TestAppContext) {
    use crate::backend::HostEffect;
    use crate::registry::RegistryEvent;
    init_test(cx);
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let backend = Rc::new(FakeBackend::default());
    registry.update(cx, |registry, cx| {
        registry.register_backend(backend.clone(), &[panel_surface("main", "Main")], cx)
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&registry, move |_, event: &RegistryEvent, _| {
            events.borrow_mut().push(event.clone())
        })
        .detach();
    });

    registry.update(cx, |registry, cx| {
        registry.apply_effect(
            "fake",
            HostEffect::AddSurface(panel_surface("added", "Added")),
            cx,
        )
    });
    cx.run_until_parked();
    let added = registry.read_with(cx, |registry, _| {
        registry
            .surface_of("fake", "added")
            .map(|s| s.state.clone())
    });
    let added = added.expect("added at runtime");
    assert!(
        added.read_with(cx, |state, _| state.tree().is_some()),
        "and mounted"
    );

    registry.update(cx, |registry, cx| {
        registry.apply_effect(
            "fake",
            HostEffect::ShowSurface {
                surface_id: "added".into(),
            },
            cx,
        );
        registry.apply_effect(
            "fake",
            HostEffect::HideSurface {
                surface_id: "added".into(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        [
            RegistryEvent::RevealPanel("fake/added".into()),
            RegistryEvent::HidePanel("fake/added".into()),
        ]
    );

    registry.update(cx, |registry, cx| {
        registry.apply_effect(
            "fake",
            HostEffect::RemoveSurface {
                surface_id: "added".into(),
            },
            cx,
        )
    });
    registry.read_with(cx, |registry, _| {
        assert!(registry.surface_of("fake", "added").is_none());
        assert!(registry.surface_of("fake", "main").is_some());
    });
}

/// integration-5 M4: a modal the modal layer did not take is closed again and the extension told,
/// so the next `show-surface` mounts it afresh instead of being a no-op.
#[gpui::test]
async fn test_modal_not_shown_is_closed_again(cx: &mut TestAppContext) {
    use ui_ext::v0_2::{ModalDescriptor, ModalWidth, SurfaceKind};
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let modal = ui_ext::v0_2::Surface {
        id: "pick".into(),
        title: "Pick".into(),
        kind: SurfaceKind::Modal(ModalDescriptor {
            width: ModalWidth::Medium,
        }),
    };
    let dyn_backend: Rc<dyn UiBackend> = backend.clone();
    registry.update(cx, |registry, cx| {
        registry.register_backend(dyn_backend, std::slice::from_ref(&modal), cx)
    });
    cx.run_until_parked();
    let state = registry.read_with(cx, |registry, _| {
        registry
            .surface("fake/pick")
            .expect("surface")
            .state
            .clone()
    });
    state.update(cx, |state, cx| state.show(cx));
    cx.run_until_parked();
    assert!(state.read_with(cx, |state, _| state.is_open()));
    backend.take_calls();

    registry.update(cx, |registry, cx| registry.modal_not_shown("fake/pick", cx));
    cx.run_until_parked();
    assert!(state.read_with(cx, |state, _| !state.is_open()));
    assert_eq!(*backend.modals_closed.borrow(), ["pick"]);
    assert!(
        backend.events().is_empty(),
        "no dismissed for a modal never shown"
    );

    state.update(cx, |state, cx| state.show(cx));
    cx.run_until_parked();
    assert_eq!(backend.take_calls(), [Call::Mount], "shown again, afresh");
}

/// integration-5 H1: a shown modal ticks, and stops when it goes.
#[gpui::test]
async fn test_modal_ticks_while_shown(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let ticks = |backend: &FakeBackend| {
        backend
            .calls()
            .iter()
            .filter(|call| **call == Call::Tick)
            .count()
    };
    let (modal, vcx) = cx.add_window_view(|window, cx| {
        crate::modal::ExtensionModal::new(
            "fake/modal".into(),
            state.clone(),
            gpui::WeakEntity::new_invalid(),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    state.update(vcx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    vcx.executor().advance_clock(Duration::from_millis(1050));
    vcx.run_until_parked();
    assert_eq!(ticks(&backend), 1, "the shown modal ticked");

    vcx.update(|window, _| window.remove_window());
    drop(modal);
    vcx.run_until_parked();
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    cx.executor().advance_clock(Duration::from_millis(5000));
    cx.run_until_parked();
    assert_eq!(ticks(&backend), 1, "a closed modal is not ticked");
}

/// Plan 2.3, design B: a requested tick fires once after its delay (at least 1 s while no window
/// is active, as in tests), replaces an earlier one, repeats when the tick asks again, waits while
/// the panel is hidden, stops after 3 errors in a row until a user event, and goes when cancelled.
#[gpui::test]
async fn test_ticks(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let state = new_state(&backend, cx);
    let panel = cx.update(|cx| cx.new(|_| ()).entity_id());
    let ticks = |backend: &FakeBackend| {
        backend
            .calls()
            .iter()
            .filter(|call| **call == Call::Tick)
            .count()
    };
    let advance = |cx: &mut TestAppContext, ms: u64| {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
    };

    // Hidden: nothing, however long; shown: the due tick goes out at once.
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    advance(cx, 5000);
    assert_eq!(ticks(&backend), 0, "a hidden panel is not ticked");
    state.update(cx, |state, cx| state.set_visible(panel, true, cx));
    advance(cx, 0);
    assert_eq!(ticks(&backend), 1);
    assert_eq!(props_of(&state, LABEL, cx), text("Tick 1"));

    // Visible, app in the background: not before 1 s.
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    advance(cx, 900);
    assert_eq!(ticks(&backend), 1);
    advance(cx, 150);
    assert_eq!(ticks(&backend), 2);

    // A tick that asks again keeps going; cancelling stops it.
    backend.tick_again.set(Some(Duration::from_millis(100)));
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    advance(cx, 1050);
    advance(cx, 1050);
    assert_eq!(ticks(&backend), 4);
    assert!(state.read_with(cx, |state, _| state.has_pending_tick()));
    state.update(cx, |state, _| state.cancel_tick());
    advance(cx, 3000);
    assert_eq!(ticks(&backend), 4);

    // Three errors in a row stop the ticks until a user event.
    backend.fail_ticks.set(true);
    for _ in 0..3 {
        state.update(cx, |state, cx| {
            state.request_tick(Duration::from_millis(100), cx)
        });
        advance(cx, 1050);
    }
    assert_eq!(ticks(&backend), 7);
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    advance(cx, 3000);
    assert_eq!(ticks(&backend), 7, "stopped");
    backend.fail_ticks.set(false);
    backend.tick_again.set(None);
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx)
    });
    advance(cx, 0);
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_millis(100), cx)
    });
    advance(cx, 1050);
    assert_eq!(ticks(&backend), 8, "resumed after a user event");
}

/// Plan 2.3, design C and D: a modal starts closed (no mount), mounts afresh at each show,
/// echoes keyboard selection into its own tree and coalesces `row-selected`, sends exactly one
/// `dismissed(reason)` when the user closes it and nothing when the extension does, and drops
/// its tick on close.
#[gpui::test]
async fn test_modal_lifecycle(cx: &mut TestAppContext) {
    use ui_ext::v0_2::{DismissReason, ModalDescriptor, ModalWidth, SurfaceKind};
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let surface = ui_ext::v0_2::Surface {
        id: "counter".into(),
        title: "Pick".into(),
        kind: SurfaceKind::Modal(ModalDescriptor {
            width: ModalWidth::Medium,
        }),
    };
    let link: Rc<dyn crate::surface::SurfaceLink> = Rc::new(FakeLink(backend.clone()));
    let dyn_backend: Rc<dyn UiBackend> = backend.clone();
    let state = cx.new(|cx| SurfaceState::new(dyn_backend, surface, Some(link), cx));
    cx.run_until_parked();
    assert!(state.read_with(cx, |state, _| !state.is_open()));
    assert!(backend.calls().is_empty(), "nothing mounted while closed");

    state.update(cx, |state, cx| state.show(cx));
    cx.run_until_parked();
    assert!(state.read_with(cx, |state, _| state.tree().is_some()));
    assert_eq!(backend.take_calls(), [Call::Mount]);

    // Two rows, then keyboard selection: echoed locally, sent once.
    for _ in 0..2 {
        state.update(cx, |state, cx| {
            state.dispatch(UiEvent::Click(INCREMENT), cx)
        });
        cx.run_until_parked();
    }
    backend.take_calls();
    assert_eq!(
        state.read_with(cx, |state, _| state.first_list()),
        Some((LIST, None, 2))
    );
    state.update(cx, |state, cx| {
        state.select_row(LIST, 0, cx);
        state.select_row(LIST, 1, cx);
    });
    assert_eq!(
        state.read_with(cx, |state, _| state.first_list()),
        Some((LIST, Some(1), 2))
    );
    cx.run_until_parked();
    assert_eq!(
        backend.events(),
        [UiEvent::RowSelected(LIST, 1)],
        "consecutive selections are coalesced"
    );

    // The user closes it: the tree goes, one dismissed goes out, the tick is dropped.
    backend.take_calls();
    state.update(cx, |state, cx| {
        state.request_tick(Duration::from_secs(1), cx);
        state.close(Some(DismissReason::Escape), cx);
    });
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert!(!state.is_open());
        assert!(state.tree().is_none());
        assert!(!state.has_pending_tick());
    });
    assert_eq!(
        backend.events(),
        [UiEvent::Dismissed(DismissReason::Escape)]
    );

    // Shown again: a fresh mount; closed by the extension: nothing sent.
    backend.take_calls();
    state.update(cx, |state, cx| state.show(cx));
    cx.run_until_parked();
    assert_eq!(backend.take_calls(), [Call::Mount]);
    state.update(cx, |state, cx| state.close(None, cx));
    cx.run_until_parked();
    assert!(backend.calls().is_empty(), "{:?}", backend.calls());

    // Integration-5 L2: shown again before its `dismissed` went out, the mount waits for it.
    state.update(cx, |state, cx| state.show(cx));
    cx.run_until_parked();
    backend.take_calls();
    state.update(cx, |state, cx| {
        state.close(Some(DismissReason::Escape), cx);
        state.show(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        backend.take_calls(),
        [
            Call::Event(UiEvent::Dismissed(DismissReason::Escape)),
            Call::Mount
        ]
    );
    assert!(state.read_with(cx, |state, _| state.tree().is_some()));
}

/// Design K: the text of a secret input reaches the extension, but never the history the
/// Extension Inspector shows (nor the logs, which use the same redaction).
#[gpui::test]
async fn test_secret_input_is_redacted(cx: &mut TestAppContext) {
    init_test(cx);
    const SECRET: NodeId = 41;
    let backend = Rc::new(FakeBackend::default());
    let tree = Tree::from_nodes(
        vec![node(
            SECRET,
            None,
            Props::Input(InputProps {
                value: String::new(),
                placeholder: "Token".into(),
                secret: true,
                label: None,
            }),
        )],
        TreeLimits::default(),
    )
    .expect("valid tree");
    let dyn_backend: Rc<dyn UiBackend> = backend.clone();
    let state =
        cx.new(|cx| SurfaceState::with_tree(dyn_backend, panel_surface("p", "P"), None, tree, cx));
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::InputSubmitted(SECRET, "hunter2".into()), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        backend.events(),
        [UiEvent::InputSubmitted(SECRET, "hunter2".into())]
    );
    let history = state.read_with(cx, |state, _| format!("{:?}", state.history()));
    assert!(!history.contains("hunter2"), "{history}");
    assert!(history.contains("<secret>"), "{history}");
}

/// A backend whose calls answer only when the test says so.
#[derive(Default)]
struct GatedBackend {
    mounts: RefCell<Vec<futures::channel::oneshot::Sender<Result<Reply<Tree>>>>>,
    events: RefCell<Vec<futures::channel::oneshot::Sender<Result<Reply<Update>>>>>,
}

impl UiBackend for GatedBackend {
    fn extension_id(&self) -> &str {
        "gated"
    }

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.mounts.borrow_mut().push(tx);
        async move { rx.await.unwrap_or_else(|_| Err(anyhow!("dropped"))) }.boxed_local()
    }

    fn handle_event(&self, _: &str, _: UiEvent) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.events.borrow_mut().push(tx);
        async move { rx.await.unwrap_or_else(|_| Err(anyhow!("dropped"))) }.boxed_local()
    }

    fn list_rows(
        &self,
        _: &str,
        _: NodeId,
        _: u32,
        _: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<crate::backend::Row>>>> {
        async {
            Ok(Reply {
                effects: Vec::new(),
                value: Vec::new(),
            })
        }
        .boxed_local()
    }
}

fn gated_tree(label: &str) -> Result<Reply<Tree>> {
    Ok(Reply {
        effects: Vec::new(),
        value: Tree::from_nodes(
            vec![
                node(1, None, Props::Flex(FlexProps::default())),
                node(2, Some(1), text(label)),
                node(3, Some(1), Props::Button(ButtonProps::default())),
            ],
            TreeLimits::default(),
        )
        .unwrap(),
    })
}

/// Triage-3 #4: after a reload (`replace_backend`), a late reply or error of the old instance
/// is dropped: it neither patches the new tree nor stops the healthy extension. A surface that
/// failed while its mount was in flight stays failed when that mount answers.
#[gpui::test]
async fn test_stale_instance_results_are_dropped(cx: &mut TestAppContext) {
    init_test(cx);
    let old = Rc::new(GatedBackend::default());
    let dyn_old: Rc<dyn UiBackend> = old.clone();
    let state = cx.new(|cx| SurfaceState::new(dyn_old, panel_surface("p", "P"), None, cx));
    old.mounts
        .borrow_mut()
        .remove(0)
        .send(gated_tree("old"))
        .ok();
    cx.run_until_parked();
    // A click on the old instance, still unanswered when the reload happens.
    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(3), cx));
    cx.run_until_parked();
    assert_eq!(old.events.borrow().len(), 1);

    let new = Rc::new(GatedBackend::default());
    let dyn_new: Rc<dyn UiBackend> = new.clone();
    state.update(cx, |state, cx| {
        state.replace_backend(dyn_new, panel_surface("p", "P"), cx)
    });
    new.mounts
        .borrow_mut()
        .remove(0)
        .send(gated_tree("new"))
        .ok();
    cx.run_until_parked();
    assert_eq!(props_of(&state, 2, cx), text("new"));

    // The old instance's answer arrives: a patch, then (another call) a fatal error.
    old.events
        .borrow_mut()
        .remove(0)
        .send(Ok(Reply {
            effects: Vec::new(),
            value: Update::Patches(vec![Patch::SetProps(2, text("stale"))]),
        }))
        .ok();
    cx.run_until_parked();
    assert_eq!(props_of(&state, 2, cx), text("new"), "no stale patch");
    assert_eq!(failure(&state, cx), None);

    // A failure while a mount is in flight: the mount's answer does not revive the surface.
    state.update(cx, |state, cx| state.retry(cx));
    state.update(cx, |state, cx| state.fail("stopped".into(), false, cx));
    if let Some(mount) = new.mounts.borrow_mut().pop() {
        mount.send(gated_tree("revived")).ok();
    }
    cx.run_until_parked();
    assert_eq!(failure(&state, cx).as_deref(), Some("stopped"));
}

/// Triage-3 (ASYNC 10): a guest that never answers cannot make the queue grow without end.
#[gpui::test]
async fn test_event_queue_is_bounded(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(GatedBackend::default());
    let dyn_backend: Rc<dyn UiBackend> = backend.clone();
    let state = cx.new(|cx| SurfaceState::new(dyn_backend, panel_surface("p", "P"), None, cx));
    backend
        .mounts
        .borrow_mut()
        .remove(0)
        .send(gated_tree("t"))
        .ok();
    cx.run_until_parked();
    state.update(cx, |state, cx| {
        for _ in 0..(crate::surface::MAX_QUEUED_EVENTS + 20) {
            state.dispatch(UiEvent::Click(3), cx);
        }
    });
    cx.run_until_parked();
    // One is in flight; the rest wait, at most the cap.
    let queued = state.read_with(cx, |state, _| state.queued_events().len());
    assert!(
        queued <= crate::surface::MAX_QUEUED_EVENTS,
        "{queued} queued"
    );
}

/// Triage-3 (BUG 31): a reinstall's new title reaches the registry's surface info.
#[gpui::test]
async fn test_reinstall_takes_the_new_title(cx: &mut TestAppContext) {
    init_test(cx);
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let backend: Rc<dyn UiBackend> = Rc::new(FakeBackend::default());
    registry.update(cx, |registry, cx| {
        registry.register_backend(backend.clone(), &[panel_surface("main", "Old")], cx)
    });
    registry.update(cx, |registry, cx| {
        registry.register_backend(backend, &[panel_surface("main", "New")], cx)
    });
    let title = registry.read_with(cx, |registry, _| {
        registry.surface_of("fake", "main").unwrap().info.title
    });
    assert_eq!(title, "New");
}

/// `SurfaceState::with_tree` starts from a known tree, without a `mount` call (decorations).
#[gpui::test]
async fn test_surface_with_tree_does_not_mount(cx: &mut TestAppContext) {
    init_test(cx);
    let backend = Rc::new(FakeBackend::default());
    let tree = Tree::from_nodes(vec![node(1, None, text("given"))], TreeLimits::default())
        .expect("valid tree");
    let surface = panel_surface("decoration", "Decoration");
    let dyn_backend: Rc<dyn UiBackend> = backend.clone();
    let state = cx.new(|cx| SurfaceState::with_tree(dyn_backend, surface, None, tree, cx));
    cx.run_until_parked();
    assert_eq!(props_of(&state, 1, cx), text("given"));
    assert!(
        !backend.calls().contains(&Call::Mount),
        "no mount: {:?}",
        backend.calls()
    );
}

/// Triage-3, SEC T8: extension markdown never forms an image, so `data:` images are not decoded.
#[test]
fn test_markdown_images_are_not_parsed() {
    use crate::view::without_images;
    assert_eq!(
        without_images("a ![x](data:image/png;base64,AAAA) b"),
        r"a \![x](data:image/png;base64,AAAA) b"
    );
    assert_eq!(without_images("![r][ref]"), r"\![r][ref]");
    // Already escaped: left alone. Two backslashes are an escaped backslash: escaped again.
    assert_eq!(without_images(r"\![x](y)"), r"\![x](y)");
    assert_eq!(without_images(r"\\![x](y)"), r"\\\![x](y)");
    assert_eq!(
        without_images("Hi! [link](https://zed.dev)"),
        "Hi! [link](https://zed.dev)"
    );
    assert_eq!(without_images("!!["), r"!\![");
}

/// Sandbox: only http(s) links in an extension's markdown may open (after a confirmation).
#[test]
fn test_markdown_links_are_filtered() {
    use crate::view::link_is_allowed;
    for allowed in [
        "https://zed.dev",
        "http://example.com/a?b=c",
        "HTTPS://EXAMPLE.COM",
    ] {
        assert!(link_is_allowed(allowed), "{allowed}");
    }
    for refused in [
        "file:///C:/Windows/System32/calc.exe",
        "javascript:alert(1)",
        "zed://settings",
        "fyx://extension/install",
        "mailto:someone@example.com",
        "ftp://example.com",
        "relative/path.md",
        "https:",
        "https:/one-slash",
        "",
    ] {
        assert!(!link_is_allowed(refused), "{refused}");
    }
}

mod notification_tests {
    use super::*;
    use crate::notifications::{ExtensionNotification, provenance_name};
    use ui_ext::v0_2::{
        Notification, NotificationAction, NotificationActionEvent, NotificationCloseReason,
        NotificationLevel,
    };

    fn notification(auto_dismiss: bool) -> Notification {
        Notification {
            owner: "counter".into(),
            level: NotificationLevel::Info,
            title: "Build finished".into(),
            body: Some("3 problems".into()),
            actions: vec![NotificationAction {
                id: "open".into(),
                label: "Open".into(),
            }],
            auto_dismiss,
        }
    }

    /// A registry with the fake extension's "counter" panel mounted and a workspace to show in.
    fn setup(cx: &mut TestAppContext) -> (Rc<FakeBackend>, Entity<crate::UiExtensionRegistry>) {
        init_test(cx);
        let backend = Rc::new(FakeBackend::default());
        let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
        let workspace = cx.new(|_| ()).entity_id();
        registry.update(cx, |registry, cx| {
            let dyn_backend: Rc<dyn UiBackend> = backend.clone();
            registry.register_backend(dyn_backend, &[panel_surface("counter", "Counter")], cx);
            registry.set_active_workspace(workspace);
        });
        cx.run_until_parked();
        backend.take_calls();
        (backend, registry)
    }

    /// What the workspace does on `ShowNotification`: the view, in a window of its own.
    fn show<'a>(
        registry: &Entity<crate::UiExtensionRegistry>,
        id: u64,
        notification: Notification,
        cx: &'a mut TestAppContext,
    ) -> (Entity<ExtensionNotification>, &'a mut VisualTestContext) {
        registry.update(cx, |registry, cx| {
            registry.apply_effect("fake", HostEffect::Notify { id, notification }, cx)
        });
        let (name, notification) = registry
            .update(cx, |registry, _| {
                registry.take_pending_notification("fake", id)
            })
            .expect("pending");
        let weak = registry.downgrade();
        let (view, vcx) = cx.add_window_view(|window, cx| {
            ExtensionNotification::new(id, "fake".into(), name, notification, weak, window, cx)
        });
        vcx.update(|window, _| window.activate_window());
        registry.update(vcx, |registry, _| {
            registry.set_notification_view("fake", id, view.downgrade())
        });
        vcx.run_until_parked();
        // The test window's pointer starts over the card.
        view.update(vcx, |view, cx| view.set_hovered(false, cx));
        (view, vcx)
    }

    #[test]
    fn test_provenance_name_follows_the_registry_rule() {
        assert_eq!(provenance_name(Some("Todo Tools"), "todo"), "Todo Tools");
        assert_eq!(provenance_name(None, "todo"), "todo");
        assert_eq!(provenance_name(Some(""), "todo"), "todo");
        assert_eq!(provenance_name(Some("Fyx\u{202e}"), "todo"), "todo", "bidi");
        assert_eq!(provenance_name(Some("a\nb"), "todo"), "todo");
        assert_eq!(provenance_name(Some(&"x".repeat(65)), "todo"), "todo");
        assert_eq!(provenance_name(Some(" padded"), "todo"), "todo");
    }

    /// M.1: auto-dismiss expires with `notification-closed(expired)` after the level's time,
    /// paused while hovered and while the window is in the background.
    #[gpui::test]
    async fn test_notification_expires_and_pauses_on_hover(cx: &mut TestAppContext) {
        let (backend, registry) = setup(cx);
        let (view, vcx) = show(&registry, 1, notification(true), cx);
        vcx.executor().advance_clock(Duration::from_secs(3));
        view.update(vcx, |view, cx| view.set_hovered(true, cx));
        vcx.executor().advance_clock(Duration::from_secs(10));
        vcx.run_until_parked();
        assert_eq!(backend.events(), [], "paused while hovered");
        view.update(vcx, |view, cx| view.set_hovered(false, cx));
        // In the background it waits too.
        vcx.deactivate_window();
        vcx.executor().advance_clock(Duration::from_secs(10));
        vcx.run_until_parked();
        assert_eq!(backend.events(), [], "paused in the background");
        vcx.update(|window, _| window.activate_window());
        vcx.run_until_parked();
        vcx.executor().advance_clock(Duration::from_secs(2));
        vcx.run_until_parked();
        assert_eq!(backend.events(), [], "3 s of 6 were left");
        vcx.executor().advance_clock(Duration::from_secs(2));
        vcx.run_until_parked();
        assert_eq!(
            backend.take_calls(),
            [Call::Event(UiEvent::NotificationClosed(
                1,
                NotificationCloseReason::Expired
            ))]
        );
        assert_eq!(
            registry.read_with(vcx, |registry, _| registry.open_notifications("fake")),
            Vec::<u64>::new()
        );
    }

    /// M.1: a button click inside the guard is ignored; after it, `notification-action` goes to
    /// the owner and no `notification-closed` follows. The close button works at once.
    #[gpui::test]
    async fn test_notification_buttons_and_close(cx: &mut TestAppContext) {
        let (backend, registry) = setup(cx);
        let (view, vcx) = show(&registry, 1, notification(false), cx);
        vcx.executor().advance_clock(Duration::from_millis(500));
        view.update(vcx, |view, cx| view.click_action("open", cx));
        vcx.run_until_parked();
        assert_eq!(backend.events(), [], "inside the click guard");
        vcx.executor()
            .advance_clock(crate::notifications::CLICK_GUARD);
        view.update(vcx, |view, cx| view.click_action("open", cx));
        vcx.run_until_parked();
        assert_eq!(
            backend.take_calls(),
            [Call::Event(UiEvent::NotificationAction(
                NotificationActionEvent {
                    notification: 1,
                    action: "open".into()
                }
            ))]
        );
        drop(view);

        // Another one, closed by the user at once.
        registry.update(vcx, |registry, cx| {
            registry.apply_effect(
                "fake",
                HostEffect::Notify {
                    id: 2,
                    notification: notification(false),
                },
                cx,
            )
        });
        let (name, pending) = registry
            .update(vcx, |registry, _| {
                registry.take_pending_notification("fake", 2)
            })
            .expect("pending");
        let weak = registry.downgrade();
        let second = vcx.update(|window, cx| {
            cx.new(|cx| {
                ExtensionNotification::new(2, "fake".into(), name, pending, weak, window, cx)
            })
        });
        second.update(vcx, |view, cx| {
            view.close(NotificationCloseReason::User, cx)
        });
        vcx.run_until_parked();
        assert_eq!(
            backend.take_calls(),
            [Call::Event(UiEvent::NotificationClosed(
                2,
                NotificationCloseReason::User
            ))]
        );
    }

    /// M.1: `dismiss-notification` and the owner's removal close without events, and the
    /// window going away sends `window-closed`.
    #[gpui::test]
    async fn test_notifications_close_without_events(cx: &mut TestAppContext) {
        let (backend, registry) = setup(cx);
        let (view, vcx) = show(&registry, 1, notification(false), cx);
        registry.update(vcx, |registry, cx| {
            registry.apply_effect("fake", HostEffect::DismissNotification { id: 1 }, cx)
        });
        vcx.run_until_parked();
        assert_eq!(backend.events(), [], "no event on dismiss-notification");
        // The view is gone with its window: nothing more is sent.
        drop(view);
        vcx.update(|window, _| window.remove_window());
        vcx.run_until_parked();
        assert_eq!(backend.events(), []);

        registry.update(vcx, |registry, cx| {
            registry.apply_effect(
                "fake",
                HostEffect::Notify {
                    id: 2,
                    notification: notification(false),
                },
                cx,
            );
            registry.apply_effect(
                "fake",
                HostEffect::RemoveSurface {
                    surface_id: "counter".into(),
                },
                cx,
            )
        });
        vcx.run_until_parked();
        assert_eq!(backend.events(), [], "no event when the owner goes");
        assert_eq!(
            registry.read_with(vcx, |registry, _| registry.open_notifications("fake")),
            Vec::<u64>::new()
        );
    }

    /// An owner with no tree (a closed modal) still hears the event, so the host frees the slot.
    #[gpui::test]
    async fn test_events_reach_an_unmounted_owner(cx: &mut TestAppContext) {
        let (backend, registry) = setup(cx);
        let modal = ui_ext::v0_2::Surface {
            id: "picker".into(),
            title: "Picker".into(),
            kind: ui_ext::v0_2::SurfaceKind::Modal(ui_ext::v0_2::ModalDescriptor {
                width: ui_ext::v0_2::ModalWidth::Medium,
            }),
        };
        registry.update(cx, |registry, cx| {
            registry.apply_effect("fake", HostEffect::AddSurface(modal), cx)
        });
        cx.run_until_parked();
        backend.take_calls();
        let mut owned_by_modal = notification(false);
        owned_by_modal.owner = "picker".into();
        let (view, vcx) = show(&registry, 4, owned_by_modal, cx);
        view.update(vcx, |view, cx| {
            view.close(NotificationCloseReason::User, cx)
        });
        vcx.run_until_parked();
        assert_eq!(
            backend.take_calls(),
            [Call::Event(UiEvent::NotificationClosed(
                4,
                NotificationCloseReason::User
            ))]
        );
    }

    /// M.1: an open notification whose window goes away tells its owner `window-closed`.
    #[gpui::test]
    async fn test_notification_window_closes(cx: &mut TestAppContext) {
        let (backend, registry) = setup(cx);
        let (view, vcx) = show(&registry, 1, notification(false), cx);
        drop(view);
        vcx.update(|window, _| window.remove_window());
        vcx.run_until_parked();
        assert_eq!(
            backend.take_calls(),
            [Call::Event(UiEvent::NotificationClosed(
                1,
                NotificationCloseReason::WindowClosed
            ))]
        );
    }
    /// integration-6 N2 (decision Q10): the T2 first-use prompt is a card, not a modal. Its
    /// buttons ignore clicks for 1 s; no key answers it, even with the card focused; Allow then
    /// grants. Closing it answers nothing: this request is denied and nothing is recorded.
    #[gpui::test]
    async fn test_permission_card_guard_and_no_keyboard_answer(cx: &mut TestAppContext) {
        use crate::notifications::{CLICK_GUARD, PermissionCard};
        use futures::FutureExt as _;
        use ui_ext_wasm::permissions::PermissionState;
        init_test(cx);
        let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
        let workspace = cx.new(|_| ()).entity_id();
        let checker = registry.update(cx, |registry, cx| {
            registry.start_permission_prompts(cx);
            registry.set_active_workspace(workspace);
            registry.consent_mut().declare(
                "demo",
                vec![
                    "network:fetch".into(),
                    "network:fetch@api.example.com".into(),
                ],
            );
            registry.capability_check()
        });
        let state = |registry: &Entity<crate::UiExtensionRegistry>, cx: &mut TestAppContext| {
            registry.read_with(cx, |registry, _| registry.t2_state("demo", "network:fetch"))
        };

        // Closed without an answer: denied this time, still asked next time.
        let request = checker.request("demo", "network:fetch");
        cx.run_until_parked();
        let (card, vcx) = cx.add_window_view(|_, cx| {
            PermissionCard::new("demo".into(), "network:fetch".into(), &registry, cx)
        });
        card.update(vcx, |card, cx| card.close(cx));
        vcx.run_until_parked();
        assert_eq!(request.now_or_never(), Some(PermissionState::Denied));
        assert_eq!(
            state(&registry, vcx),
            PermissionState::Ask,
            "nothing recorded"
        );
        vcx.update(|window, _| window.remove_window());

        let request = checker.request("demo", "network:fetch");
        cx.run_until_parked();
        let (card, vcx) = cx.add_window_view(|_, cx| {
            PermissionCard::new("demo".into(), "network:fetch".into(), &registry, cx)
        });
        let armed = card.read_with(vcx, |card, cx| card.buttons_armed(cx));
        assert!(!armed, "not armed when shown");
        vcx.executor()
            .advance_clock(CLICK_GUARD - Duration::from_millis(1));
        card.update(vcx, |card, cx| card.answer(true, cx));
        vcx.run_until_parked();
        assert_eq!(
            state(&registry, vcx),
            PermissionState::Ask,
            "inside the guard"
        );

        vcx.executor().advance_clock(Duration::from_millis(1));
        vcx.update(|window, cx| gpui::Focusable::focus_handle(card.read(cx), cx).focus(window, cx));
        vcx.simulate_keystrokes("enter space escape");
        vcx.run_until_parked();
        assert_eq!(
            state(&registry, vcx),
            PermissionState::Ask,
            "keys answer nothing"
        );

        card.update(vcx, |card, cx| card.answer(true, cx));
        vcx.run_until_parked();
        assert_eq!(state(&registry, vcx), PermissionState::Granted);
        assert_eq!(request.now_or_never(), Some(PermissionState::Granted));
    }
}

mod progress_tests {
    use super::*;
    use crate::progress::{Progresses, panel_bar};
    use ui_ext::v0_2::{ProgressLocation, ProgressOptions, ProgressValue};

    fn options(owner: &str, title: &str, location: ProgressLocation) -> ProgressOptions {
        ProgressOptions {
            owner: owner.into(),
            title: title.into(),
            location,
            cancellable: true,
        }
    }

    fn setup(
        cx: &mut TestAppContext,
    ) -> (
        Rc<FakeBackend>,
        Entity<crate::UiExtensionRegistry>,
        Entity<Progresses>,
    ) {
        init_test(cx);
        cx.update(crate::progress::init);
        let backend = Rc::new(FakeBackend::default());
        let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
        registry.update(cx, |registry, cx| {
            let dyn_backend: Rc<dyn UiBackend> = backend.clone();
            registry.register_backend(dyn_backend, &[panel_surface("counter", "Counter")], cx);
        });
        cx.run_until_parked();
        backend.take_calls();
        let progresses = cx.update(|cx| Progresses::global(cx).expect("initialized"));
        (backend, registry, progresses)
    }

    fn effect(
        registry: &Entity<crate::UiExtensionRegistry>,
        effect: HostEffect,
        cx: &mut TestAppContext,
    ) {
        registry.update(cx, |registry, cx| registry.apply_effect("fake", effect, cx));
        cx.run_until_parked();
    }

    /// M.2: start, update (no message keeps the last), the status-bar grouping with the newest
    /// last, and end.
    #[gpui::test]
    async fn test_progress_effects(cx: &mut TestAppContext) {
        let (backend, registry, progresses) = setup(cx);
        for (id, title) in [(1, "Indexing"), (2, "Building")] {
            effect(
                &registry,
                HostEffect::ProgressStart {
                    id,
                    options: options("counter", title, ProgressLocation::StatusBar),
                },
                cx,
            );
        }
        effect(
            &registry,
            HostEffect::ProgressUpdate {
                id: 1,
                value: ProgressValue::Percent(40),
                message: Some("src/".into()),
            },
            cx,
        );
        effect(
            &registry,
            HostEffect::ProgressUpdate {
                id: 1,
                value: ProgressValue::Percent(60),
                message: None,
            },
            cx,
        );
        progresses.read_with(cx, |progresses, _| {
            let groups = progresses.status_bar();
            assert_eq!(groups.len(), 1, "one chip per extension");
            let titles: Vec<&str> = groups[0]
                .1
                .iter()
                .map(|running| running.options.title.as_str())
                .collect();
            assert_eq!(titles, ["Indexing", "Building"], "the newest last");
            let first = groups[0].1[0];
            assert_eq!(first.value, ProgressValue::Percent(60));
            assert_eq!(first.message.as_deref(), Some("src/"), "kept");
        });
        effect(&registry, HostEffect::ProgressEnd { id: 1 }, cx);
        progresses.read_with(cx, |progresses, _| {
            assert_eq!(progresses.running().len(), 1)
        });
        assert_eq!(backend.events(), [], "progress effects send no events");
    }

    /// M.2: Cancel sends `progress-cancel-requested` once; the progress stays until the
    /// extension ends it. The owner's removal ends it without an event.
    #[gpui::test]
    async fn test_progress_cancel_and_owner_removal(cx: &mut TestAppContext) {
        let (backend, registry, progresses) = setup(cx);
        effect(
            &registry,
            HostEffect::ProgressStart {
                id: 3,
                options: options("counter", "Sync", ProgressLocation::Panel),
            },
            cx,
        );
        assert!(cx.update(|cx| panel_bar("fake", "counter", cx).is_some()));
        assert!(cx.update(|cx| panel_bar("fake", "other", cx).is_none()));
        for _ in 0..2 {
            registry.update(cx, |registry, cx| {
                registry.request_progress_cancel("fake", 3, cx)
            });
            cx.run_until_parked();
        }
        assert_eq!(
            backend.take_calls(),
            [Call::Event(UiEvent::ProgressCancelRequested(3))],
            "once"
        );
        progresses.read_with(cx, |progresses, _| {
            assert!(
                progresses.running()[0].cancel_requested,
                "it stays, Cancel disabled"
            );
        });

        effect(
            &registry,
            HostEffect::RemoveSurface {
                surface_id: "counter".into(),
            },
            cx,
        );
        progresses.read_with(cx, |progresses, _| assert!(progresses.running().is_empty()));
        assert_eq!(backend.events(), []);
    }

    /// M.2: at most 10 redraws a second, whatever the rate of updates; the last value wins.
    #[gpui::test]
    async fn test_progress_redraws_are_coalesced(cx: &mut TestAppContext) {
        let (_, registry, progresses) = setup(cx);
        effect(
            &registry,
            HostEffect::ProgressStart {
                id: 1,
                options: options("counter", "Copy", ProgressLocation::StatusBar),
            },
            cx,
        );
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        let notified = Rc::new(Cell::new(0));
        let _observe = cx.update(|cx| {
            let notified = notified.clone();
            cx.observe(&progresses, move |_, _| notified.set(notified.get() + 1))
        });
        for percent in 0..100u8 {
            registry.update(cx, |registry, cx| {
                registry.apply_effect(
                    "fake",
                    HostEffect::ProgressUpdate {
                        id: 1,
                        value: ProgressValue::Percent(percent),
                        message: None,
                    },
                    cx,
                )
            });
            cx.executor().advance_clock(Duration::from_millis(10));
            cx.run_until_parked();
        }
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        assert!(
            (9..=12).contains(&notified.get()),
            "{} redraws for 1 s of updates",
            notified.get()
        );
        progresses.read_with(cx, |progresses, _| {
            assert_eq!(progresses.running()[0].value, ProgressValue::Percent(99))
        });
    }
}

/// Plan 4.14: a clickable canvas sends `canvas-click` in canvas coordinates; one that is not
/// clickable sends nothing.
#[gpui::test]
async fn test_canvas_click_in_canvas_coordinates(cx: &mut TestAppContext) {
    use ui_ext::canvas::{CanvasClick, CanvasFit, CanvasProps, Num};
    init_test(cx);
    let canvas = |clickable: bool| {
        Props::Canvas(CanvasProps {
            width: 200,
            height: 100,
            description: "A chart".into(),
            ops: Vec::new(),
            clickable,
            fit: CanvasFit::Clip,
        })
    };
    for clickable in [true, false] {
        let backend = Rc::new(FakeBackend::default());
        let tree = Tree::from_nodes(
            vec![node(1, None, canvas(clickable))],
            TreeLimits::default(),
        )
        .expect("valid tree");
        let dyn_backend: Rc<dyn UiBackend> = backend.clone();
        let state = cx.new(|cx| {
            SurfaceState::with_tree(dyn_backend, panel_surface("chart", "Chart"), None, tree, cx)
        });
        let (_view, vcx) = open_view(&state, cx);
        let bounds = vcx.debug_bounds("fyx-node-1").expect("the canvas is drawn");
        vcx.simulate_click(
            bounds.origin + gpui::point(gpui::px(50.), gpui::px(20.)),
            Modifiers::none(),
        );
        vcx.run_until_parked();
        let expected: Vec<UiEvent> = if clickable {
            vec![UiEvent::CanvasClick(CanvasClick {
                canvas: 1,
                x: Num(50.),
                y: Num(20.),
            })]
        } else {
            Vec::new()
        };
        assert_eq!(backend.events(), expected, "clickable: {clickable}");
    }
}

/// Plan 4.14: every op kind paints (tokens and data colours, both fits) without a panic.
#[gpui::test]
async fn test_canvas_paints_every_op(cx: &mut TestAppContext) {
    use ui_ext::canvas::*;
    init_test(cx);
    let p = |x: f32, y: f32| Point {
        x: Num(x),
        y: Num(y),
    };
    let paint = |color| Paint { color, opacity: 80 };
    let stroke = Stroke {
        paint: paint(CanvasColor::Data(DataColor::Series2)),
        width: Num(2.),
    };
    let rect = Rect {
        x: Num(10.),
        y: Num(10.),
        width: Num(40.),
        height: Num(-20.),
    };
    let ops = vec![
        DrawOp::Line {
            from: p(0., 0.),
            to: p(100., 50.),
            stroke,
        },
        DrawOp::Rect {
            bounds: rect,
            fill: Some(paint(CanvasColor::Token(
                ui_ext::v0_2::ColorToken::TextMuted,
            ))),
            stroke: Some(stroke),
        },
        DrawOp::RoundedRect {
            bounds: rect,
            radius: Num(99.),
            fill: None,
            stroke: Some(stroke),
        },
        DrawOp::Circle {
            center: p(60., 40.),
            radius: Num(10.),
            fill: Some(paint(CanvasColor::Data(DataColor::Series8))),
            stroke: None,
        },
        DrawOp::Path {
            segments: vec![
                PathSegment::MoveTo(p(0., 0.)),
                PathSegment::QuadTo(p(10., 30.), p(20., 0.)),
                PathSegment::CubicTo(p(25., 10.), p(30., 10.), p(40., 0.)),
                PathSegment::LineTo(p(40., 20.)),
                PathSegment::Close,
            ],
            fill: Some(paint(CanvasColor::Data(DataColor::Series1))),
            stroke: Some(stroke),
        },
        DrawOp::Text {
            at: p(100., 50.),
            text: "42 %".into(),
            paint: paint(CanvasColor::Token(ui_ext::v0_2::ColorToken::Text)),
            size: CanvasTextSize::Large,
            anchor: TextAnchor::Middle,
            monospace: true,
        },
    ];
    for fit in [CanvasFit::Clip, CanvasFit::ScaleDown] {
        let props = Props::Canvas(CanvasProps {
            width: 4000,
            height: 100,
            description: "Every op".into(),
            ops: ops.clone(),
            clickable: false,
            fit,
        });
        let backend = Rc::new(FakeBackend::default());
        let tree =
            Tree::from_nodes(vec![node(1, None, props)], TreeLimits::default()).expect("valid");
        let dyn_backend: Rc<dyn UiBackend> = backend.clone();
        let state = cx.new(|cx| {
            SurfaceState::with_tree(dyn_backend, panel_surface("chart", "Chart"), None, tree, cx)
        });
        let (_view, vcx) = open_view(&state, cx);
        assert!(vcx.debug_bounds("fyx-node-1").is_some(), "{fit:?} drawn");
    }
}

/// The proxy cell follows Fyx's `proxy` setting, the raw value (never the environment), and an
/// unusable value goes direct.
#[gpui::test]
fn test_extension_proxy_follows_the_setting(cx: &mut TestAppContext) {
    init_test(cx);
    cx.update(crate::registry::follow_proxy_setting);
    let proxy = cx.update(crate::registry::extension_proxy);
    assert_eq!(proxy.get(), None, "no setting: direct");
    let set = |value: Option<&str>, cx: &mut TestAppContext| {
        let value = value.map(str::to_string);
        cx.update(|cx| {
            gpui::BorrowAppContext::update_global::<SettingsStore, _>(cx, |store, cx| {
                store.update_user_settings(cx, |settings| settings.proxy = value)
            })
        });
    };
    set(Some(" http://proxy.example:8080 "), cx);
    assert_eq!(
        proxy.get().map(|url| url.to_string()),
        Some("http://proxy.example:8080/".to_string())
    );
    set(Some("not a url"), cx);
    assert_eq!(proxy.get(), None, "unusable: direct");
    set(None, cx);
    assert_eq!(proxy.get(), None);
}
