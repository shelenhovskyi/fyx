//! The status icon on a terminal's tab, with a tooltip that always names where the status came from.

use gpui::{AnyElement, App, Entity, Focusable as _, IntoElement as _};
use terminal::{Event as TerminalEvent, Terminal};
use terminal_ext::osc::{BlockedKind, StatusState};
use terminal_ext::status::app_display_name;
use terminal_ext::{
    ExtensionOrigin, StatusSource, TerminalDecorations, TerminalExt, TerminalStatus,
};
use terminal_view::TerminalView;
use ui::{CommonAnimationExt as _, Indicator, Tooltip, prelude::*};

pub(crate) fn init(cx: &mut App) {
    terminal_ext::set_decorations(TabStatus, cx);

    // The user has seen a terminal's status once it has focus, now or when it changes. Every view is
    // also an agent-sessions entry; its status changes arrive with the terminal's Wakeup.
    cx.observe_new(|view: &mut TerminalView, window, cx| {
        let Some(window) = window else {
            return;
        };
        let sessions = crate::sessions::global(cx);
        if let Some(sessions) = &sessions {
            let view = cx.weak_entity();
            let window = window.window_handle();
            sessions.update(cx, |sessions, _| sessions.register(view, window));
        }
        let terminal_entity = view.terminal().clone();
        let id = terminal_entity.read(cx).fyx.id();
        crate::remember_terminal(id, terminal_entity.downgrade(), cx);
        let front = crate::front(cx);
        if let Some(front) = &front {
            let (title, cwd) = title_and_cwd(&terminal_entity, cx);
            let status = terminal_entity.read(cx).fyx.status_handle();
            front.terminal_opened(status, None, title, cwd);
            let front = front.clone();
            // A pseudo-terminal the host holds for an extension panel outlives its views.
            cx.on_release(move |_, cx| {
                if !crate::pseudo::is_held(id, cx) {
                    front.terminal_closed(id)
                }
            })
            .detach();
        }
        let focus_handle = view.focus_handle(cx);
        cx.on_focus_in(&focus_handle, window, |view, _, cx| {
            view.terminal().read(cx).fyx.clear_attention();
            cx.notify();
        })
        .detach();
        let terminal = view.terminal().clone();
        cx.subscribe_in(
            &terminal,
            window,
            move |view, terminal, event, window, cx| {
                if !matches!(event, TerminalEvent::Wakeup) {
                    return;
                }
                let focused = view.focus_handle(cx).contains_focused(window, cx);
                if focused {
                    terminal.read(cx).fyx.clear_attention();
                }
                if let Some(front) = &front {
                    let (title, cwd) = title_and_cwd(terminal, cx);
                    front.terminal_changed(terminal.read(cx).fyx.id(), title, cwd);
                }
                if let Some(sessions) = &sessions {
                    let status = terminal.read(cx).fyx.status();
                    let this = cx.entity();
                    let window_active = window.is_window_active();
                    sessions.update(cx, |sessions, cx| {
                        sessions.status_maybe_changed(&this, status, window_active, cx)
                    });
                }
            },
        )
        .detach();
    })
    .detach();
}

fn title_and_cwd(terminal: &Entity<Terminal>, cx: &App) -> (String, Option<String>) {
    let terminal = terminal.read(cx);
    (
        terminal.title(true),
        terminal
            .working_directory()
            .map(|cwd| cwd.to_string_lossy().into_owned()),
    )
}

struct TabStatus;

impl TerminalDecorations for TabStatus {
    fn tab_indicator(&self, terminal: &TerminalExt, _cx: &App) -> Option<AnyElement> {
        let origin = terminal.origin();
        let Some((status, icon)) = terminal
            .status()
            .and_then(|status| Some((status.clone(), status_icon(&status)?)))
        else {
            return origin.map(|origin| origin_marker(terminal, &origin));
        };
        let (title, mut meta) = tooltip_text(&status);
        if let Some(origin) = &origin {
            meta.push_str(" · ");
            meta.push_str(&origin.describe());
        }
        Some(
            h_flex()
                .id(("fyx-terminal-status", terminal.id().as_u64()))
                .gap_0p5()
                .child(icon)
                .when(status.attention, |this| {
                    this.child(Indicator::dot().color(Color::Accent))
                })
                .tooltip(move |_window, cx| {
                    Tooltip::with_meta(title.clone(), None, meta.clone(), cx)
                })
                .into_any_element(),
        )
    }
}

