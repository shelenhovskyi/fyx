//! Fyx's terminal UI on top of `terminal_ext`: the status on a terminal's tab, the Agent Sessions
//! panel and pop-up notifications; later the `ui_ext::terminal` host for extensions.

mod host;
mod notification;
mod sessions;
mod sessions_panel;
mod settings;
mod tab_status;

use gpui::App;
use workspace::Workspace;

pub use sessions_panel::{AgentSessionsPanel, ToggleAgentSessions};
pub use settings::FyxTerminalSettings;

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{Global, WeakEntity};
use terminal::Terminal;
use terminal_ext::TerminalId;

pub use host::HostFront;

struct GlobalHost(Arc<HostFront>);

impl Global for GlobalHost {}

/// Main thread: terminals by id, for work the host queues from other threads.
#[derive(Default)]
struct TerminalsById(HashMap<TerminalId, WeakEntity<Terminal>>);

impl Global for TerminalsById {}

/// The `fyx:ui@0.4` terminal host, for `ui_ext_host` to pass to extensions. Set by [`init`].
pub fn host(cx: &App) -> Option<Arc<dyn ui_ext::terminal::TerminalHost>> {
    cx.try_global::<GlobalHost>()
        .map(|host| host.0.clone() as Arc<dyn ui_ext::terminal::TerminalHost>)
}

pub(crate) fn front(cx: &App) -> Option<Arc<HostFront>> {
    cx.try_global::<GlobalHost>().map(|host| host.0.clone())
}

pub(crate) fn remember_terminal(id: TerminalId, terminal: WeakEntity<Terminal>, cx: &mut App) {
    let terminals = cx.default_global::<TerminalsById>();
    terminals.0.retain(|_, terminal| terminal.is_upgradable());
    terminals.0.insert(id, terminal);
}

fn init_host(cx: &mut App) {
    let (commands, mut queued) = futures::channel::mpsc::unbounded();
    cx.set_global(GlobalHost(Arc::new(HostFront::new(commands))));
    cx.spawn(async move |cx| {
        while let Some(command) = queued.next().await {
            cx.update(|cx| run(command, cx));
        }
    })
    .detach();
}

fn run(command: host::Command, cx: &mut App) {
    match command {
        host::Command::Repaint(id) => {
            let terminal = cx
                .try_global::<TerminalsById>()
                .and_then(|terminals| terminals.0.get(&id))
                .and_then(WeakEntity::upgrade);
            if let Some(terminal) = terminal {
                terminal.update(cx, |_, cx| cx.emit(terminal::Event::Wakeup));
            }
        }
    }
}

pub fn init(cx: &mut App) {
    init_host(cx);
    sessions::init(cx);
    tab_status::init(cx);
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        if let Some(window) = window {
            sessions_panel::register(workspace, window, cx);
        }
    })
    .detach();
}
