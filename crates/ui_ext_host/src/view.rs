//! Draws a retained surface tree with native GPUI and `ui` elements.
//!
//! `render()` only reads `SurfaceState`; it never calls the extension. Clicks and edits are routed
//! by node id to `SurfaceState::dispatch`, which makes the call on a task.

use std::{ops::Range, time::Duration};

use collections::HashMap;
use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, Focusable as _, FontWeight, Hsla,
    IntoElement, ListSizingBehavior, ParentElement, Render, SharedString, Styled, Subscription,
    Task, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use markdown::{Markdown, MarkdownElement, MarkdownFont, MarkdownStyle};
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonStyle as UiButtonStyle, Clickable as _, Color,
    Disableable as _, Icon, IconButton, IconName, IconSize, Label, LabelCommon as _, LabelSize,
    TintColor, h_flex, v_flex,
};
use ui_ext::{
    Align, ButtonProps, ButtonStyle, ColorToken, DiffViewProps, FlexDirection, FlexProps,
    IconProps, InputProps, ListProps, NodeId, Props, TableProps, TextProps, Tree, UiEvent,
};

use crate::surface::{Phase, SurfaceState, fetch_range};

/// Typing is sent to the extension once the user pauses this long.
pub const TYPING_DEBOUNCE: Duration = Duration::from_millis(150);
/// A list or table shows at least this many rows when its parent does not give it a height.
const MIN_VISIBLE_ROWS: u32 = 3;
const DEFAULT_ROW_HEIGHT: u16 = 24;

