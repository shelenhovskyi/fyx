//! The Extension Inspector (plan 1.7, spec decision 9): a dock panel for extension authors,
//! present in dev builds or with `fyx.extensions.developer_mode`. For one surface at a time it
//! shows the live tree, the recent events and updates (with whether each applied), the
//! instance's calls with their duration against the budget and push count, its capability checks
//! and denials, and its `log` output.
//!
//! Everything shown is recorded elsewhere (`SurfaceState::history`, `UiExtension::diagnostics`);
//! the panel re-renders when the registry or a surface changes, and never calls the extension.

use std::time::SystemTime;

use gpui::{
    Action, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Pixels,
    Render, ScrollHandle, Subscription, Window, actions, prelude::*, px,
};
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Clickable as _, Color, IconName, Label,
    LabelCommon as _, LabelSize, h_flex, v_flex,
};
use ui_ext::v0_2::{NodeId, Props, tree::Tree};
use workspace::dock::{DockPosition, Panel, PanelEvent};

use crate::{
    UiExtensionRegistry,
    surface::{HistoryEntry, Phase},
};

actions!(
    fyx_ui,
    [
        /// Shows or hides the Extension Inspector (dev builds, or fyx.extensions.developer_mode).
        ToggleExtensionInspector,
    ]
);

/// Lines of each list the panel shows, newest first.
const SHOWN_ENTRIES: usize = 50;

/// What the inspector shows, as text: built from the registry, so tests read it without
/// rendering.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorSnapshot {
    /// Display keys of the loaded surfaces.
    pub surfaces: Vec<String>,
    pub selected: Option<String>,
    pub phase: String,
    /// Plan 7.7: the last and slowest recent render time, and the instance's memory.
    pub performance: String,
    /// One line per node, indented by depth.
    pub tree: Vec<String>,
    /// Events and updates, newest first.
    pub history: Vec<String>,
    /// Calls, newest first: kind, surface, duration against budget, pushes, error.
    pub calls: Vec<String>,
    pub capability_checks: Vec<String>,
    /// 0.3 I/O, newest first, one line per operation: fetches, programs, edits, the agent feed
    /// (redacted by the host before it gets here).
    pub io: Vec<String>,
    pub logs: Vec<String>,
}

pub struct ExtensionInspector {
    registry: Entity<UiExtensionRegistry>,
    selected: Option<String>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    position: DockPosition,
    _observe_registry: Subscription,
    _observe_surfaces: Vec<Subscription>,
}

fn clock(at: SystemTime) -> String {
    let seconds = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs_f64())
        .unwrap_or_default();
    // Seconds within the minute, enough to order what happened.
    format!("{:06.3}", seconds % 60.0)
}

fn short(text: &str) -> String {
    const MAX: usize = 40;
    let first_line = text.lines().next().unwrap_or_default();
    if first_line.chars().count() > MAX || text.contains('\n') {
        format!("{:?}…", first_line.chars().take(MAX).collect::<String>())
    } else {
        format!("{first_line:?}")
    }
}

fn describe_props(props: &Props) -> String {
    let kind = props.kind_name();
    match props {
        Props::Text(text) => format!("{kind} {}", short(&text.content)),
        Props::Button(button) => format!(
            "{kind} {}{}",
            short(&button.label),
            if button.disabled { " (disabled)" } else { "" }
        ),
        // Design K: a secret value is never shown.
        Props::Input(input) if input.secret => format!("{kind} (secret)"),
        Props::Input(input) => format!("{kind} {}", short(&input.value)),
        Props::List(list) => format!("{kind} {} rows", list.row_count),
        Props::Table(table) => format!("{kind} {} rows", table.row_count),
        Props::Icon(icon) => match &icon.name {
            ui_ext::v0_2::IconRef::Frozen(frozen) => format!("{kind} {}", frozen.name()),
            ui_ext::v0_2::IconRef::Upstream(name) => format!("{kind} {name} (0.1)"),
        },
        Props::Markdown(source) => format!("{kind} {}", short(source)),
        Props::DiffView(diff) => format!(
            "{kind} {} → {} bytes",
            diff.old_text.len(),
            diff.new_text.len()
        ),
        Props::Image(image) => format!("{kind} {}", short(&image.alt)),
        Props::Canvas(canvas) => format!(
            "{kind} {}x{} {}",
            canvas.width,
            canvas.height,
            short(&canvas.description)
        ),
        Props::Terminal(terminal) => format!("{kind} handle {}", terminal.handle),
        Props::Flex(_) => kind.to_string(),
    }
}

