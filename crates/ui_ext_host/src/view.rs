//! Draws a retained surface tree with native GPUI and `ui` elements.
//!
//! `render()` only reads `SurfaceState`; it never calls the extension. Clicks and edits are routed
//! by node id to `SurfaceState::dispatch`, which makes the call on a task.

use std::{ops::Range, sync::Arc, time::Duration};

use collections::{HashMap, HashSet};
use editor::{
    Editor, EditorEvent, EditorMode, HiddenUnstagedDiffHunkRenderer, MinimapVisibility,
    SizingBehavior,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, FocusHandle, Focusable as _,
    FontWeight, Global, Hsla, IntoElement, ListSizingBehavior, ParentElement, Pixels, Render,
    RenderOnce, ScrollHandle, SharedString, Styled, Subscription, Task, UniformListScrollHandle,
    WeakEntity, Window, div, prelude::*, px, uniform_list,
};
use language::language_settings::SoftWrap;
use markdown::{Markdown, MarkdownElement, MarkdownFont, MarkdownStyle};
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonStyle as UiButtonStyle, Clickable as _, Color,
    CommonAnimationExt as _, Disableable as _, Icon, IconButton, IconName, IconSize, Label,
    LabelCommon as _, LabelSize, TintColor, Tooltip, WithScrollbar as _, h_flex, v_flex,
};
use ui_ext::v0_2::{
    Align, ButtonProps, ButtonStyle, ColorToken, DiffViewProps, FlexDirection, FlexProps,
    IconProps, InputProps, ListProps, NodeId, Props, TableProps, TextProps, UiEvent, tree::Tree,
};

use crate::surface::{Phase, SurfaceState, fetch_range};

/// Typing is sent to the extension once the user pauses this long.
pub const TYPING_DEBOUNCE: Duration = Duration::from_millis(150);
/// A list or table shows at least this many rows when its parent does not give it a height.
const MIN_VISIBLE_ROWS: u32 = 3;
pub(crate) const DEFAULT_ROW_HEIGHT: u16 = 24;

pub(crate) struct InputEditor {
    pub(crate) editor: Entity<Editor>,
    /// The text both sides last agreed on: what the extension last set, or what was last sent.
    /// Edits that leave the text equal to it (including our own `set_text`) are not sent.
    last_known: String,
    /// `SurfaceState::input_set_count` as of the last value written into the editor.
    sets_seen: u64,
    placeholder: String,
    debounce: Option<Task<()>>,
    _subscription: Subscription,
}

/// A view of one surface. Panels and status items wrap it; several views (one per window) can
/// show the same `SurfaceState`.
pub struct SurfaceView {
    state: Entity<SurfaceState>,
    compact: bool,
    pub(crate) inputs: HashMap<NodeId, InputEditor>,
    markdown: HashMap<NodeId, Entity<Markdown>>,
    pub(crate) scroll_handles: HashMap<NodeId, UniformListScrollHandle>,
    /// The `selected` row each list was last scrolled to.
    scrolled_selection: HashMap<NodeId, u32>,
    /// A read-only diff editor per `diff-view` node.
    pub(crate) diffs: HashMap<NodeId, DiffEditor>,
    /// The view's own focus target: the panel focuses it on activation and Tab starts from it.
    pub(crate) focus_handle: FocusHandle,
    /// Every button of the surface (a disabled one keeps focus it already has).
    pub(crate) button_focus: HashMap<NodeId, FocusHandle>,
    pub(crate) list_focus: HashMap<NodeId, FocusHandle>,
    /// The keyboard selection of each list or table, per window.
    pub(crate) selection: HashMap<NodeId, u32>,
    /// Enabled buttons, inputs, lists and tables in tree order: the Tab order.
    pub(crate) focus_order: Vec<NodeId>,
    /// The host's own buttons around the surface; they follow `focus_order` in the Tab order.
    pub(crate) host_focus: HashMap<HostControl, FocusHandle>,
    /// The panel body scrolls when the tree is taller than the panel.
    pub(crate) body_scroll: ScrollHandle,
    synced_revision: Option<u64>,
    /// The props of each input, markdown and diff node at the last sync. The tree shares them
    /// (`TreeNode::props_arc`), so an unchanged node is the same pointer and is skipped without
    /// copying or comparing its text (triage-3, PERF 12).
    synced_props: HashMap<NodeId, Arc<Props>>,
    /// The state's `mount_count` at the last sync.
    synced_mount: u64,
    _observe_state: Subscription,
}

impl SurfaceView {
    /// `compact` lays the roots out in a row without filling the parent, for the status bar.
    pub fn new(
        state: Entity<SurfaceState>,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe_state = cx.observe_in(&state, window, |this, _, window, cx| {
            this.sync(window, cx);
            cx.notify();
        });
        let extension_id = state.read(cx).extension_id().to_string();
        let me = cx.entity().downgrade();
        let views = cx.default_global::<ExtensionViews>();
        let siblings = views.0.entry(extension_id).or_default();
        siblings.retain(|view| view.upgrade().is_some());
        siblings.push(me);
        let mut this = Self {
            state,
            compact,
            inputs: HashMap::default(),
            markdown: HashMap::default(),
            scroll_handles: HashMap::default(),
            scrolled_selection: HashMap::default(),
            diffs: HashMap::default(),
            focus_handle: cx.focus_handle(),
            button_focus: HashMap::default(),
            list_focus: HashMap::default(),
            selection: HashMap::default(),
            focus_order: Vec::new(),
            host_focus: HostControl::ALL
                .iter()
                .map(|control| (*control, cx.focus_handle()))
                .collect(),
            body_scroll: ScrollHandle::new(),
            synced_revision: None,
            synced_mount: 0,
            synced_props: HashMap::default(),
            _observe_state: observe_state,
        };
        this.sync(window, cx);
        // Typing still waiting for its pause goes out when the view closes (a panel removed, a
        // window closed), instead of being lost (triage-3, ASYNC 12).
        cx.on_release(|this: &mut Self, cx| {
            let pending: Vec<(NodeId, String)> = this
                .inputs
                .iter()
                .filter(|(_, input)| input.debounce.is_some())
                .map(|(id, input)| {
                    (
                        *id,
                        input.editor.read(cx).text(cx),
                        input.last_known.clone(),
                    )
                })
                .filter(|(_, text, last_known)| text != last_known)
                .map(|(id, text, _)| (id, text))
                .collect();
            if pending.is_empty() {
                return;
            }
            this.state.update(cx, |state, cx| {
                for (id, text) in pending {
                    state.dispatch(UiEvent::InputChanged(id, text), cx);
                }
            });
        })
        .detach();
        this
    }