pub(crate) struct InputEditor {
    pub(crate) editor: Entity<Editor>,
    /// The text both sides last agreed on: what the extension last set, or what was last sent.
    /// Edits that leave the text equal to it (including our own `set_text`) are not sent.
    last_known: String,
    /// The `value` prop as last seen, to tell an extension-driven change from a stale value.
    props_value: String,
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
    scroll_handles: HashMap<NodeId, UniformListScrollHandle>,
    synced_revision: Option<u64>,
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
        let mut this = Self {
            state,
            compact,
            inputs: HashMap::default(),
            markdown: HashMap::default(),
            scroll_handles: HashMap::default(),
            synced_revision: None,
            _observe_state: observe_state,
        };
        this.sync(window, cx);
        this
    }

    pub fn state(&self) -> &Entity<SurfaceState> {
        &self.state
    }

    fn dispatch(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.dispatch(event, cx));
    }

    /// Brings the per-window objects (editors, markdown models, scroll handles) in line with the
    /// tree. Runs when the state changes, not per frame.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let revision = state.revision();
        if self.synced_revision == Some(revision) {
            return;
        }
        self.synced_revision = Some(revision);

        let mut inputs = Vec::new();
        let mut markdown = Vec::new();
        let mut lists = Vec::new();
        if let Some(tree) = state.tree() {
            let mut stack: Vec<NodeId> = tree.roots().to_vec();
            while let Some(id) = stack.pop() {
                let Some(node) = tree.get(id) else {
                    continue;
                };
                match node.props() {
                    Props::Input(props) => inputs.push((id, props.clone())),
                    Props::Markdown(source) => markdown.push((id, source.clone())),
                    Props::List(_) | Props::Table(_) => lists.push(id),
                    Props::Flex(_)
                    | Props::Text(_)
                    | Props::Button(_)
                    | Props::Icon(_)
                    | Props::DiffView(_) => {}
                }
                stack.extend_from_slice(node.children());
            }
        }

        self.inputs
            .retain(|id, _| inputs.iter().any(|(input, _)| input == id));
        for (id, props) in inputs {
            self.sync_input(id, props, window, cx);
        }

        self.markdown
            .retain(|id, _| markdown.iter().any(|(node, _)| node == id));
        for (id, source) in markdown {
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

        self.scroll_handles.retain(|id, _| lists.contains(id));
        for id in lists {
            self.scroll_handles.entry(id).or_default();
        }
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
                editor
            });
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
                    last_known: props.value.clone(),
                    props_value: props.value,
                    placeholder: props.placeholder,
                    debounce: None,
                    _subscription: subscription,
                },
            );
            return;
        };

        if input.placeholder != props.placeholder {
            input.editor.update(cx, |editor, cx| {
                editor.set_placeholder_text(&props.placeholder, window, cx)
            });
            input.placeholder = props.placeholder;
        }

        // The text belongs to the user while the field is focused (W4's contract). The extension's
        // value still wins when it changed and the field is not focused, or when it answers the
        // user's own submit (clearing the field, say).
        let overridden = self
            .state
            .update(cx, |state, _| state.take_input_override(id));
        let changed_by_extension = input.props_value != props.value;
        input.props_value = props.value.clone();
        let focused = input.editor.focus_handle(cx).is_focused(window);
        let text = input.editor.read(cx).text(cx);
        if text != props.value && (overridden || (changed_by_extension && !focused)) {
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
            Phase::Mounting => Label::new("Loading…")
                .color(Color::Muted)
                .size(LabelSize::Small)
                .into_any_element(),
            Phase::Failed(error) => Label::new(error.clone())
                .color(Color::Error)
                .size(LabelSize::Small)
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
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .children(children)
                        .into_any_element()
                }
            }
        };
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
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.state.update(cx, |state, cx| state.dismiss_error(cx));
                }))
        });
        container
            .child(body)
            .when(!compact, |this| this.children(error))
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
            Props::DiffView(props) => self.diff_view(props),
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
            .when(props.grow, |this| this.flex_1().min_h_0())
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
        let icon = props.icon.as_deref().map(icon_name);
        let style = button_style(props.style);
        // Inside a row the whole row is the click target (`row-click`): row-local ids are not
        // surface ids, so a `click` on them would be ambiguous.
        let on_click = match scope {
            Scope::Surface => Some(self.cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.dispatch(UiEvent::Click(id), cx)
            })),
            Scope::Row { .. } => None,
        };
        if props.label.is_empty()
            && let Some(icon) = icon
        {
            return IconButton::new(element_id, icon)
                .icon_size(IconSize::Small)
                .style(style)
                .disabled(props.disabled)
                .when_some(on_click, |this, on_click| this.on_click(on_click))
                .into_any_element();
        }
        Button::new(element_id, props.label.clone())
            .style(style)
            .disabled(props.disabled)
            .when_some(icon, |this, icon| {
                this.start_icon(Icon::new(icon).size(IconSize::Small))
            })
            .when_some(on_click, |this, on_click| this.on_click(on_click))
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
        div()
            .id(scope.element_id(id))
            .w_full()
            .min_w_0()
            .px_2()
            .py_1()
            .rounded_sm()
            .border_1()
            .border_color(colors.border)
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
            .flex_1()
            .min_h_0()
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
        if let Scope::Row { .. } = scope {
            return Label::new("Lists inside list rows are not supported")
                .color(Color::Muted)
                .size(LabelSize::Small)
                .into_any_element();
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
        .with_sizing_behavior(ListSizingBehavior::Infer)
        .flex_1()
        .min_h(px(min_height as f32))
        .w_full();
        match self.view.scroll_handles.get(&id) {
            Some(handle) => list.track_scroll(handle).into_any_element(),
            None => list.into_any_element(),
        }
    }

    fn row(
        &self,
        list: NodeId,
        index: u32,
        row_height: u16,
        table_columns: Option<usize>,
    ) -> AnyElement {
        let scope = Scope::Row { list, index };
        let row = div()
            .id(ElementId::NamedInteger("fyx-row".into(), index as u64))
            .h(px(row_height.into()))
            .w_full()
            .px_1()
            .flex()
            .items_center()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|style| style.bg(self.cx.theme().colors().ghost_element_hover))
            .on_click(self.cx.listener(move |this, _: &ClickEvent, _, cx| {
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

    fn icon(&self, props: &IconProps) -> AnyElement {
        Icon::new(icon_name(&props.name))
            .size(IconSize::Small)
            .when_some(props.color, |this, token| {
                this.color(text_color(token, self.cx))
            })
            .into_any_element()
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
                .child(MarkdownElement::new(
                    entity,
                    MarkdownStyle::themed(MarkdownFont::Agent, self.window, self.cx),
                ))
                .into_any_element(),
            None => Label::new(source.to_string()).into_any_element(),
        }
    }

    /// Placeholder until the host diff component lands: unchanged leading and trailing lines as
    /// context, the middle as removed and added lines.
    fn diff_view(&self, props: &DiffViewProps) -> AnyElement {
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

/// Icon names are snake_case `IconName`s (`plus`, `list_todo`); unknown names get a neutral icon.
pub(crate) fn icon_name(name: &str) -> IconName {
    name.parse().unwrap_or(IconName::Blocks)
}

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
