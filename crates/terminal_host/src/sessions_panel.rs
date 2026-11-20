//! "Agent Sessions": every terminal in this workspace with a status, the ones that need attention
//! first; a click jumps to the terminal.
//!
//! The list is a sidebar to the right of the terminal panel's panes (like Zed's threads sidebar),
//! so the terminal stays visible beside it. A status-bar button with the count waiting, or
//! `fyx terminal: toggle agent sessions`, shows and hides it and opens the terminal panel.

use std::collections::HashMap;

use gpui::{
    App, Context, Entity, EntityId, FocusHandle, Focusable, Global, Subscription, WeakEntity,
    Window, actions, px,
};
use terminal_ext::TerminalStatus;
use terminal_view::TerminalView;
use terminal_view::terminal_panel::TerminalPanel;
use ui::{CountBadge, Indicator, Tab, Tooltip, prelude::*};
use workspace::Workspace;
use workspace::item::ItemHandle;
use workspace::{HideStatusItem, StatusItemView};

use crate::sessions::{self, Sessions};
use crate::tab_status::{status_icon, tooltip_text};

actions!(
    fyx_terminal,
    [
        /// Shows or hides the Agent Sessions list beside the terminal panel: terminals whose
        /// programs report a status.
        ToggleAgentSessions,
    ]
);

const SIDEBAR_WIDTH: f32 = 240.;

/// Each workspace's list, by the workspace's entity id; the terminal panel asks for it through
/// `terminal_ext::panel_sidebar`.
#[derive(Default)]
struct Sidebars(HashMap<EntityId, Entity<AgentSessionsPanel>>);

impl Global for Sidebars {}

pub(crate) fn sidebar(workspace: EntityId, cx: &App) -> Option<Entity<AgentSessionsPanel>> {
    cx.try_global::<Sidebars>()?.0.get(&workspace).cloned()
}

pub(crate) fn register(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(sessions) = sessions::global(cx) else {
        return;
    };
    let workspace_id = cx.entity_id();
    let workspace_handle = cx.weak_entity();
    let panel = cx.new(|cx| AgentSessionsPanel::new(workspace_handle, sessions, cx));
    cx.default_global::<Sidebars>()
        .0
        .insert(workspace_id, panel.clone());
    cx.on_release(move |_, cx| {
        cx.default_global::<Sidebars>().0.remove(&workspace_id);
    })
    .detach();
    let button = cx.new(|cx| AgentSessionsButton::new(panel, cx));
    workspace.status_bar().update(cx, |status_bar, cx| {
        status_bar.add_right_item(button, window, cx)
    });
    workspace.register_action(|workspace, _: &ToggleAgentSessions, window, cx| {
        toggle(workspace, window, cx);
    });
}

/// Hides the list when it is showing; otherwise shows it and opens the terminal panel.
pub(crate) fn toggle(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(panel) = sidebar(cx.entity_id(), cx) else {
        return;
    };
    let showing = panel.read(cx).open && terminal_panel_visible(workspace, cx);
    panel.update(cx, |panel, cx| {
        panel.open = !showing;
        cx.notify();
    });
    if !showing {
        workspace.reveal_panel::<TerminalPanel>(window, cx);
    }
}

fn terminal_panel_visible(workspace: &Workspace, cx: &App) -> bool {
    let Some(terminal_panel) = workspace.panel::<TerminalPanel>(cx) else {
        return false;
    };
    workspace.all_docks().iter().any(|dock| {
        let dock = dock.read(cx);
        dock.is_open()
            && dock
                .visible_panel()
                .is_some_and(|panel| panel.panel_id() == terminal_panel.entity_id())
    })
}

pub struct AgentSessionsPanel {
    workspace: WeakEntity<Workspace>,
    sessions: Entity<Sessions>,
    focus_handle: FocusHandle,
    pub(crate) open: bool,
    _observe_sessions: Subscription,
}