    pub fn state(&self) -> &Entity<SurfaceState> {
        &self.state
    }

    pub(crate) fn dispatch(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        // Events reach the extension in the order the user made them: typing still waiting for
        // its pause goes first (judge review w3-deep-1, finding 2).
        if !matches!(event, UiEvent::InputChanged(..)) {
            self.flush_pending_inputs(cx);
            self.flush_sibling_views(cx);
        }
        self.state.update(cx, |state, cx| state.dispatch(event, cx));
    }

    /// The same for the extension's other views (its other surfaces, other windows): one
    /// extension, one order of events (integration-3 finding 9).
    fn flush_sibling_views(&mut self, cx: &mut Context<Self>) {
        let extension_id = self.state.read(cx).extension_id().to_string();
        let me = cx.entity_id();
        let siblings: Vec<Entity<SurfaceView>> = cx
            .try_global::<ExtensionViews>()
            .and_then(|views| views.0.get(&extension_id))
            .into_iter()
            .flatten()
            .filter_map(|view| view.upgrade())
            .filter(|view| view.entity_id() != me)
            .collect();
        for view in siblings {
            view.update(cx, |view, cx| view.flush_pending_inputs(cx));
        }
    }

    fn flush_pending_inputs(&mut self, cx: &mut Context<Self>) {
        let pending: Vec<NodeId> = self
            .inputs
            .iter()
            .filter(|(_, input)| input.debounce.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in pending {
            self.flush_input(id, cx);
        }
    }

    /// Brings the per-window objects (editors, markdown models, scroll handles) in line with the
    /// tree. Runs when the state changes, not per frame.
    /// Focuses the first input in tree order, or the view itself when there is none (a modal
    /// opening: Escape must still reach it).
    pub(crate) fn focus_first_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync(window, cx);
        let first = self
            .focus_order
            .iter()
            .find_map(|id| self.inputs.get(id).map(|input| input.editor.clone()));
        match first {
            Some(editor) => editor.focus_handle(cx).focus(window, cx),
            None => self.focus_handle.focus(window, cx),
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.synced_revision == Some(self.state.read(cx).revision()) {
            return;
        }
        let focused_before = self.focused_node(window, cx);
        let old_order = std::mem::take(&mut self.focus_order);
        let state = self.state.read(cx);
        self.synced_revision = Some(state.revision());
        let remounted = self.synced_mount != state.mount_count();
        self.synced_mount = state.mount_count();

        let mut inputs = Vec::new();
        let mut selections: Vec<(NodeId, u32)> = Vec::new();
        let mut markdown = Vec::new();
        let mut diffs = Vec::new();
        let mut lists = Vec::new();
        let mut buttons = Vec::new();
        let mut focus_order = Vec::new();
        if let Some(tree) = state.tree() {
            // Pre-order, children in order: the Tab order is the reading order.
            let mut stack: Vec<NodeId> = tree.roots().iter().rev().copied().collect();
            while let Some(id) = stack.pop() {
                let Some(node) = tree.get(id) else {
                    continue;
                };
                match node.props() {
                    Props::Input(_) => {
                        inputs.push((id, node.props_arc().clone()));
                        focus_order.push(id);
                    }
                    Props::Markdown(_) => markdown.push((id, node.props_arc().clone())),
                    Props::List(ListProps {
                        row_count,
                        selected,
                        ..
                    }) => {
                        lists.push((id, *row_count));
                        if let Some(selected) = selected {
                            selections.push((id, *selected));
                        }
                        focus_order.push(id);
                    }
                    Props::Table(TableProps { row_count, .. }) => {
                        lists.push((id, *row_count));
                        focus_order.push(id);
                    }
                    Props::Button(props) => {
                        buttons.push(id);
                        if !props.disabled {
                            focus_order.push(id);
                        }
                    }
                    Props::DiffView(_) => diffs.push((id, node.props_arc().clone())),
                    // A canvas takes focus with plan 4.14, when it is clickable.
                    Props::Flex(_)
                    | Props::Text(_)
                    | Props::Icon(_)
                    | Props::Image(_)
                    | Props::Canvas(_) => {}
                }
                stack.extend(node.children().iter().rev());
            }
        }
        self.focus_order = focus_order;

        // Sets for the membership checks below: with lists they were quadratic in the number of
        // nodes (triage-3, PERF 5).
        let button_ids: HashSet<NodeId> = buttons.iter().copied().collect();
        let list_ids: HashSet<NodeId> = lists.iter().map(|(id, _)| *id).collect();
        let input_ids: HashSet<NodeId> = inputs.iter().map(|(id, _)| *id).collect();
        let markdown_ids: HashSet<NodeId> = markdown.iter().map(|(id, _)| *id).collect();
        let row_counts: HashMap<NodeId, u32> = lists.iter().copied().collect();

        self.button_focus.retain(|id, _| button_ids.contains(id));
        for id in buttons {
            self.button_focus
                .entry(id)
                .or_insert_with(|| cx.focus_handle());
        }
        self.list_focus.retain(|id, _| list_ids.contains(id));
        self.selection.retain(|id, selected| {
            row_counts
                .get(id)
                .is_some_and(|row_count| *selected < *row_count)
        });
        for (id, _) in &lists {
            self.list_focus
                .entry(*id)
                .or_insert_with(|| cx.focus_handle());
        }
        let lists: Vec<NodeId> = lists.into_iter().map(|(id, _)| id).collect();

        // An SDK remount gives every node a new id, so an input with typing the extension has not
        // seen (or with focus) would lose it with its old editor. Carry it to the input at the
        // same position among the surface's inputs (integration-3 finding 10). Only after a mount
        // the host started: when the extension replaces or rebuilds its own tree, a new empty
        // input is what it means, such as a form cleared after a submit (integration-4 finding 8).
        let old_inputs: Vec<NodeId> = old_order
            .iter()
            .filter(|id| self.inputs.contains_key(id))
            .copied()
            .collect();
        let new_inputs: Vec<NodeId> = inputs.iter().map(|(id, _)| *id).collect();
        let old_input_ids: HashSet<NodeId> = old_inputs.iter().copied().collect();
        let carried: Vec<(usize, String, bool)> = old_inputs
            .iter()
            .enumerate()
            .filter(|_| remounted)
            .filter(|(_, id)| !input_ids.contains(id))
            .filter_map(|(position, id)| {
                let input = self.inputs.get(id)?;
                let focused = focused_before == Some(*id);
                (input.debounce.is_some() || focused)
                    .then(|| (position, input.editor.read(cx).text(cx), focused))
            })
            .collect();

        self.inputs.retain(|id, _| input_ids.contains(id));
        let inputs_synced = inputs.clone();
        for (id, props) in inputs {
            let sets = self.state.read(cx).input_set_count(id);
            let unchanged = self
                .synced_props
                .get(&id)
                .is_some_and(|synced| Arc::ptr_eq(synced, &props))
                && self
                    .inputs
                    .get(&id)
                    .is_some_and(|input| input.sets_seen == sets);
            if !unchanged && let Props::Input(input_props) = &*props {
                self.sync_input(id, input_props.clone(), window, cx);
            }
        }
        for (position, text, focused) in carried {
            let Some(&id) = new_inputs.get(position) else {
                continue;
            };
            if old_input_ids.contains(&id) {
                continue;
            }
            let Some(input) = self.inputs.get_mut(&id) else {
                continue;
            };
            let editor = input.editor.clone();
            if editor.read(cx).text(cx) != text {
                input.last_known = text.clone();
                editor.update(cx, |editor, cx| editor.set_text(text.as_str(), window, cx));
                self.dispatch(UiEvent::InputChanged(id, text), cx);
            }
            if focused {
                editor.focus_handle(cx).focus(window, cx);
            }
        }

        {
            let diff_props: HashMap<NodeId, &Arc<Props>> =
                diffs.iter().map(|(id, props)| (*id, props)).collect();
            let synced = &self.synced_props;
            self.diffs.retain(|id, diff| {
                diff_props.get(id).is_some_and(|props| {
                    synced.get(id).is_some_and(|old| Arc::ptr_eq(old, props))
                        || matches!(&***props, Props::DiffView(props) if *props == diff.props)
                })
            });
        }
        for (id, props) in &diffs {
            if let collections::hash_map::Entry::Vacant(entry) = self.diffs.entry(*id)
                && let Props::DiffView(diff_props) = &**props
            {
                entry.insert(DiffEditor::new(diff_props.clone(), window, cx));
            }
        }

        self.markdown.retain(|id, _| markdown_ids.contains(id));
        for (id, props) in &markdown {
            let id = *id;
            if self.markdown.contains_key(&id)
                && self
                    .synced_props
                    .get(&id)
                    .is_some_and(|synced| Arc::ptr_eq(synced, props))
            {
                continue;
            }
            let Props::Markdown(source) = &**props else {
                continue;
            };
            let source = without_images(source);
            match self.markdown.get(&id) {
                Some(entity) => {
                    if entity.read(cx).source().as_ref() != source {
                        entity.update(cx, |entity, cx| entity.replace(source, cx));
                    }
                }
                None => {
                    let entity = cx.new(|cx| Markdown::new(source.into(), None, None, cx));
                    self.markdown.insert(id, entity);
                }
            }
        }

        self.synced_props = inputs_synced
            .into_iter()
            .chain(markdown)
            .chain(diffs)
            .collect();

        self.scroll_handles.retain(|id, _| list_ids.contains(id));
        for id in &lists {
            self.scroll_handles.entry(*id).or_default();
        }
        // A `selected` the extension (or the keyboard) moved is scrolled into view (design C).
        self.scrolled_selection
            .retain(|id, _| list_ids.contains(id));
        for (id, selected) in selections {
            if self.scrolled_selection.insert(id, selected) != Some(selected)
                && let Some(handle) = self.scroll_handles.get(&id)
            {
                handle.scroll_to_item(selected as usize, gpui::ScrollStrategy::Nearest);
            }
        }
        self.restore_focus(focused_before, &old_order, window, cx);
    }

    fn sync_input(
        &mut self,
        id: NodeId,
        props: InputProps,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.inputs.get_mut(&id) else {
            let editor = cx.new(|cx| {
                let mut editor = Editor::single_line(window, cx);
                editor.set_placeholder_text(&props.placeholder, window, cx);
                editor.set_text(props.value.as_str(), window, cx);
                // Design K: a secret input is masked.
                editor.set_masked(props.secret, cx);
                // Triage-3 #5: a blinking caret repaints the whole surface twice a second.
                editor.disable_cursor_blink(cx);
                editor
            });
            let sets_seen = self.state.read(cx).input_set_count(id);
            let subscription = cx.subscribe_in(
                &editor,
                window,
                move |this, _, event: &EditorEvent, _, cx| {
                    if matches!(event, EditorEvent::BufferEdited) {
                        this.input_edited(id, cx);
                    }
                },
            );
            self.inputs.insert(
                id,
                InputEditor {
                    editor,
                    last_known: props.value,
                    sets_seen,
                    placeholder: props.placeholder,
                    debounce: None,
                    _subscription: subscription,
                },
            );
            return;
        };

        input
            .editor
            .update(cx, |editor, cx| editor.set_masked(props.secret, cx));
        if input.placeholder != props.placeholder {
            input.editor.update(cx, |editor, cx| {
                editor.set_placeholder_text(&props.placeholder, window, cx)
            });
            input.placeholder = props.placeholder;
        }

        // Integrator decision 6: a value the extension sets is written into the editor whenever it
        // differs from the editor's text, focused or not. Without a new set the user's text stands
        // (the host tree already follows it, see `SurfaceState::set_input_locally`).
        let sets = self.state.read(cx).input_set_count(id);
        if sets == input.sets_seen {
            return;
        }
        input.sets_seen = sets;
        if input.debounce.is_some() {
            // The user typed in the last moments and the extension has not seen it yet (a
            // remount or `replace` sets every input): send the typing now and keep it on screen
            // (judge review w3-deep-1, finding 8).
            self.flush_input(id, cx);
            return;
        }
        let text = input.editor.read(cx).text(cx);
        if text != props.value {
            input.debounce = None;
            input.last_known = props.value.clone();
            input
                .editor
                .update(cx, |editor, cx| editor.set_text(props.value, window, cx));
        }
    }

    fn input_edited(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(input) = self.inputs.get_mut(&id) else {
            return;
        };
        if input.editor.read(cx).text(cx) == input.last_known {
            input.debounce = None;
            return;
        }
        input.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TYPING_DEBOUNCE).await;
            this.update(cx, |this, cx| this.flush_input(id, cx)).ok();
        }));
    }

    fn flush_input(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(input) = self.inputs.get_mut(&id) else {
            return;
        };
        input.debounce = None;
        let text = input.editor.read(cx).text(cx);
        if text == input.last_known {
            return;
        }
        input.last_known = text.clone();
        self.dispatch(UiEvent::InputChanged(id, text), cx);
    }

    fn submit_input(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(input) = self.inputs.get_mut(&id) else {
            return;
        };
        // The submit carries the text, so a pending `input-changed` would only be a second call.
        input.debounce = None;
        let text = input.editor.read(cx).text(cx);
        input.last_known = text.clone();
        self.dispatch(UiEvent::InputSubmitted(id, text), cx);
    }

    fn render_rows(
        &mut self,
        list: NodeId,
        row_count: u32,
        row_height: u16,
        table_columns: Option<usize>,
        visible: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let fetch = fetch_range(visible.clone(), row_count);
        self.state
            .update(cx, |state, cx| state.request_rows(list, fetch, cx));
        let state = self.state.read(cx);
        let renderer = Renderer {
            view: self,
            state,
            window,
            cx,
        };
        visible
            .map(|index| renderer.row(list, index as u32, row_height, table_columns))
            .collect()
    }
}