fn tree_lines(tree: &Tree) -> Vec<String> {
    let mut lines = Vec::new();
    let mut stack: Vec<(NodeId, usize)> = tree.roots().iter().rev().map(|&id| (id, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = tree.get(id) else {
            continue;
        };
        lines.push(format!(
            "{}#{id} {}",
            "  ".repeat(depth),
            describe_props(node.props())
        ));
        stack.extend(
            node.children()
                .iter()
                .rev()
                .map(|&child| (child, depth + 1)),
        );
    }
    lines
}

/// Bytes as `812 B`, `4.2 KiB` or `8.0 MiB`.
fn byte_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KIB {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{:.1} MiB", value / (KIB * KIB))
    }
}

/// One I/O record: when, which capability, its id, what it does, and how it ended.
fn performance_line(
    render_times: Option<(std::time::Duration, std::time::Duration)>,
    memory_bytes: Option<usize>,
) -> String {
    let render = match render_times {
        Some((last, slowest)) => format!(
            "render {:.1} ms, slowest of the last frames {:.1} ms",
            last.as_secs_f64() * 1000.0,
            slowest.as_secs_f64() * 1000.0
        ),
        None => "not rendered yet".to_string(),
    };
    match memory_bytes {
        Some(bytes) => format!("{render}; memory {}", byte_size(bytes)),
        None => render,
    }
}

fn io_line(record: &ui_ext_wasm::diagnostics::IoRecord) -> String {
    let outcome = match (&record.outcome, record.duration) {
        (Some(outcome), Some(duration)) => format!("{outcome} in {} ms", duration.as_millis()),
        (Some(outcome), None) => outcome.clone(),
        (None, _) => "running".to_string(),
    };
    // A fetch's method as HTTP writes it (the record has the enum's name, `Get`).
    let what = match record.what.split_once(' ') {
        Some((method, rest)) if record.capability == "network:fetch" => {
            format!("{} {rest}", method.to_ascii_uppercase())
        }
        _ => record.what.clone(),
    };
    format!(
        "{} {} #{} {}: {}",
        clock(record.at),
        record.capability,
        record.id,
        ui_ext::sanitize_for_log(&what),
        outcome
    )
}

