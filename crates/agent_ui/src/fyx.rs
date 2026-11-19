//! fyx: the account an agent uses, beside the agent's icon in the panel's toolbar (`fyx.accounts`,
//! design §C.6: the account is always visible). Fyx's code in this crate lives in files named
//! `fyx*`, so rebases never conflict on it; `agent_panel.rs` calls it from one `fyx:` line.

use agent_servers::AgentAccount;
use gpui::{AnyElement, App, Entity, Hsla};
use project::Project;
use settings::FyxAccountColor;
use ui::{Indicator, Tooltip, prelude::*};

use crate::Agent;

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
                Label::new(account.account.name.clone())
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