/// The host's own buttons around a surface: what a failed surface offers and the error footer's.
/// They come after the surface's nodes in the Tab order, so a keyboard user can reach them
/// (triage-3, UX F6, F21).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum HostControl {
    Retry,
    Restart,
    CopyError,
    ReloadRetry,
    DismissError,
}

impl HostControl {
    const ALL: [Self; 5] = [
        Self::Retry,
        Self::Restart,
        Self::CopyError,
        Self::ReloadRetry,
        Self::DismissError,
    ];
}

impl SurfaceView {
    /// The host controls shown now, in the order they are drawn.
    pub(crate) fn host_controls(&self, cx: &App) -> Vec<HostControl> {
        let state = self.state.read(cx);
        let mut controls = Vec::new();
        if let Phase::Failed { retryable, .. } = state.phase() {
            if *retryable {
                controls.push(HostControl::Retry);
            } else if state.can_restart() {
                controls.push(HostControl::Restart);
            }
            if state.failure_detail().is_some() {
                controls.push(HostControl::CopyError);
            }
        }
        if !self.compact && state.last_error().is_some() {
            if state.reload_failed() && state.can_restart() {
                controls.push(HostControl::ReloadRetry);
            }
            controls.push(HostControl::DismissError);
        }
        controls
    }

    pub(crate) fn run_host_control(
        &mut self,
        control: HostControl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match control {
            HostControl::Retry => self.state.update(cx, |state, cx| state.retry(cx)),
            HostControl::Restart | HostControl::ReloadRetry => self
                .state
                .update(cx, |state, cx| state.restart_extension(cx)),
            HostControl::CopyError => {
                if let Some(detail) = self.state.read(cx).failure_detail() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(detail.to_string()));
                }
            }
            HostControl::DismissError => self.state.update(cx, |state, cx| state.dismiss_error(cx)),
        }
        // A control that went away with what it did leaves focus with the surface, so Tab and
        // Escape keep working.
        if self.host_focus[&control].is_focused(window)
            && !self.host_controls(cx).contains(&control)
        {
            self.focus_handle.focus(window, cx);
        }
    }

    /// A host control: focusable, in the button key context (Space and Enter press it), with a
    /// focus ring.
    fn host_button(
        &self,
        control: HostControl,
        button: AnyElement,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let focus = &self.host_focus[&control];
        div()
            .id(("fyx-ui-host-control", control as usize))
            .key_context(crate::keyboard::BUTTON_CONTEXT)
            .track_focus(focus)
            .rounded_sm()
            .border_1()
            .border_color(focus_ring(focus.is_focused(window), cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.run_host_control(control, window, cx);
            }))
            .child(button)
            .into_any_element()
    }
}

