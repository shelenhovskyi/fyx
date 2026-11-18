//! Modal surfaces (design.md, C and D): an extension surface shown in the workspace's modal
//! layer, opened by `show-surface` after a click or Enter (the host checked that), closed by
//! `hide-surface` or by the user.
//!
//! - The tree comes from a fresh `mount` each time it is shown; until it arrives the modal frame
//!   of the declared width is already open, so focus moves at once.
//! - Every close the user makes sends exactly one `dismissed(reason)`: `escape` from the modal's
//!   own Cancel; `replaced` when another modal took the layer; `focus-lost` for a click outside
//!   or focus elsewhere; `window-closed` when the window goes while it is shown. A close the
//!   extension asked for sends nothing.
//! - Up and Down (and Home and End, as the menu's first and last) move the selection of the
//!   modal's first list from anywhere in it (`row-selected`); Enter in an input still sends
//!   `input-submitted`.
//! - It never vetoes its dismissal: the host cannot ask the guest synchronously.

use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, WeakEntity, Window, prelude::*, rems,
};
use theme::ActiveTheme as _;
use ui::{Label, LabelCommon as _, LabelSize, StyledExt as _, v_flex};
use ui_ext::v0_2::{DismissReason, ModalWidth};
use workspace::{DismissDecision, ModalView, Workspace};

use crate::{SurfaceState, SurfaceView};

pub struct ExtensionModal {
    /// `<extension id>/<surface id>`.
    key: SharedString,
    title: SharedString,
    width: ModalWidth,
    state: Entity<SurfaceState>,
    view: Entity<SurfaceView>,
    workspace: WeakEntity<Workspace>,
    /// Set by the modal's own Cancel (Escape) before it asks to be dismissed.
    reason: Option<DismissReason>,
    /// The extension closed it (`hide-surface`, `remove-surface`): no `dismissed`.
    closed_by_host: bool,
    /// `dismissed` was already queued (so `on_release` must not send another).
    dismiss_sent: bool,
    focused_first_input: bool,
    _observe_state: gpui::Subscription,
}

impl ExtensionModal {
    pub fn new(
        key: SharedString,
        state: Entity<SurfaceState>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let surface = state.read(cx).surface().clone();
        let width = match surface.kind {
            ui_ext::v0_2::SurfaceKind::Modal(descriptor) => descriptor.width,
            _ => ModalWidth::Medium,
        };
        let view = cx.new(|cx| SurfaceView::new(state.clone(), false, window, cx));
        let observe_state = cx.observe_in(&state, window, |this, _, window, cx| {
            // Focus the first input once the tree is there (or keep it on the modal).
            if !this.focused_first_input && this.state.read(cx).tree().is_some() {
                this.focused_first_input = true;
                this.view
                    .update(cx, |view, cx| view.focus_first_input(window, cx));
            }
            cx.notify();
        });
        // A shown modal is visible, so its ticks run (integration-5 H1); it stops being visible
        // when it goes (dismissed, closed by the extension, or its window closed).
        let container = cx.entity_id();
        state.update(cx, |state, cx| state.set_visible(container, true, cx));
        cx.on_release(move |this: &mut Self, cx| {
            this.state
                .update(cx, |state, cx| state.set_visible(container, false, cx));
            // The window went while the modal was shown.
            if !this.closed_by_host && !this.dismiss_sent {
                this.state.update(cx, |state, cx| {
                    state.close(Some(DismissReason::WindowClosed), cx)
                });
            }
        })
        .detach();
        Self {
            key,
            title: surface.title.into(),
            width,
            state,
            view,
            workspace,
            reason: None,
            closed_by_host: false,
            dismiss_sent: false,
            focused_first_input: false,
            _observe_state: observe_state,
        }
    }

    pub fn key(&self) -> &SharedString {
        &self.key
    }

    /// The extension asked for the close; no `dismissed` goes out.
    pub fn close_by_host(&mut self, cx: &mut Context<Self>) {
        self.closed_by_host = true;
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.reason = Some(DismissReason::Escape);
        cx.emit(DismissEvent);
    }

    fn move_selection(&mut self, delta: i64, cx: &mut Context<Self>) {
        let Some((list, selected, row_count)) = self.state.read(cx).first_list() else {
            return;
        };
        if row_count == 0 {
            return;
        }
        let last = i64::from(row_count - 1);
        let next = match selected {
            Some(selected) => (i64::from(selected) + delta).clamp(0, last),
            None if delta >= 0 => 0,
            None => last,
        };
        self.state.update(cx, |state, cx| {
            state.select_row(list, next as u32, cx);
        });
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, cx);
    }

    fn select_page(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some((list, _, _)) = self.state.read(cx).first_list() else {
            return;
        };
        let row_height = match self.state.read(cx).tree().and_then(|tree| tree.get(list)) {
            Some(node) => match node.props() {
                ui_ext::v0_2::Props::List(props) => props.row_height,
                _ => return,
            },
            None => return,
        };
        let page = i64::from(self.view.read(cx).page_rows(list, row_height));
        self.move_selection(if down { page } else { -page }, cx);
    }

    fn select_page_up(
        &mut self,
        _: &crate::keyboard::SelectPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_page(false, cx);
    }

    fn select_page_down(
        &mut self,
        _: &crate::keyboard::SelectPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_page(true, cx);
    }

    fn select_last(&mut self, _: &menu::SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(i64::MAX / 2, cx);
    }

    fn select_first(&mut self, _: &menu::SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(i64::MIN / 2, cx);
    }
}

impl EventEmitter<DismissEvent> for ExtensionModal {}

impl Focusable for ExtensionModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle.clone()
    }
}

impl ModalView for ExtensionModal {
    fn on_before_dismiss(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DismissDecision {
        if self.closed_by_host || self.dismiss_sent {
            return DismissDecision::Dismiss(true);
        }
        self.dismiss_sent = true;
        let state = self.state.clone();
        let recorded = self.reason.take();
        let workspace = self.workspace.clone();
        // Without a recorded reason, decide one frame later: another modal open by then means
        // this one was replaced; otherwise focus went elsewhere.
        window.defer(cx, move |window, cx| {
            let reason = recorded.unwrap_or_else(|| {
                let replaced = workspace.upgrade().is_some_and(|workspace| {
                    workspace.update(cx, |workspace, cx| workspace.has_active_modal(window, cx))
                });
                if replaced {
                    DismissReason::Replaced
                } else {
                    DismissReason::FocusLost
                }
            });
            state.update(cx, |state, cx| state.close(Some(reason), cx));
        });
        DismissDecision::Dismiss(true)
    }
}

impl Render for ExtensionModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = match self.width {
            ModalWidth::Small => rems(24.),
            ModalWidth::Medium => rems(34.),
            ModalWidth::Large => rems(48.),
        };
        let max_height = window.viewport_size().height * 0.6;
        v_flex()
            .key_context(crate::keyboard::MODAL_CONTEXT)
            .elevation_3(cx)
            .w(width)
            .max_h(max_height)
            .overflow_hidden()
            .bg(cx.theme().colors().elevated_surface_background)
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .when(!self.title.is_empty(), |this| {
                this.child(
                    v_flex()
                        .px_2()
                        .py_1()
                        .border_b_1()
                        .border_color(cx.theme().colors().border_variant)
                        .child(Label::new(self.title.clone()).size(LabelSize::Small)),
                )
            })
            .child(v_flex().flex_1().min_h_0().child(self.view.clone()))
    }
}