/// A terminal of an extension with no status to show: the extension's mark and whose it is.
fn origin_marker(terminal: &TerminalExt, origin: &ExtensionOrigin) -> AnyElement {
    let title = origin.extension().to_owned();
    let meta = origin.describe();
    div()
        .id(("fyx-terminal-origin", terminal.id().as_u64()))
        .child(
            Icon::new(IconName::Blocks)
                .size(IconSize::Small)
                .color(Color::Muted),
        )
        .tooltip(move |_window, cx| Tooltip::with_meta(title.clone(), None, meta.clone(), cx))
        .into_any_element()
}

/// The state icon (working spins); `None` for idle.
pub(crate) fn status_icon(status: &TerminalStatus) -> Option<AnyElement> {
    let (icon, color) = match status.state {
        StatusState::Working => (IconName::LoadCircle, Color::Accent),
        StatusState::Blocked => (IconName::Warning, Color::Warning),
        StatusState::Error => (IconName::XCircle, Color::Error),
        StatusState::Done => (IconName::Check, Color::Success),
        StatusState::Idle | StatusState::Clear => return None,
    };
    let icon = Icon::new(icon).size(IconSize::Small).color(color);
    Some(if status.state == StatusState::Working {
        icon.with_rotate_animation(2).into_any_element()
    } else {
        icon.into_any_element()
    })
}

const MAX_TOOLTIP_DETAIL: usize = 200;

/// "Claude Code: needs permission — Run tests?", "40% · cargo test · Claude Code via OSC 7501".
pub(crate) fn tooltip_text(status: &TerminalStatus) -> (String, String) {
    let state = match (status.state, status.kind) {
        (StatusState::Blocked, Some(BlockedKind::Permission)) => "needs permission",
        (StatusState::Blocked, Some(BlockedKind::Question)) => "has a question",
        (StatusState::Blocked, Some(BlockedKind::Auth)) => "needs sign-in",
        (StatusState::Blocked, None) => "waiting for input",
        (StatusState::Working, _) => "working",
        (StatusState::Done, _) => "done",
        (StatusState::Error, _) => "failed",
        (StatusState::Idle | StatusState::Clear, _) => "idle",
    };
    let mut title = match (&status.source, status.source.app()) {
        (_, Some(app)) => format!("{}: {state}", app_display_name(app)),
        (StatusSource::Extension { id }, None) => format!("{id}: {state}"),
        (StatusSource::Program { .. }, None) => capitalize(state),
    };
    if let Some(label) = &status.label {
        title.push_str(" — ");
        title.push_str(label);
    }
    let mut meta = Vec::new();
    if let Some(progress) = status.progress {
        meta.push(format!("{progress}%"));
    }
    if let Some(detail) = &status.detail {
        let mut detail: String = detail.chars().take(MAX_TOOLTIP_DETAIL).collect();
        if detail.len() < status.detail.as_ref().map_or(0, String::len) {
            detail.push('…');
        }
        meta.push(detail);
    }
    meta.push(status.source.describe());
    (title, meta.join(" · "))
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use terminal_ext::osc::Protocol;

    use super::*;

    fn status(state: StatusState, app: Option<&str>) -> TerminalStatus {
        TerminalStatus {
            state,
            kind: None,
            label: None,
            detail: None,
            progress: None,
            attention: false,
            source: StatusSource::Program {
                protocol: Protocol::Osc7501,
                app: app.map(str::to_owned),
            },
            updated: Instant::now(),
        }
    }

    #[test]
    fn tooltip_always_names_the_source() {
        let mut blocked = status(StatusState::Blocked, Some("claude-code"));
        blocked.kind = Some(BlockedKind::Permission);
        blocked.label = Some("Run tests?".into());
        blocked.detail = Some("x".repeat(300));
        blocked.progress = Some(40);
        let (title, meta) = tooltip_text(&blocked);
        assert_eq!(title, "Claude Code: needs permission — Run tests?");
        assert!(meta.starts_with("40% · xxx"));
        assert!(meta.ends_with("… · Claude Code via OSC 7501"));

        let (title, meta) = tooltip_text(&status(StatusState::Working, None));
        assert_eq!((title.as_str(), meta.as_str()), ("Working", "via OSC 7501"));

        let (title, _) = tooltip_text(&status(StatusState::Done, Some("my-tool")));
        assert_eq!(title, "my-tool: done");
    }
}