impl Render for SurfaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let compact = self.compact;
        let container = if compact {
            h_flex().gap_1()
        } else {
            v_flex().size_full().min_h_0()
        };
        let body = match state.phase() {
            Phase::Closed => div().into_any_element(),
            Phase::Mounting => Label::new("Loading…")
                .color(Color::Muted)
                .size(LabelSize::Small)
                .into_any_element(),
            Phase::Failed { message, retryable } => h_flex()
                .gap_1()
                .when(!compact, |this| this.p_2().flex_wrap())
                .child(
                    Label::new(message.clone())
                        .color(Color::Error)
                        .size(LabelSize::Small),
                )
                // A dead instance refuses every call, so a retry could only fail again: it gets a
                // fresh instance instead.
                .when(*retryable, |this| {
                    this.child(self.host_button(
                        HostControl::Retry,
                        Button::new("fyx-ui-retry", "Retry").into_any_element(),
                        window,
                        cx,
                    ))
                })
                .when(!*retryable && state.can_restart(), |this| {
                    this.child(self.host_button(
                        HostControl::Restart,
                        Button::new("fyx-ui-restart", "Restart extension").into_any_element(),
                        window,
                        cx,
                    ))
                })
                .when(state.failure_detail().is_some(), |this| {
                    this.child(self.host_button(
                        HostControl::CopyError,
                        Button::new("fyx-ui-copy-error", "Copy Error Details").into_any_element(),
                        window,
                        cx,
                    ))
                })
                .into_any_element(),
            Phase::Ready(tree) => {
                let renderer = Renderer {
                    view: self,
                    state,
                    window,
                    cx,
                };
                let children = tree
                    .roots()
                    .iter()
                    .map(|&id| renderer.node(tree, id, Scope::Surface));
                if compact {
                    h_flex().gap_1().children(children).into_any_element()
                } else {
                    // The content is at least as tall as the panel (so `grow` nodes and lists fill
                    // it) and scrolls when it is taller.
                    WithVerticalScrollbar {
                        id: "fyx-ui-body".into(),
                        handle: ScrollbarHandle::Body(self.body_scroll.clone()),
                        min_height: None,
                        focus: None,
                        content: div()
                            .id("fyx-ui-body-content")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.body_scroll)
                            .child(v_flex().w_full().min_h_full().children(children))
                            .into_any_element(),
                    }
                    .into_any_element()
                }
            }
        };
        let reload_failed = state.reload_failed() && state.can_restart();
        let error = state.last_error().cloned().map(|error| {
            h_flex()
                .id("fyx-ui-error")
                .px_2()
                .py_1()
                .gap_1()
                .border_t_1()
                .border_color(cx.theme().colors().border)
                .child(
                    Icon::new(IconName::Warning)
                        .size(IconSize::Small)
                        .color(Color::Error),
                )
                .child(
                    Label::new(error)
                        .size(LabelSize::Small)
                        .color(Color::Error)
                        .truncate(),
                )
                .when(reload_failed, |this| {
                    this.child(self.host_button(
                        HostControl::ReloadRetry,
                        Button::new("fyx-ui-reload-retry", "Retry").into_any_element(),
                        window,
                        cx,
                    ))
                })
                .child(
                    self.host_button(
                        HostControl::DismissError,
                        IconButton::new("fyx-ui-dismiss-error", IconName::Close)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Dismiss"))
                            .into_any_element(),
                        window,
                        cx,
                    ),
                )
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.state.update(cx, |state, cx| state.dismiss_error(cx));
                }))
        });
        container
            .key_context(crate::keyboard::SURFACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_previous))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::cancel_input))
            .child(body)
            .when(!compact, |this| this.children(error))
    }
}

