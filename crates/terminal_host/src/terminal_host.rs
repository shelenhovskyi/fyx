//! Fyx's terminal UI on top of `terminal_ext`: the status on a terminal's tab, the Agent Sessions
//! panel and pop-up notifications, and the `ui_ext::terminal` host for extensions.

mod accounts;
mod created;
mod discovery;
mod host;
mod notification;
mod profiles;
mod pseudo;
mod relay;
mod sessions;
mod sessions_panel;
mod settings;
mod t3;
mod tab_status;

use gpui::App;
use workspace::Workspace;

pub use accounts::{FyxAccountsSettings, account_choice};
pub use discovery::{
    Context as DiscoveryContext, DiscoveryFile, DiscoveryWriter, OpenListener, discovery_writer,
    init_discovery,
};
pub use profiles::TerminalIntegration;
pub use relay::{
    RelayDelivery, RelayReply, init_relay, relay_binary, set_relay_channels, take_relay_requests,
};
pub use sessions_panel::{AgentSessionsPanel, ToggleAgentSessions};
pub use settings::{FyxTerminalSettings, init_shell_integration};
pub use t3::{Share, ShareBridge, ShareTarget, set_share_bridge, shares_changed};

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{Entity, Global, WeakEntity};
use terminal::Terminal;
use terminal_ext::TerminalId;

pub use host::{HostFront, TerminalClosedHook};

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

/// §C.3: told every terminal that closes (by [`TerminalId::as_u64`]), so the consent broker
/// drops its shares and ends its open cards. Set once by `main.rs`.
pub fn set_terminal_closed_hook(hook: TerminalClosedHook, cx: &App) {
    if let Some(front) = front(cx) {
        front.set_closed_hook(hook);
    }
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
        host::Command::Integration(ext, integration) => profiles::set(ext, integration, cx),
        host::Command::Input(id, bytes) => created::input(id, bytes, cx),
        host::Command::Paste(id, bytes) => created::paste(id, bytes, cx),
        host::Command::Read {
            id,
            scope,
            lines,
            reply,
        } => t3::read(id, scope, lines, reply, cx),
        host::Command::Write {
            id,
            extension,
            text,
            submit,
            reply,
        } => t3::write(id, extension, text, submit, reply, cx),
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

/// §B.11 focus rule: whether one of an extension's terminals (its pseudo or created terminals,
/// its profiles' terminals) has keyboard focus now. Callable from any thread.
pub type TerminalFocus = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The [`TerminalFocus`] for `ui_ext_host`'s editor context (`workspace.open` with `focus`).
pub fn terminal_focus(cx: &App) -> Option<TerminalFocus> {
    let front = front(cx)?;
    Some(Arc::new(move |ext: &str| {
        front.extension_terminal_focused(ext)
    }))
}

/// For a [`RelayDelivery`]: the handle its extension holds for the terminal the hook runs in, or
/// `None` when the extension has never been told of that terminal.
pub fn relay_terminal_handle(
    ext: &str,
    terminal: TerminalId,
    cx: &App,
) -> Option<ui_ext::terminal::TerminalHandle> {
    front(cx)?.existing_handle(ext, terminal)
}

/// For `ui_ext_host::set_terminal_node_views`: [`extension_terminal`] in a terminal view of the
/// window's workspace, for an extension panel's `terminal` node. The caller keeps the view while a
/// node shows the handle.
pub fn extension_terminal_view(
    ext: &str,
    handle: ui_ext::terminal::TerminalHandle,
    window: &mut gpui::Window,
    cx: &mut App,
) -> Option<gpui::AnyView> {
    let terminal = extension_terminal(ext, handle, cx)?;
    let workspace = Workspace::for_window(window, cx)?;
    let (weak, database_id, project) = {
        let workspace = workspace.read(cx);
        (
            workspace.weak_handle(),
            workspace.database_id(),
            workspace.project().downgrade(),
        )
    };
    let view = gpui::AppContext::new(cx, |cx| {
        terminal_view::TerminalView::new(terminal, weak, database_id, project, window, cx)
    });
    Some(view.into())
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