impl AgentSessionsPanel {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        sessions: Entity<Sessions>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _observe_sessions: cx.observe(&sessions, |_, _, cx| cx.notify()),
            workspace,
            sessions,
            focus_handle: cx.focus_handle(),
            open: false,
        }
    }

    fn rows(&self, cx: &App) -> Vec<(Entity<TerminalView>, TerminalStatus)> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        self.sessions
            .read(cx)
            .list(cx)
            .into_iter()
            .filter(|(view, _)| sessions::workspace_contains(workspace, view, cx))
            .collect()
    }

    fn waiting(&self, cx: &App) -> usize {
        self.rows(cx)
            .iter()
            .filter(|(_, status)| status.attention)
            .count()
    }

    /// Runs from a row's click listener, i.e. inside this list's update and, as it is drawn by the
    /// terminal panel, while that panel's dock may be mid-update too; revealing the terminal
    /// (re)activates panels synchronously, so it waits for the window's next turn.
    pub(crate) fn jump_to(
        &mut self,
        view: &Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace.clone();
        let view = view.clone();
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    sessions::reveal(workspace, &view, window, cx)
                })
                .ok();
        });
    }

    fn render_rows(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows(cx);
        if rows.is_empty() {
            return v_flex()
                .p_2()
                .gap_1()
                .child(Label::new("No terminal reports a status.").size(LabelSize::Small))
                .child(
                    Label::new(
                        "Programs that send OSC 7501 (Claude Code 2.1.295 and later) or \
                         OSC 9;4 progress show up here.",
                    )
                    .size(LabelSize::Small)
                    .color(Color::Muted),
                )
                .into_any_element();
        }
        v_flex()
            .p_1()
            .gap_0p5()
            .children(rows.into_iter().enumerate().map(|(index, (view, status))| {
                let terminal_title = view.read(cx).terminal().read(cx).title(true);
                let accounts = view.read(cx).terminal().read(cx).fyx.accounts();
                let (title, meta) = tooltip_text(&status);
                let icon = status_icon(&status).unwrap_or_else(|| {
                    Icon::new(IconName::Terminal)
                        .size(IconSize::Small)
                        .color(Color::Muted)
                        .into_any_element()
                });
                h_flex()
                    .id(("fyx-agent-session", index))
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_start()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().colors().element_hover))
                    .child(div().pt_0p5().child(icon))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(Label::new(terminal_title).single_line().truncate())
                                    .children(
                                        accounts
                                            .iter()
                                            .map(|account| crate::accounts::badge(account, cx)),
                                    )
                                    .when(status.attention, |this| {
                                        this.child(Indicator::dot().color(Color::Accent))
                                    }),
                            )
                            .child(
                                Label::new(title)
                                    .size(LabelSize::Small)
                                    .single_line()
                                    .truncate(),
                            )
                            .child(
                                Label::new(meta)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .single_line()
                                    .truncate(),
                            ),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.jump_to(&view, window, cx)),
                    )
            }))
            .into_any_element()
    }
}

impl Render for AgentSessionsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let colors = cx.theme().colors();
        let (panel_background, border) = (colors.panel_background, colors.border);
        v_flex()
            .key_context("AgentSessionsPanel")
            .track_focus(&self.focus_handle)
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .border_l_1()
            .border_color(border)
            .bg(panel_background)
            .child(
                h_flex()
                    .h(Tab::container_height(cx))
                    .flex_none()
                    .px_2()
                    .justify_between()
                    .border_b_1()
                    .border_color(border)
                    .child(Label::new("Agent Sessions").size(LabelSize::Small))
                    .child(
                        IconButton::new("fyx-agent-sessions-close", IconName::Close)
                            .icon_size(IconSize::Small)
                            .tooltip(move |_window, cx| {
                                Tooltip::for_action("Hide Agent Sessions", &ToggleAgentSessions, cx)
                            })
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(ToggleAgentSessions), cx)
                            }),
                    ),
            )
            .child(
                v_flex()
                    .id("fyx-agent-sessions-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_rows(cx)),
            )
            .into_any_element()
    }
}

impl Focusable for AgentSessionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The status-bar button: shows or hides the list, with the count of sessions waiting.
struct AgentSessionsButton {
    panel: Entity<AgentSessionsPanel>,
    _observe_panel: Subscription,
}

impl AgentSessionsButton {
    fn new(panel: Entity<AgentSessionsPanel>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe_panel: cx.observe(&panel, |_, _, cx| cx.notify()),
            panel,
        }
    }
}

impl Render for AgentSessionsButton {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = self.panel.read(cx);
        let open = panel.open;
        let waiting = panel.waiting(cx);
        let icon = if open {
            IconName::ThreadsSidebarRightOpen
        } else {
            IconName::ThreadsSidebarRightClosed
        };
        div()
            .relative()
            .child(
                IconButton::new(("fyx-agent-sessions-button", open as u64), icon)
                    .icon_size(IconSize::Small)
                    .toggle_state(open)
                    .aria_label("Agent Sessions")
                    .tooltip(move |_window, cx| {
                        Tooltip::for_action("Agent Sessions", &ToggleAgentSessions, cx)
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(ToggleAgentSessions), cx)
                    }),
            )
            .when(waiting > 0 && !open, |this| {
                this.child(CountBadge::new(waiting))
            })
    }
}

impl StatusItemView for AgentSessionsButton {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}