enum ScrollbarHandle {
    List(UniformListScrollHandle),
    Body(ScrollHandle),
}

/// A scrolling element with the `ui` crate's vertical scrollbar, as Zed's panels have. A
/// component, because the scrollbar needs `&mut Window` and `&mut App` while the tree renderer
/// only borrows them.
#[derive(IntoElement)]
struct WithVerticalScrollbar {
    id: ElementId,
    handle: ScrollbarHandle,
    /// `Some` for a list: it fills the space its parent gives (`flex_1`), at least this tall. The
    /// panel body fills the panel.
    min_height: Option<Pixels>,
    /// A list's focus target: keyboard selection, and a focus ring while focused.
    focus: Option<FocusHandle>,
    content: AnyElement,
}

impl RenderOnce for WithVerticalScrollbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // A flex column, so the content's `flex_1` gets a definite height (a percentage height
        // would resolve to nothing inside a flexed block).
        let container = v_flex()
            .id(self.id)
            .flex_1()
            .w_full()
            .min_h(self.min_height.unwrap_or(Pixels::ZERO))
            .when_some(self.focus, |this, focus| {
                let ring = focus_ring(focus.is_focused(window), cx);
                this.key_context(crate::keyboard::LIST_CONTEXT)
                    .track_focus(&focus)
                    .rounded_sm()
                    .border_1()
                    .border_color(ring)
            })
            .child(self.content);
        match &self.handle {
            ScrollbarHandle::List(handle) => container.vertical_scrollbar_for(handle, window, cx),
            ScrollbarHandle::Body(handle) => container.vertical_scrollbar_for(handle, window, cx),
        }
    }
}

/// Where a node lives. Row subtrees use row-local ids (W4: from `0x8000_0000`), so their element
/// ids and events are scoped by `(list node, row index)`.
#[derive(Clone, Copy)]
enum Scope {
    Surface,
    Row { list: NodeId, index: u32 },
}

impl Scope {
    fn element_id(self, id: NodeId) -> ElementId {
        match self {
            Scope::Surface => ElementId::NamedInteger("fyx-node".into(), id as u64),
            Scope::Row { list, index } => ElementId::NamedInteger(
                SharedString::from(format!("fyx-row-{list}-{index}")),
                id as u64,
            ),
        }
    }
}

struct Renderer<'a, 'b> {
    view: &'a SurfaceView,
    state: &'a SurfaceState,
    window: &'a Window,
    cx: &'a Context<'b, SurfaceView>,
}

impl Renderer<'_, '_> {
    fn node(&self, tree: &Tree, id: NodeId, scope: Scope) -> AnyElement {
        let Some(node) = tree.get(id) else {
            return div().into_any_element();
        };
        let element = match node.props() {
            Props::Flex(props) => self.flex(tree, props, node.children(), scope),
            Props::Text(props) => self.text(props),
            Props::Button(props) => self.button(id, props, scope),
            Props::Input(props) => self.input(id, props, scope),
            Props::List(props) => self.list(id, props, scope),
            Props::Table(props) => self.table(id, props, scope),
            Props::Icon(props) => self.icon(props),
            Props::Markdown(source) => self.markdown(id, source, scope),
            Props::DiffView(props) => self.diff_view(id, props, scope),
            Props::Image(props) => self.image_placeholder(props),
            Props::Canvas(props) => self.canvas_placeholder(props),
        };
        // Only leaves: a wrapper around a flex or list node would change the layout in tests.
        let leaf = !matches!(
            node.props(),
            Props::Flex(_) | Props::List(_) | Props::Table(_)
        );
        with_test_selector(element, id, scope, leaf)
    }

    fn flex(
        &self,
        tree: &Tree,
        props: &FlexProps,
        children: &[NodeId],
        scope: Scope,
    ) -> AnyElement {
        let element = match props.direction {
            FlexDirection::Row => h_flex(),
            FlexDirection::Column => v_flex(),
        };
        let element = match props.align {
            Align::Start => element.items_start(),
            Align::Center => element.items_center(),
            Align::End => element.items_end(),
            Align::Stretch => element.items_stretch(),
        };
        element
            .gap(px(props.gap.into()))
            .p(px(props.padding.into()))
            .min_w_0()
            .when(props.grow, |this| this.flex_grow_1())
            .when_some(props.background, |this, token| {
                this.bg(token_hsla(token, self.cx))
            })
            .children(children.iter().map(|&child| self.node(tree, child, scope)))
            .into_any_element()
    }

    fn text(&self, props: &TextProps) -> AnyElement {
        Label::new(props.content.clone())
            .when_some(props.color, |this, token| {
                this.color(text_color(token, self.cx))
            })
            .when(props.bold, |this| this.weight(FontWeight::BOLD))
            .when(props.monospace, |this| this.buffer_font(self.cx))
            .into_any_element()
    }