/// The snapshot of `selected` (or the first surface) in `registry`.
pub fn snapshot(
    registry: &UiExtensionRegistry,
    selected: Option<&str>,
    cx: &App,
) -> InspectorSnapshot {
    let surfaces: Vec<String> = registry
        .surfaces()
        .iter()
        .map(|surface| surface.info.key.to_string())
        .collect();
    let selected = selected
        .filter(|key| surfaces.iter().any(|surface| surface == key))
        .or(surfaces.first().map(String::as_str))
        .map(str::to_string);
    let mut snapshot = InspectorSnapshot {
        surfaces,
        selected: selected.clone(),
        ..InspectorSnapshot::default()
    };
    let Some(loaded) = selected.as_deref().and_then(|key| registry.surface(key)) else {
        return snapshot;
    };
    let state = loaded.state.read(cx);
    snapshot.phase = match state.phase() {
        Phase::Closed => "closed (a modal that is not shown)".into(),
        Phase::Mounting => "mounting".into(),
        Phase::Ready(tree) => format!(
            "ready, {} nodes, {} of props text (cap {})",
            tree.len(),
            byte_size(tree.text_bytes()),
            byte_size(ui_ext::v0_2::tree::MAX_TREE_TEXT_BYTES)
        ),
        Phase::Failed { message, .. } => format!("failed: {message}"),
    };
    if let Some(tree) = state.tree() {
        snapshot.tree = tree_lines(tree);
    }
    snapshot.performance = performance_line(
        state.render_times(),
        state
            .diagnostics()
            .map(|diagnostics| diagnostics.memory_bytes),
    );
    snapshot.history = state
        .history()
        .iter()
        .rev()
        .take(SHOWN_ENTRIES)
        .map(|entry| match entry {
            HistoryEntry::Event { at, event } => format!("{} event {event:?}", clock(*at)),
            HistoryEntry::Update {
                at,
                summary,
                applied,
            } => format!(
                "{} {summary}{}",
                clock(*at),
                if *applied { "" } else { " (rejected)" }
            ),
        })
        .collect();
    if let Some(diagnostics) = state.diagnostics() {
        snapshot.calls = diagnostics
            .calls
            .iter()
            .rev()
            .take(SHOWN_ENTRIES)
            .map(|call| {
                format!(
                    "{} {} {}: {:.1} ms of {} ms, {} pushes{}",
                    clock(call.at),
                    call.kind,
                    call.surface_id,
                    call.duration.as_secs_f64() * 1000.0,
                    call.budget.as_millis(),
                    call.pushes,
                    call.error
                        .as_ref()
                        .map(|error| format!(", failed: {error}"))
                        .unwrap_or_default()
                )
            })
            .collect();
        snapshot.capability_checks = diagnostics
            .capability_checks
            .iter()
            .rev()
            .take(SHOWN_ENTRIES)
            .map(|check| match &check.denied {
                None => format!("{} {} allowed", clock(check.at), check.capability),
                Some(reason) => {
                    format!("{} {} denied: {reason}", clock(check.at), check.capability)
                }
            })
            .collect();
        snapshot.io = diagnostics
            .io
            .iter()
            .rev()
            .take(SHOWN_ENTRIES)
            .map(io_line)
            .collect();
        snapshot.logs = diagnostics
            .logs
            .iter()
            .rev()
            .take(SHOWN_ENTRIES)
            .map(|log| format!("{} {} {}", clock(log.at), log.level, log.message))
            .collect();
    }
    snapshot
}

impl ExtensionInspector {
    pub fn new(registry: Entity<UiExtensionRegistry>, cx: &mut Context<Self>) -> Self {
        let observe_registry = cx.observe(&registry, |this, _, cx| {
            this.observe_surfaces(cx);
            cx.notify();
        });
        let mut this = Self {
            registry,
            selected: None,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            position: DockPosition::Right,
            _observe_registry: observe_registry,
            _observe_surfaces: Vec::new(),
        };
        this.observe_surfaces(cx);
        this
    }

    fn observe_surfaces(&mut self, cx: &mut Context<Self>) {
        let states: Vec<_> = self
            .registry
            .read(cx)
            .surfaces()
            .iter()
            .map(|surface| surface.state.clone())
            .collect();
        self._observe_surfaces = states
            .iter()
            .map(|state| cx.observe(state, |_, _, cx| cx.notify()))
            .collect();
    }

    pub fn select(&mut self, key: String, cx: &mut Context<Self>) {
        self.selected = Some(key);
        cx.notify();
    }

    pub fn snapshot(&self, cx: &App) -> InspectorSnapshot {
        snapshot(self.registry.read(cx), self.selected.as_deref(), cx)
    }
}

fn section(
    title: &'static str,
    lines: Vec<String>,
    empty: &'static str,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .pt_2()
        .child(Label::new(title).size(LabelSize::Small).color(Color::Muted))
        .when(lines.is_empty(), |this| {
            this.child(
                Label::new(empty)
                    .size(LabelSize::XSmall)
                    .color(Color::Placeholder),
            )
        })
        .children(lines.into_iter().map(|line| {
            Label::new(line)
                .size(LabelSize::XSmall)
                .buffer_font(cx)
                .single_line()
        }))
}

