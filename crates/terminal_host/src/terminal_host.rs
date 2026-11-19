//! Fyx's terminal UI: the status on a terminal's tab (from `terminal_ext`), and later the
//! agent-sessions panel and the `ui_ext::terminal` host for extensions.

mod tab_status;

use gpui::App;

pub fn init(cx: &mut App) {
    tab_status::init(cx);
}