    fn button(&self, id: NodeId, props: &ButtonProps, scope: Scope) -> AnyElement {
        let element_id = scope.element_id(id);
        let icon = props
            .icon
            .as_ref()
            .map(|icon| crate::icons::resolve_icon(icon).static_name());
        let style = button_style(props.style);
        // Design J: at most 256 bytes, cut at a character boundary by the model.
        let tooltip: Option<SharedString> = props
            .tooltip_text()
            .map(|text| SharedString::from(text.to_string()));
        // Inside a row the whole row is the click target (`row-click`): row-local ids are not
        // surface ids, so a `click` on them would be ambiguous.
        let on_click = match scope {
            Scope::Surface => Some(self.cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.dispatch(UiEvent::Click(id), cx)
            })),
            Scope::Row { .. } => None,
        };
        let focus = match scope {
            Scope::Surface => self.view.button_focus.get(&id),
            Scope::Row { .. } => None,
        };
        let button = if props.label.is_empty()
            && let Some(icon) = icon
        {
            // F16: an icon-only button is named by its tooltip, else by its icon.
            let name: SharedString = tooltip
                .clone()
                .unwrap_or_else(|| format!("{icon:?}").into());
            IconButton::new(element_id, icon)
                .icon_size(IconSize::Small)
                .style(style)
                .disabled(props.disabled)
                .aria_label(name)
                .when_some(tooltip, |this, text| this.tooltip(Tooltip::text(text)))
                .when_some(focus, |this, focus| this.track_focus(focus))
                .when_some(on_click, |this, on_click| this.on_click(on_click))
                .into_any_element()
        } else {
            Button::new(element_id, props.label.clone())
                .style(style)
                .disabled(props.disabled)
                .when_some(tooltip, |this, text| this.tooltip(Tooltip::text(text)))
                .when_some(icon, |this, icon| {
                    this.start_icon(Icon::new(icon).size(IconSize::Small))
                })
                .when_some(focus, |this, focus| this.track_focus(focus))
                .when_some(on_click, |this, on_click| this.on_click(on_click))
                .into_any_element()
        };
        let Some(focus) = focus else {
            return button;
        };
        div()
            .key_context(crate::keyboard::BUTTON_CONTEXT)
            .rounded_sm()
            .border_1()
            .border_color(focus_ring(focus.is_focused(self.window), self.cx))
            .child(button)
            .into_any_element()
    }

    fn input(&self, id: NodeId, props: &InputProps, scope: Scope) -> AnyElement {
        let editor = match scope {
            Scope::Surface => self.view.inputs.get(&id).map(|input| input.editor.clone()),
            Scope::Row { .. } => None,
        };
        let Some(editor) = editor else {
            // Rows are read-only snapshots: show the value.
            let (text, color) = if props.value.is_empty() {
                (props.placeholder.clone(), Color::Placeholder)
            } else {
                (props.value.clone(), Color::Default)
            };
            return Label::new(text).color(color).into_any_element();
        };
        let colors = self.cx.theme().colors();
        // F16: the input's accessible name is its `label`, else its placeholder.
        let name: SharedString = props
            .label_text()
            .unwrap_or(&props.placeholder)
            .to_string()
            .into();
        div()
            .id(scope.element_id(id))
            .aria_label(name)
            .w_full()
            .min_w_0()
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(
                if editor
                    .focus_handle(self.cx)
                    .contains_focused(self.window, self.cx)
                {
                    colors.border_focused
                } else {
                    colors.border
                },
            )
            .bg(colors.editor_background)
            .on_action(
                self.cx
                    .listener(move |this, _: &menu::Confirm, _, cx| this.submit_input(id, cx)),
            )
            .child(editor)
            .into_any_element()
    }

    fn list(&self, id: NodeId, props: &ListProps, scope: Scope) -> AnyElement {
        self.virtualized(id, props.row_count, props.row_height, None, scope)
    }

    fn table(&self, id: NodeId, props: &TableProps, scope: Scope) -> AnyElement {
        let columns = props.columns.len().max(1);
        let header = h_flex()
            .w_full()
            .px_1()
            .border_b_1()
            .border_color(self.cx.theme().colors().border)
            .children(props.columns.iter().map(|column| {
                div().flex_1().min_w_0().child(
                    Label::new(column.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted)
                        .weight(FontWeight::SEMIBOLD)
                        .truncate(),
                )
            }));
        v_flex()
            .w_full()
            .flex_grow_1()
            .child(header)
            .child(self.virtualized(
                id,
                props.row_count,
                DEFAULT_ROW_HEIGHT,
                Some(columns),
                scope,
            ))
            .into_any_element()
    }

    fn virtualized(
        &self,
        id: NodeId,
        row_count: u32,
        row_height: u16,
        table_columns: Option<usize>,
        scope: Scope,
    ) -> AnyElement {
        if let Scope::Row { list, index } = scope {
            // An authoring mistake: logged for the author, nothing drawn for the user.
            log::debug!(
                "fyx:ui {}: list {id} inside row {index} of list {list} is not drawn (lists \
                 inside list rows are not supported)",
                self.state.extension_id()
            );
            return div().into_any_element();
        }
        let row_height = if row_height == 0 {
            DEFAULT_ROW_HEIGHT
        } else {
            row_height
        };
        let min_height = row_height as u32 * row_count.min(MIN_VISIBLE_ROWS);
        let list = uniform_list(
            scope.element_id(id),
            row_count as usize,
            self.cx.processor(move |this, visible, window, cx| {
                this.render_rows(
                    id,
                    row_count,
                    row_height,
                    table_columns,
                    visible,
                    window,
                    cx,
                )
            }),
        )
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .flex_1()
        .w_full();
        let Some(handle) = self.view.scroll_handles.get(&id) else {
            return list.min_h(px(min_height as f32)).into_any_element();
        };
        // `uniform_list` measures one item each layout; measuring the top visible one keeps that
        // request inside the visible range (judge review w3-deep-1, finding 6).
        let scrolled = -handle.0.borrow().base_handle.offset().y;
        let top = (scrolled / px(row_height.into())).max(0.) as u32;
        let list = list
            .with_width_from_item(Some(top.min(row_count.saturating_sub(1)) as usize))
            .track_scroll(handle);
        WithVerticalScrollbar {
            id: ElementId::NamedInteger("fyx-list-scroll".into(), id as u64),
            handle: ScrollbarHandle::List(handle.clone()),
            min_height: Some(px(min_height as f32)),
            focus: self.view.list_focus.get(&id).cloned(),
            content: list.into_any_element(),
        }
        .into_any_element()
    }

    fn row(
        &self,
        list: NodeId,
        index: u32,
        row_height: u16,
        table_columns: Option<usize>,
    ) -> AnyElement {
        let scope = Scope::Row { list, index };
        // `selected` (0.2) is the extension's highlighted row; the view's own keyboard selection
        // is drawn the same way.
        let extension_selected =
            self.state
                .tree()
                .and_then(|tree| tree.get(list))
                .and_then(|node| match node.props() {
                    Props::List(props) => props.selected,
                    _ => None,
                });
        let selected =
            extension_selected == Some(index) || self.view.selection.get(&list) == Some(&index);
        // Design J: nesting depth, clamped by the host; 0 for 0.1 rows.
        let indent = self.state.row_indent(list, index);
        let row = div()
            .id(ElementId::NamedInteger("fyx-row".into(), index as u64))
            // Tests find rows by `fyx-row-<list>-<index>`; a no-op outside test builds.
            .debug_selector(|| format!("fyx-row-{list}-{index}"))
            .h(px(row_height.into()))
            .w_full()
            .px_1()
            .when(indent > 0, |this| {
                this.pl(px(4. + f32::from(indent) * ROW_INDENT_PX))
            })
            .flex()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|style| style.bg(self.cx.theme().colors().ghost_element_hover))
            .when(selected, |this| {
                this.bg(self.cx.theme().colors().element_selected)
            })
            .on_click(self.cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.selection.insert(list, index);
                if let Some(focus) = this.list_focus.get(&list) {
                    focus.focus(window, cx);
                }
                this.dispatch(UiEvent::RowClick(list, index), cx)
            }));
        let Some(tree) = self.state.row(list, index) else {
            // Not fetched yet: an empty row of the right height.
            return row.into_any_element();
        };
        let Some(&root) = tree.roots().first() else {
            return row.into_any_element();
        };
        match (table_columns, tree.get(root).map(|node| node.props())) {
            (Some(_), Some(Props::Flex(_))) => {
                let cells = tree.children(Some(root)).iter().map(|&cell| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(self.node(tree, cell, scope))
                });
                row.child(h_flex().w_full().children(cells))
                    .into_any_element()
            }
            _ => row.child(self.node(tree, root, scope)).into_any_element(),
        }
    }

    /// A 0.3 `image` node as the muted box with its alt text that design.md §12 draws
    /// while an image loads or when it fails. Decoding comes with plan 4.11.
    fn image_placeholder(&self, props: &ui_ext::image::ImageProps) -> AnyElement {
        div()
            .when_some(props.width, |this, width| this.w(px(width as f32)))
            .when_some(props.height, |this, height| this.h(px(height as f32)))
            .max_w_full()
            .min_h(px(20.))
            .p_1()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded_sm()
            .border_1()
            .border_color(self.cx.theme().colors().border_variant)
            .bg(self.cx.theme().colors().element_background)
            .child(
                Label::new(props.alt.clone())
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .truncate(),
            )
            .into_any_element()
    }

    /// Until plan 4.14 paints the ops: the canvas's box with its description.
    fn canvas_placeholder(&self, props: &ui_ext::canvas::CanvasProps) -> AnyElement {
        div()
            .w(px(props.width as f32))
            .h(px(props.height as f32))
            .max_w_full()
            .p_1()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded_sm()
            .border_1()
            .border_color(self.cx.theme().colors().border_variant)
            .bg(self.cx.theme().colors().element_background)
            .child(
                Label::new(props.description.clone())
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .truncate(),
            )
            .into_any_element()
    }

    fn icon(&self, props: &IconProps) -> AnyElement {
        use crate::icons::ResolvedIcon;
        let icon = |name: IconName| {
            Icon::new(name)
                .size(IconSize::Small)
                .when_some(props.color, |this, token| {
                    this.color(text_color(token, self.cx))
                })
        };
        match crate::icons::resolve_icon(&props.name) {
            ResolvedIcon::Named(name) => icon(name).into_any_element(),
            ResolvedIcon::Spinner => icon(IconName::LoadCircle)
                .with_rotate_animation(2)
                .into_any_element(),
            ResolvedIcon::Blank => div()
                .size(IconSize::Small.rems())
                .flex_none()
                .into_any_element(),
        }
    }

    fn markdown(&self, id: NodeId, source: &str, scope: Scope) -> AnyElement {
        let entity = match scope {
            Scope::Surface => self.view.markdown.get(&id).cloned(),
            Scope::Row { .. } => None,
        };
        match entity {
            Some(entity) => div()
                .w_full()
                .min_w_0()
                .child(
                    MarkdownElement::new(
                        entity,
                        MarkdownStyle::themed(MarkdownFont::Agent, self.window, self.cx),
                    )
                    .on_url_click(open_extension_link),
                )
                .into_any_element(),
            None => Label::new(source.to_string()).into_any_element(),
        }
    }

    /// Placeholder until the host diff component lands: unchanged leading and trailing lines as
    /// context, the middle as removed and added lines.
    /// Zed's diff editor, read-only, every hunk expanded; inside list rows (read-only snapshots)
    /// a light line diff instead.
    fn diff_view(&self, id: NodeId, props: &DiffViewProps, scope: Scope) -> AnyElement {
        if let Scope::Surface = scope
            && let Some(diff) = self.view.diffs.get(&id)
        {
            let colors = self.cx.theme().colors();
            return div()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .rounded_sm()
                .border_1()
                .border_color(colors.border)
                .bg(colors.editor_background)
                .child(diff.editor.clone())
                .into_any_element();
        }
        self.line_diff(props)
    }

    fn line_diff(&self, props: &DiffViewProps) -> AnyElement {
        let status = self.cx.theme().status();
        let lines = diff_lines(&props.old_text, &props.new_text);
        v_flex()
            .w_full()
            .min_w_0()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(self.cx.theme().colors().border)
            .bg(self.cx.theme().colors().editor_background)
            .children(lines.into_iter().map(|(kind, line)| {
                let (prefix, background, color) = match kind {
                    DiffLine::Context => ("  ", None, Color::Default),
                    DiffLine::Removed => ("- ", Some(status.deleted_background), Color::Deleted),
                    DiffLine::Added => ("+ ", Some(status.created_background), Color::Created),
                };
                div()
                    .w_full()
                    .px_2()
                    .when_some(background, |this, background| this.bg(background))
                    .child(
                        Label::new(format!("{prefix}{line}"))
                            .buffer_font(self.cx)
                            .size(LabelSize::Small)
                            .color(color)
                            .single_line(),
                    )
            }))
            .into_any_element()
    }
}

