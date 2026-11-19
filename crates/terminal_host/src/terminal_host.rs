//! Fyx's terminal UI on top of `terminal_ext`: the status on a terminal's tab, the Agent Sessions
//! panel and pop-up notifications; later the `ui_ext::terminal` host for extensions.

mod notification;
mod sessions;
mod sessions_panel;
mod settings;
mod tab_status;

use gpui::App;
use workspace::Workspace;

pub use sessions_panel::{AgentSessionsPanel, ToggleAgentSessions};
pub use settings::FyxTerminalSettings;

pub fn init(cx: &mut App) {
    sessions::init(cx);
    tab_status::init(cx);
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        if let Some(window) = window {
            sessions_panel::register(workspace, window, cx);
        }
    })
    .detach();
}
