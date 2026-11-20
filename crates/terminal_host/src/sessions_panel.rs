//! The "Agent Sessions" dock panel: every terminal in this workspace with a status and every agent
//! thread of its project (plan 7.9), the ones that need you first; a click jumps to the terminal
//! or brings the thread to the front in the agent panel.

use gpui::{
    Action, App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, Pixels, Stateful,
    Subscription, WeakEntity, Window, actions, px,
};
use terminal_ext::TerminalStatus;
use terminal_view::TerminalView;
use ui::{Indicator, prelude::*};
use workspace::Workspace;
use workspace::dock::{DockPosition, Panel, PanelEvent};

use crate::agent_board::{self, BoardRow, ThreadPhase, ThreadRow};
use crate::sessions::{self, Sessions};
use crate::tab_status::{status_icon, tooltip_text};

actions!(
    fyx_terminal,
    [
        /// Shows or hides the Agent Sessions panel: terminals whose programs report a status.
        ToggleAgentSessions,
    ]
);

pub(crate) fn register(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(sessions) = sessions::global(cx) else {
        return;
    };
    let workspace_handle = cx.weak_entity();
    let panel = cx.new(|cx| AgentSessionsPanel::new(workspace_handle, sessions, cx));
    workspace.add_panel(panel, window, cx);
    workspace.register_action(|workspace, _: &ToggleAgentSessions, window, cx| {
        workspace.toggle_panel_focus::<AgentSessionsPanel>(window, cx);
    });
}

pub struct AgentSessionsPanel {
    workspace: WeakEntity<Workspace>,
    sessions: Entity<Sessions>,
    focus_handle: FocusHandle,
    position: DockPosition,
    _observe_sessions: Subscription,
    /// Plan 7.8: today's cost line follows the meters.
    _observe_meters: Option<Subscription>,
}

impl AgentSessionsPanel {
    pub(crate) fn new(
        workspace: WeakEntity<Workspace>,
        sessions: Entity<Sessions>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _observe_sessions: cx.observe(&sessions, |_, _, cx| cx.notify()),
            _observe_meters: crate::usage_meters::global(cx)
                .map(|meters| cx.observe(&meters, |_, _, cx| cx.notify())),
            workspace,
            sessions,
            focus_handle: cx.focus_handle(),
            position: DockPosition::Right,
        }
    }

    fn rows(&self, cx: &App) -> Vec<BoardRow> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        let terminals = self
            .sessions
            .read(cx)
            .list(cx)
            .into_iter()
            .filter(|(view, _)| sessions::workspace_contains(workspace, view, cx))
            .collect();
        let threads = agent_board::thread_rows(workspace.project(), cx);
        agent_board::board(terminals, threads)
    }

    /// Deferred: the terminal panel can share this panel's dock, and focusing it deactivates this
    /// panel, which cannot be updated while its own click handler runs.
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
}

impl AgentSessionsPanel {
    fn render_terminal(
        &self,
        index: usize,
        view: Entity<TerminalView>,
        status: TerminalStatus,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let terminal_title = view.read(cx).terminal().read(cx).title(true);
        let accounts = view.read(cx).terminal().read(cx).fyx.accounts();
        let (title, meta) = tooltip_text(&status);
        let icon = status_icon(&status).unwrap_or_else(|| {
            Icon::new(IconName::Terminal)
                .size(IconSize::Small)
                .color(Color::Muted)
                .into_any_element()
        });
        row_frame(("fyx-agent-session", index), cx)
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
            .on_click(cx.listener(move |this, _, window, cx| this.jump_to(&view, window, cx)))
    }

    fn render_thread(
        &self,
        index: usize,
        row: ThreadRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let meta = row.meta();
        let (icon_color, label_color) = match row.phase {
            ThreadPhase::NeedsYou => (Color::Accent, Color::Accent),
            ThreadPhase::Working => (Color::Default, Color::Default),
            ThreadPhase::Idle => (Color::Muted, Color::Muted),
        };
        let thread = row.thread.clone();
        row_frame(("fyx-agent-thread", index), cx)
            .child(
                div().pt_0p5().child(
                    Icon::new(IconName::ZedAgent)
                        .size(IconSize::Small)
                        .color(icon_color),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Label::new(row.title.clone()).single_line().truncate())
                            .when(row.phase == ThreadPhase::NeedsYou, |this| {
                                this.child(Indicator::dot().color(Color::Accent))
                            }),
                    )
                    .child(
                        Label::new(row.phase.label())
                            .size(LabelSize::Small)
                            .color(label_color),
                    )
                    .when(!meta.is_empty(), |this| {
                        this.child(
                            Label::new(meta)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .single_line()
                                .truncate(),
                        )
                    }),
            )
            // Deferred like `jump_to`: the agent panel can share this panel's dock.
            .on_click(move |_, window, cx| {
                let thread = thread.clone();
                window.defer(cx, move |window, cx| {
                    acp_thread::fyx_focus::focus_thread(&thread, window, cx)
                });
            })
    }
}

fn row_frame(id: (&'static str, usize), cx: &App) -> Stateful<Div> {
    h_flex()
        .id(id)
        .w_full()
        .px_2()
        .py_1()
        .gap_2()
        .items_start()
        .rounded_sm()
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().colors().element_hover))
}

impl Render for AgentSessionsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows(cx);
        let body = if rows.is_empty() {
            v_flex()
                .p_2()
                .gap_1()
                .child(
                    Label::new("No agent thread, and no terminal reports a status.")
                        .size(LabelSize::Small),
                )
                .child(
                    Label::new(
                        "Programs that send OSC 7501 (Claude Code 2.1.295 and later) or \
                         OSC 9;4 progress show up here.",
                    )
                    .size(LabelSize::Small)
                    .color(Color::Muted),
                )
        } else {
            v_flex()
                .p_1()
                .gap_0p5()
                .children(rows.into_iter().enumerate().map(|(index, row)| {
                    match row {
                        BoardRow::Terminal(view, status) => self
                            .render_terminal(index, view, status, cx)
                            .into_any_element(),
                        BoardRow::Thread(row) => {
                            self.render_thread(index, row, cx).into_any_element()
                        }
                    }
                }))
        };
        let today = crate::usage_meters::global(cx).and_then(|meters| {
            let meters = meters.read(cx);
            crate::usage_meters::today_line(&meters.today(), meters.cap_state(cx))
        });
        v_flex()
            .key_context("AgentSessionsPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(
                v_flex()
                    .id("fyx-agent-sessions-body")
                    .size_full()
                    .overflow_y_scroll()
                    .child(body),
            )
            .children(today.map(|(line, reached)| {
                h_flex()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new(line).size(LabelSize::Small).color(if reached {
                        Color::Warning
                    } else {
                        Color::Muted
                    }))
            }))
    }
}

impl EventEmitter<PanelEvent> for AgentSessionsPanel {}

impl Focusable for AgentSessionsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for AgentSessionsPanel {
    fn persistent_name() -> &'static str {
        "Agent Sessions"
    }

    fn panel_key() -> &'static str {
        "FyxAgentSessions"
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
        px(320.)
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::Bell)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some("Agent Sessions")
    }

    fn icon_label(&self, _: &Window, cx: &App) -> Option<String> {
        let waiting = self.rows(cx).iter().filter(|row| row.needs_you()).count();
        (waiting > 0).then(|| waiting.to_string())
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleAgentSessions)
    }

    fn activation_priority(&self) -> u32 {
        8
    }
}