/// Tests find surface nodes by `fyx-node-<id>` (`VisualTestContext::debug_bounds`).
#[cfg(test)]
fn with_test_selector(element: AnyElement, id: NodeId, scope: Scope, leaf: bool) -> AnyElement {
    match scope {
        Scope::Surface if leaf => div()
            .debug_selector(|| format!("fyx-node-{id}"))
            .child(element)
            .into_any_element(),
        Scope::Surface | Scope::Row { .. } => element,
    }
}

#[cfg(not(test))]
fn with_test_selector(element: AnyElement, _: NodeId, _: Scope, _: bool) -> AnyElement {
    element
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLine {
    Context,
    Removed,
    Added,
}

pub(crate) fn diff_lines<'a>(old: &'a str, new: &'a str) -> Vec<(DiffLine, &'a str)> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old
        .iter()
        .zip(&new)
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    let mut lines = Vec::with_capacity(old.len() + new.len() - prefix - suffix);
    lines.extend(old[..prefix].iter().map(|line| (DiffLine::Context, *line)));
    lines.extend(
        old[prefix..old.len() - suffix]
            .iter()
            .map(|line| (DiffLine::Removed, *line)),
    );
    lines.extend(
        new[prefix..new.len() - suffix]
            .iter()
            .map(|line| (DiffLine::Added, *line)),
    );
    lines.extend(
        old[old.len() - suffix..]
            .iter()
            .map(|line| (DiffLine::Context, *line)),
    );
    lines
}

/// The host component behind `diff-view`: `acp_thread::Diff` (the agent panel's and W5's review
/// pane's diff) in a read-only editor sized by its content, every hunk expanded.
pub(crate) struct DiffEditor {
    props: DiffViewProps,
    pub(crate) editor: Entity<Editor>,
    _diff: Entity<acp_thread::Diff>,
}

