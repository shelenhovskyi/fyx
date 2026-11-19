//! Fyx's terminal UI on top of `terminal_ext`: the status on a terminal's tab, the Agent Sessions
//! panel and pop-up notifications, and the `ui_ext::terminal` host for extensions.

mod accounts;
mod created;
mod host;
mod notification;
mod pseudo;
mod sessions;
mod sessions_panel;
mod settings;
mod tab_status;

use gpui::App;
use workspace::Workspace;

pub use accounts::{FyxAccountsSettings, account_choice};
pub use sessions_panel::{AgentSessionsPanel, ToggleAgentSessions};
pub use settings::FyxTerminalSettings;

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{Entity, Global, WeakEntity};
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

fn terminal_by_id(id: TerminalId, cx: &App) -> Option<Entity<Terminal>> {
    cx.try_global::<TerminalsById>()
        .and_then(|terminals| terminals.0.get(&id))
        .and_then(WeakEntity::upgrade)
}

fn run(command: host::Command, cx: &mut App) {
    match command {
        host::Command::Repaint(id) => {
            if let Some(terminal) = terminal_by_id(id, cx) {
                terminal.update(cx, |_, cx| cx.emit(terminal::Event::Wakeup));
            }
        }
        host::Command::OpenPseudo(open) => pseudo::open(open, cx),
        host::Command::Output(id, bytes) => {
            if let Some(terminal) = terminal_by_id(id, cx) {
                terminal.update(cx, |terminal, cx| terminal.write_raw_output(&bytes, cx));
            }
        }
        host::Command::OpenCreated(open) => created::open(open, cx),
        host::Command::Input(id, bytes) => created::input(id, bytes, cx),
        host::Command::Paste(id, bytes) => created::paste(id, bytes, cx),
        host::Command::Close(id) => {
            if !created::cancel(id, cx) {
                pseudo::close(id, cx);
            }
        }
    }
}

/// For an extension panel's `terminal` node (`fyx:ui` §B.2): the terminal `ext` holds without a
/// tab under `handle` (a pseudo-terminal opened `in_panel`, or a created one started with no
/// window). `None` for anyone else's handle.
pub fn extension_terminal(
    ext: &str,
    handle: ui_ext::terminal::TerminalHandle,
    cx: &App,
) -> Option<Entity<Terminal>> {
    let id = front(cx)?.owned_terminal(ext, handle)?;
    pseudo::held_terminal(id, cx)
}

pub fn init(cx: &mut App) {
    init_host(cx);
    sessions::init(cx);
    tab_status::init(cx);
    accounts::init(cx);
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        if let Some(window) = window {
            sessions_panel::register(workspace, window, cx);
        }
    })
    .detach();
}
