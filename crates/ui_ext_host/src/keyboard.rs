//! Keyboard support for extension surfaces.
//!
//! - **Tab / Shift-Tab** move focus between the surface's enabled buttons, inputs, lists and
//!   tables, in tree order, wrapping around.
//! - **Enter / Space** on a focused button send one `click`, the same event a mouse click sends.
//! - **Up / Down / Home / End / PageUp / PageDown** move the selection of a focused list or table;
//!   **Enter** sends `row-click` for the selected row.
//! - **Escape** returns focus to the editor (`workspace::FocusCenterPane`), as other panels do.
//!
//! No key binding is added. Enter, Escape, Up, Down, Home and End already map to `menu::*` actions
//! in the default keymap, and the surface handles those. Tab, Shift-Tab, Space and the page keys
//! are taken by a keystroke interceptor, but only while focus is inside a surface (key context
//! `FyxSurface`). Upstream binds Tab to `menu::SelectNext`, the same action as Down, so a list
//! could not tell "leave the list" from "next row". Interceptors are not key bindings, so a keymap
//! reload (which clears every binding registered from code) does not drop them.

use gpui::{Action, App, Context, FocusHandle, Focusable as _, ScrollStrategy, Window, actions};
use ui_ext::v0_2::{NodeId, Props, UiEvent};

use crate::view::{DEFAULT_ROW_HEIGHT, SurfaceView};

actions!(
    fyx_ui,
    [
        /// Moves focus to the next button, input or list of the surface.
        FocusNext,
        /// Moves focus to the previous button, input or list of the surface.
        FocusPrevious,
        /// Activates the focused button (Space).
        Activate,
        /// Moves a list's selection up by a page.
        SelectPageUp,
        /// Moves a list's selection down by a page.
        SelectPageDown,
    ]
);

/// Key context of every surface view.
pub(crate) const SURFACE_CONTEXT: &str = "FyxSurface";
/// Key context around a focusable button.
pub(crate) const BUTTON_CONTEXT: &str = "FyxSurfaceButton";
/// Key context of a focusable list or table.
pub(crate) const LIST_CONTEXT: &str = "FyxSurfaceList";

/// Takes Tab, Shift-Tab, Space and the page keys while focus is inside a surface.
pub(crate) fn intercept_keys(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        let in_context = |name: &str| {
            event
                .context_stack
                .iter()
                .any(|context| context.contains(name))
        };
        if !in_context(SURFACE_CONTEXT) {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
            return;
        }
        let action: Box<dyn Action> = match (keystroke.key.as_str(), modifiers.shift) {
            ("tab", false) => Box::new(FocusNext),
            ("tab", true) => Box::new(FocusPrevious),
            ("space", false) if in_context(BUTTON_CONTEXT) => Box::new(Activate),
            ("pageup", false) if in_context(LIST_CONTEXT) => Box::new(SelectPageUp),
            ("pagedown", false) if in_context(LIST_CONTEXT) => Box::new(SelectPageDown),
            _ => return,
        };
        window.dispatch_action(action, cx);
        cx.stop_propagation();
    })
    .detach();
}

#[derive(Clone, Copy)]
enum Selection {
    Next,
    Previous,
    First,
    Last,
    PageUp,
    PageDown,
}

