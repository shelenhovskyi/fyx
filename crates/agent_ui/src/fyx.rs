//! fyx: the account an agent uses, beside the agent's icon in the panel's toolbar (`fyx.accounts`,
//! design §C.6: the account is always visible), and the "+" menu's "Claude Code — Work" entries.
//! Fyx's code in this crate lives in files named `fyx*`, so rebases never conflict on it;
//! `agent_panel.rs` calls it from `fyx:` lines.

use std::rc::Rc;

use agent_servers::AgentAccount;
use gpui::{AnyElement, App, Entity, Global, Hsla, WeakEntity};
use project::{AgentId, Project};
use settings::FyxAccountColor;
use ui::{ContextMenu, ContextMenuEntry, IconPosition, Indicator, Tooltip, prelude::*};
use workspace::Workspace;

use crate::{Agent, NewExternalAgentThread, agent_panel::AgentPanel};

/// Set by a "+" account entry for the new thread it opens right after: an empty draft of the
/// agent belongs to the old account's connection, so a new draft replaces it.
#[derive(Default)]
struct NewAccountThread(bool);

impl Global for NewAccountThread {}

/// For `AgentPanel::activate_new_thread`: whether this new thread is for a newly chosen account
/// (§C.6 rule 3: a new account is a new thread, never the old one carried over). Read once.
pub(crate) fn take_new_account_thread(cx: &mut App) -> bool {
    let new_account = cx
        .try_global::<NewAccountThread>()
        .is_some_and(|flag| flag.0);
    if new_account {
        cx.set_global(NewAccountThread(false));
    }
    new_account
}

/// After an agent's own "+" entry: "<agent> — <account>" for each account of the agent's tool,
/// the project's current one ticked. Choosing one makes it the project's account for the tool
/// (user settings) and opens a new thread with it. An agent connected with another account gets
/// a new connection; threads already open keep theirs.
pub(crate) fn account_entries(
    mut menu: ContextMenu,
    agent_id: &AgentId,
    display_name: &SharedString,
    workspace: &WeakEntity<Workspace>,
    disabled: bool,
    cx: &App,
) -> ContextMenu {
    let Some(project) = workspace
        .upgrade()
        .map(|workspace| workspace.read(cx).project().clone())
    else {
        return menu;
    };
    for (account, chosen) in agent_servers::account_choices(agent_id, &project, cx) {
        let label = format!("{display_name} — {}", account.name);
        let entry = ContextMenuEntry::new(label)
            .toggleable(IconPosition::End, chosen)
            .disabled(disabled)
            .handler({
                let (workspace, agent_id) = (workspace.clone(), agent_id.clone());
                move |window, cx| {
                    let Some(workspace) = workspace.upgrade() else {
                        return;
                    };
                    workspace.update(cx, |workspace, cx| {
                        let project = workspace.project().clone();
                        let Some(panel) = workspace.panel::<AgentPanel>(cx) else {
                            return;
                        };
                        panel.update(cx, |panel, cx| {
                            if agent_servers::choose_account(&agent_id, &project, &account, cx) {
                                let id = agent_id.clone();
                                panel.connection_store().update(cx, |store, cx| {
                                    store.restart_connection(
                                        Agent::Custom { id: id.clone() },
                                        Rc::new(agent_servers::CustomAgentServer::new(id)),
                                        cx,
                                    );
                                });
                                cx.set_global(NewAccountThread(true));
                            }
                            panel.new_external_agent_thread(
                                &NewExternalAgentThread {
                                    agent: agent_id.clone(),
                                },
                                window,
                                cx,
                            );
                            cx.set_global(NewAccountThread(false));
                        });
                    });
                }
            });
        menu = menu.item(entry);
    }
    menu
}

fn color(color: FyxAccountColor, cx: &App) -> Hsla {
    let colors = cx.theme().colors();
    match color {
        FyxAccountColor::Green => colors.terminal_ansi_green,
        FyxAccountColor::Yellow => colors.terminal_ansi_yellow,
        FyxAccountColor::Red => colors.terminal_ansi_red,
        FyxAccountColor::Magenta => colors.terminal_ansi_magenta,
        FyxAccountColor::Cyan => colors.terminal_ansi_cyan,
        FyxAccountColor::Blue | FyxAccountColor::Unrecognized => colors.terminal_ansi_blue,
    }
}

/// The tooltip: which account, and whether the agent already uses it.
pub(crate) fn describe(account: &AgentAccount) -> String {
    let state = if account.connected {
        "this agent connected with it"
    } else {
        "the agent gets it when it connects"
    };
    format!(
        "{} account: {} ({state})",
        account.tool_label, account.account.name
    )
}

/// The selected agent's account in this project, as a coloured dot and its name.
pub(crate) fn account_badge(
    agent: &Agent,
    project: &Entity<Project>,
    showing_terminal: bool,
    cx: &App,
) -> Option<AnyElement> {
    let Agent::Custom { id } = agent else {
        return None;
    };
    if showing_terminal {
        return None;
    }
    let account = agent_servers::agent_account(id, project, cx)?;
    let tooltip = describe(&account);
    Some(
        h_flex()
            .id("fyx-agent-account")
            .flex_none()
            .gap_0p5()
            .child(Indicator::dot().color(Color::Custom(color(account.account.color, cx))))
            .child(
                Label::new(account.account.name)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .single_line(),
            )
            .tooltip(Tooltip::text(tooltip))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use settings::FyxAccount;

    use super::*;

    #[test]
    fn the_tooltip_names_the_tool_the_account_and_whether_it_is_in_use() {
        let mut account = AgentAccount {
            account: FyxAccount {
                name: "Work".into(),
                tool: "claude".into(),
                env: "CLAUDE_CONFIG_DIR".into(),
                dir: PathBuf::from("/w"),
                color: FyxAccountColor::Green,
            },
            tool_label: "Claude Code".into(),
            connected: false,
        };
        assert_eq!(
            describe(&account),
            "Claude Code account: Work (the agent gets it when it connects)"
        );
        account.connected = true;
        assert_eq!(
            describe(&account),
            "Claude Code account: Work (this agent connected with it)"
        );
    }
}

/// Plan 7.9: the Agent Sessions board (`terminal_host`) brings a thread to the front through
/// `acp_thread::fyx_focus`: its window's agent panel opens the thread and takes focus.
pub(crate) fn install_thread_focuser(cx: &mut App) {
    acp_thread::fyx_focus::set_thread_focuser(
        Rc::new(|thread, window, cx| {
            let Some(workspace) = Workspace::for_window(window, cx) else {
                return;
            };
            let (session_id, title) = {
                let thread = thread.read(cx);
                (thread.session_id().clone(), thread.title())
            };
            workspace.update(cx, |workspace, cx| {
                if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                    panel.update(cx, |panel, cx| {
                        panel.open_thread(session_id, None, title, window, cx)
                    });
                }
                workspace.focus_panel::<AgentPanel>(window, cx);
            });
        }),
        cx,
    );
}