impl Render for ExtensionInspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot(cx);
        let selector =
            h_flex()
                .flex_wrap()
                .gap_1()
                .children(snapshot.surfaces.iter().enumerate().map(|(index, key)| {
                    let selected = snapshot.selected.as_deref() == Some(key.as_str());
                    let key = key.clone();
                    Button::new(("fyx-inspector-surface", index), key.clone())
                        .style(if selected {
                            ButtonStyle::Filled
                        } else {
                            ButtonStyle::Subtle
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.select(key.clone(), cx)))
                }));
        let body = if snapshot.selected.is_none() {
            v_flex().child(
                Label::new("No UI extension is loaded.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
        } else {
            v_flex()
                .child(Label::new(snapshot.phase).size(LabelSize::Small))
                .child(
                    Label::new(snapshot.performance)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child(section("Tree", snapshot.tree, "no tree", cx))
                .child(section(
                    "Events and updates",
                    snapshot.history,
                    "none yet",
                    cx,
                ))
                .child(section(
                    "Calls (this extension)",
                    snapshot.calls,
                    "none yet",
                    cx,
                ))
                .child(section(
                    "Capability checks",
                    snapshot.capability_checks,
                    "none",
                    cx,
                ))
                .child(section("Network, programs, edits", snapshot.io, "none", cx))
                .child(section("Log", snapshot.logs, "nothing logged", cx))
        };
        v_flex()
            .key_context("ExtensionInspector")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(
                v_flex()
                    .id("fyx-inspector-body")
                    .size_full()
                    .p_2()
                    .gap_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(selector)
                    .child(body),
            )
    }
}

impl EventEmitter<PanelEvent> for ExtensionInspector {}

impl Focusable for ExtensionInspector {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for ExtensionInspector {
    fn persistent_name() -> &'static str {
        "Extension Inspector"
    }

    fn panel_key() -> &'static str {
        "FyxExtensionInspector"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        self.position
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        self.position = position;
        cx.notify();
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(380.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::ListTree)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Extension Inspector")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleExtensionInspector)
    }

    fn activation_priority(&self) -> u32 {
        99
    }
}

/// Whether workspaces get the inspector: dev builds, or `fyx.extensions.developer_mode`.
pub fn inspector_enabled(cx: &App) -> bool {
    cfg!(debug_assertions) || crate::install::developer_mode_setting(cx)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use ui_ext_wasm::diagnostics::IoRecord;

    use super::{byte_size, io_line, performance_line};

    #[test]
    fn test_byte_size() {
        assert_eq!(byte_size(812), "812 B");
        assert_eq!(byte_size(4300), "4.2 KiB");
        assert_eq!(byte_size(8 << 20), "8.0 MiB");
    }

    #[test]
    fn test_performance_line() {
        assert_eq!(
            performance_line(
                Some((Duration::from_micros(2_500), Duration::from_millis(12))),
                Some(3 * 1024 * 1024)
            ),
            format!(
                "render 2.5 ms, slowest of the last frames 12.0 ms; memory {}",
                byte_size(3 * 1024 * 1024)
            )
        );
        assert_eq!(performance_line(None, None), "not rendered yet");
    }

    #[test]
    fn test_io_line() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_millis(61_250);
        let running = IoRecord {
            at,
            capability: "network:fetch",
            id: 3,
            what: "Get https://example.com/a\nforged line".into(),
            outcome: None,
            duration: None,
        };
        let line = io_line(&running);
        assert!(line.starts_with("01.250 network:fetch #3 GET https://example.com/a"));
        assert!(line.ends_with(": running"), "{line}");
        assert!(!line.contains('\n'), "one line per record: {line}");
        let done = IoRecord {
            outcome: Some("200".into()),
            duration: Some(Duration::from_millis(42)),
            ..running
        };
        assert!(io_line(&done).ends_with(": 200 in 42 ms"));
    }
}