impl SurfaceView {
    /// The surface node that holds focus in `window`, if any.
    pub(crate) fn focused_node(&self, window: &Window, cx: &App) -> Option<NodeId> {
        self.button_focus
            .iter()
            .chain(&self.list_focus)
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(id, _)| *id)
            .or_else(|| {
                self.inputs
                    .iter()
                    .find(|(_, input)| input.editor.focus_handle(cx).contains_focused(window, cx))
                    .map(|(id, _)| *id)
            })
    }

    fn focus_handle_of(&self, id: NodeId, cx: &App) -> Option<FocusHandle> {
        self.button_focus
            .get(&id)
            .or_else(|| self.list_focus.get(&id))
            .cloned()
            .or_else(|| {
                self.inputs
                    .get(&id)
                    .map(|input| input.editor.focus_handle(cx))
            })
    }

    fn focus_node(&self, id: NodeId, window: &mut Window, cx: &mut App) {
        if let Some(handle) = self.focus_handle_of(id, cx) {
            handle.focus(window, cx);
        }
    }

    fn move_focus(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.focus_order.len();
        if count == 0 {
            return;
        }
        let current = self
            .focused_node(window, cx)
            .and_then(|id| self.focus_order.iter().position(|&node| node == id));
        let next = match (current, forward) {
            (Some(index), true) => (index + 1) % count,
            (Some(index), false) => (index + count - 1) % count,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        self.focus_node(self.focus_order[next], window, cx);
        cx.notify();
    }

    pub(crate) fn focus_next(
        &mut self,
        _: &FocusNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_focus(true, window, cx);
    }

    pub(crate) fn focus_previous(
        &mut self,
        _: &FocusPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_focus(false, window, cx);
    }

    /// The focused node if it is an enabled button.
    fn focused_button(&self, window: &Window, cx: &App) -> Option<NodeId> {
        let id = self.focused_node(window, cx)?;
        let tree = self.state().read(cx).tree()?;
        match tree.get(id)?.props() {
            Props::Button(button) if !button.disabled => Some(id),
            _ => None,
        }
    }

    /// The focused list or table, with its row count and row height.
    fn focused_list(&self, window: &Window, cx: &App) -> Option<(NodeId, u32, u16)> {
        let id = self.focused_node(window, cx)?;
        let tree = self.state().read(cx).tree()?;
        match tree.get(id)?.props() {
            Props::List(list) => Some((id, list.row_count, list.row_height)),
            Props::Table(table) => Some((id, table.row_count, DEFAULT_ROW_HEIGHT)),
            _ => None,
        }
    }

    pub(crate) fn activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.focused_button(window, cx) {
            self.dispatch(UiEvent::Click(id), cx);
        }
    }

    pub(crate) fn confirm(
        &mut self,
        _: &menu::Confirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.focused_button(window, cx) {
            self.dispatch(UiEvent::Click(id), cx);
        } else if let Some((list, _, _)) = self.focused_list(window, cx)
            && let Some(&index) = self.selection.get(&list)
        {
            self.dispatch(UiEvent::RowClick(list, index), cx);
        } else {
            cx.propagate();
        }
    }

    fn move_selection(&mut self, how: Selection, window: &mut Window, cx: &mut Context<Self>) {
        let Some((list, row_count, row_height)) = self.focused_list(window, cx) else {
            cx.propagate();
            return;
        };
        if row_count == 0 {
            return;
        }
        let last = row_count - 1;
        let page = self
            .scroll_handles
            .get(&list)
            .map(|handle| {
                let height = handle.0.borrow().base_handle.bounds().size.height;
                (height / gpui::px(row_height.max(1).into())).floor() as u32
            })
            .unwrap_or(0)
            .max(1);
        let current = self.selection.get(&list).copied();
        let next = match (how, current) {
            (Selection::Next, Some(index)) => (index + 1).min(last),
            (Selection::Previous, Some(index)) => index.saturating_sub(1),
            (Selection::Next | Selection::Previous | Selection::First, None) => 0,
            (Selection::First, Some(_)) => 0,
            (Selection::Last, _) => last,
            (Selection::PageDown, current) => current.map_or(0, |index| index + page).min(last),
            (Selection::PageUp, current) => current.map_or(0, |index| index.saturating_sub(page)),
        };
        self.selection.insert(list, next);
        if let Some(handle) = self.scroll_handles.get(&list) {
            handle.scroll_to_item(next as usize, ScrollStrategy::Top);
        }
        cx.notify();
    }

    pub(crate) fn select_next(
        &mut self,
        _: &menu::SelectNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(Selection::Next, window, cx);
    }

    pub(crate) fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(Selection::Previous, window, cx);
    }

    pub(crate) fn select_first(
        &mut self,
        _: &menu::SelectFirst,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(Selection::First, window, cx);
    }

    pub(crate) fn select_last(
        &mut self,
        _: &menu::SelectLast,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(Selection::Last, window, cx);
    }

    pub(crate) fn select_page_up(
        &mut self,
        _: &SelectPageUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(Selection::PageUp, window, cx);
    }

    pub(crate) fn select_page_down(
        &mut self,
        _: &SelectPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(Selection::PageDown, window, cx);
    }

    /// Escape: back to the editor, as other panels do.
    pub(crate) fn cancel(&mut self, _: &menu::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.return_to_editor(window, cx);
    }

    /// Escape inside an input that had nothing of its own to cancel.
    pub(crate) fn cancel_input(
        &mut self,
        _: &editor::actions::Cancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.return_to_editor(window, cx);
    }

    fn return_to_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(Box::new(workspace::FocusCenterPane), cx);
    }

    /// Keeps keyboard focus on the surface when a patch removes the focused node: the next node in
    /// the old Tab order that still exists takes it, else the closest one before it, else the
    /// surface itself.
    pub(crate) fn restore_focus(
        &mut self,
        focused_before: Option<NodeId>,
        old_order: &[NodeId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(gone) = focused_before else {
            return;
        };
        if self.focused_node(window, cx).is_some() {
            return;
        }
        if self.focus_handle_of(gone, cx).is_some() {
            return;
        }
        let position = old_order.iter().position(|&id| id == gone).unwrap_or(0);
        let still_there = |id: &&NodeId| self.focus_order.contains(id);
        let replacement = old_order[position..]
            .iter()
            .find(still_there)
            .or_else(|| old_order[..position].iter().rev().find(still_there))
            .copied();
        match replacement {
            Some(id) => self.focus_node(id, window, cx),
            None => self.focus_handle.focus(window, cx),
        }
    }
}