impl DiffEditor {
    fn new(props: DiffViewProps, window: &mut Window, cx: &mut Context<SurfaceView>) -> Self {
        let languages = workspace::AppState::try_global(cx)
            .map(|app_state| app_state.languages.clone())
            .unwrap_or_else(|| {
                Arc::new(language::LanguageRegistry::new(
                    cx.background_executor().clone(),
                ))
            });
        let path = diff_path(props.language.as_deref(), &languages);
        let diff = cx.new(|_| {
            acp_thread::Diff::finalized(
                path,
                Some(props.old_text.clone()),
                props.new_text.clone(),
                languages,
            )
        });
        let multibuffer = diff.update(cx, |diff, cx| diff.multibuffer(cx));
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(
                EditorMode::Full {
                    scale_ui_elements_with_buffer_font_size: false,
                    show_active_line_background: false,
                    sizing_behavior: SizingBehavior::SizeByContent,
                },
                multibuffer,
                None,
                window,
                cx,
            );
            editor.set_read_only(true);
            editor.disable_diagnostics(cx);
            editor.set_show_gutter(false, cx);
            editor.disable_expand_excerpt_buttons(cx);
            editor.set_show_vertical_scrollbar(false, cx);
            editor.set_minimap_visibility(MinimapVisibility::Disabled, window, cx);
            editor.set_soft_wrap_mode(SoftWrap::None, cx);
            editor.set_forbid_vertical_scroll(true);
            editor.set_show_indent_guides(false, cx);
            editor.set_show_code_actions(false, cx);
            editor.set_show_git_diff_gutter(false, cx);
            editor.set_expand_all_diff_hunks(cx);
            editor.set_diff_hunk_renderer(Some(Arc::new(HiddenUnstagedDiffHunkRenderer)), cx);
            editor
        });
        Self {
            props,
            editor,
            _diff: diff,
        }
    }
}

/// `acp_thread::Diff` picks the language from a file path; `diff-view` names a language, so the
/// path gets that language's first file suffix (`rust` → `diff.rs`).
pub(crate) fn diff_path(
    language: Option<&str>,
    languages: &Arc<language::LanguageRegistry>,
) -> String {
    language
        .and_then(|name| languages.available_language_for_modeline_name(name))
        .and_then(|language| language.matcher().path_suffixes.first().cloned())
        .map_or_else(|| "diff.txt".to_string(), |suffix| format!("diff.{suffix}"))
}

/// Every view of each extension, by extension id, so a view can flush the typing of the others.
#[derive(Default)]
struct ExtensionViews(HashMap<String, Vec<WeakEntity<SurfaceView>>>);

impl Global for ExtensionViews {}

/// The focus ring: the theme's focused border, or a transparent one of the same width so focusing
/// does not move anything.
fn focus_ring(focused: bool, cx: &App) -> Hsla {
    let colors = cx.theme().colors();
    if focused {
        colors.border_focused
    } else {
        colors.border_transparent
    }
}

/// Extension markdown shows no images (triage-3, SEC T8): `markdown` decodes `data:` images
/// in-process while parsing, before anything is drawn. Every `![` that would open one gets a
/// backslash, so the parser sees a `!` and, after it, at most a link (which `link_is_allowed`
/// filters). Inside code the backslash shows; `![` there is rare.
pub(crate) fn without_images(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut backslashes = 0;
    let mut chars = source.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '!' && chars.peek() == Some(&'[') && backslashes % 2 == 0 {
            out.push('\\');
        }
        backslashes = if character == '\\' {
            backslashes + 1
        } else {
            0
        };
        out.push(character);
    }
    out
}

/// Only web links leave an extension's markdown: `http` and `https`. Anything else (`file:`,
/// `javascript:`, `zed:`, custom schemes, relative paths) would hand the OS URL handler a target
/// the extension chose.
pub(crate) fn link_is_allowed(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        && rest.starts_with("//")
        && rest.len() > 2
}

/// A click on a link in an extension's markdown: refused unless it is a web link, and even then
/// opened only after the user confirms the address.
fn open_extension_link(url: SharedString, window: &mut Window, cx: &mut App) {
    if !link_is_allowed(&url) {
        log::warn!("fyx:ui: not opening {url:?} from an extension: only http(s) links open");
        return;
    }
    let answer = window.prompt(
        gpui::PromptLevel::Info,
        "Open this link from an extension?",
        Some(url.as_ref()),
        &["Open", "Cancel"],
        cx,
    );
    cx.spawn(async move |cx| {
        if answer.await == Ok(0) {
            cx.update(|cx| cx.open_url(&url));
        }
    })
    .detach();
}

/// Icon names are snake_case `IconName`s (`plus`, `list_todo`); unknown names get a neutral icon.
/// Pixels per indent level of a list row (design J).
const ROW_INDENT_PX: f32 = 16.;

pub(crate) fn button_style(style: ButtonStyle) -> UiButtonStyle {
    match style {
        ButtonStyle::Default => UiButtonStyle::Outlined,
        ButtonStyle::Filled => UiButtonStyle::Filled,
        ButtonStyle::Subtle => UiButtonStyle::Subtle,
        ButtonStyle::Danger => UiButtonStyle::Tinted(TintColor::Error),
    }
}

/// Text and icon colours: the `ui` crate's semantic colours where one exists, otherwise the
/// theme's own token. Never a raw colour from the extension.
pub(crate) fn text_color(token: ColorToken, cx: &App) -> Color {
    match token {
        ColorToken::Text => Color::Default,
        ColorToken::TextMuted => Color::Muted,
        ColorToken::TextAccent => Color::Accent,
        ColorToken::Error => Color::Error,
        ColorToken::Warning => Color::Warning,
        ColorToken::Success => Color::Success,
        ColorToken::Border | ColorToken::Surface | ColorToken::ElevatedSurface => {
            Color::Custom(token_hsla(token, cx))
        }
    }
}

pub(crate) fn token_hsla(token: ColorToken, cx: &App) -> Hsla {
    let theme = cx.theme();
    let colors = theme.colors();
    let status = theme.status();
    match token {
        ColorToken::Text => colors.text,
        ColorToken::TextMuted => colors.text_muted,
        ColorToken::TextAccent => colors.text_accent,
        ColorToken::Error => status.error,
        ColorToken::Warning => status.warning,
        ColorToken::Success => status.success,
        ColorToken::Border => colors.border,
        ColorToken::Surface => colors.surface_background,
        ColorToken::ElevatedSurface => colors.elevated_surface_background,
    }
}
