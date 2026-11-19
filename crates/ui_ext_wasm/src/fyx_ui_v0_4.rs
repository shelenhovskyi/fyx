//! Host side of the staged `fyx:ui@0.4.0` world (`crates/ui_ext/wit/since_v0.4.0/fyx-ui.wit`,
//! docs/fyx/wit-0.4/design.md): everything 0.3 has (`t2_shared.rs`), the ten T2 cases of
//! `permissions`, `terminals` over [`crate::terminals`], and `approvals` (§B.12). `listeners`,
//! `relay`, `review` and `context` trap when called until plan 5.11 and 5.13 build them
//! (`define_unbuilt_as_traps`).
//!
//! A `terminal` node becomes [`model::Props::Terminal`]; the renderer resolves its handle.

use ui_ext::{
    canvas as model_canvas,
    image::{self as model_image},
    terminal as model_terminal, v0_2 as model,
};

use crate::{UiState, conversions, edit, network, observe, permissions::PermissionState, process};

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit/since_v0.4.0",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
        exports: {
            default: async,
        },
    });
}

pub(crate) use bindings::UiExtension;
use bindings::fyx::ui::{
    agent_observe as wit_observe, approvals as wit_approvals, decorations as wit_decorations,
    host as wit_host, images as wit_images, io as wit_io, network as wit_network,
    notifications as wit_notifications, permissions as wit_permissions, process as wit_process,
    terminals as wit_terminals, types as wit, workspace_edit as wit_edit,
};

conversions::host_imports!(
    denied: wit_host::HostErrorCode::Denied,
    untrusted_worktree: wit_host::HostErrorCode::UntrustedWorktree,
    no_worktree: wit_host::HostErrorCode::NoWorktree
);
conversions::notification_imports!();
conversions::decoration_imports!();

include!("t2_shared.rs");

fn t2_capability(capability: wit_permissions::T2Capability) -> &'static str {
    use wit_permissions::T2Capability as T2;
    match capability {
        T2::NetworkFetch => network::CAPABILITY_NETWORK,
        T2::ProcessSpawn => process::CAPABILITY_PROCESS,
        T2::WorkspaceEdit => edit::CAPABILITY_EDIT,
        T2::AgentObserve => observe::CAPABILITY_OBSERVE,
        T2::NetworkListen => ui_ext::integration::CAPABILITY_NETWORK_LISTEN,
        T2::TerminalCreate => model_terminal::CAPABILITY_CREATE,
        T2::TerminalEnv => model_terminal::CAPABILITY_ENV,
        T2::TerminalObserve => model_terminal::CAPABILITY_OBSERVE,
        T2::FsDiscoveryFile => ui_ext::integration::CAPABILITY_DISCOVERY_FILE,
        T2::RelayHooks => ui_ext::integration::CAPABILITY_RELAY_HOOKS,
    }
}

fn t2_from_name(capability: &str) -> Option<wit_permissions::T2Capability> {
    use wit_permissions::T2Capability as T2;
    [
        T2::NetworkFetch,
        T2::ProcessSpawn,
        T2::WorkspaceEdit,
        T2::AgentObserve,
        T2::NetworkListen,
        T2::TerminalCreate,
        T2::TerminalEnv,
        T2::TerminalObserve,
        T2::FsDiscoveryFile,
        T2::RelayHooks,
    ]
    .into_iter()
    .find(|case| t2_capability(*case) == capability)
}

fn terminal_handle(handle: u64) -> model_terminal::TerminalHandle {
    model_terminal::TerminalHandle(handle)
}

fn status(status: wit_terminals::TerminalStatus) -> model_terminal::TerminalStatus {
    use wit_terminals::StatusState as State;
    model_terminal::TerminalStatus {
        state: match status.state {
            State::Idle => model_terminal::StatusState::Idle,
            State::Working => model_terminal::StatusState::Working,
            State::Blocked => model_terminal::StatusState::Blocked,
            State::Done => model_terminal::StatusState::Done,
            State::Error => model_terminal::StatusState::Error,
        },
        label: status.label,
        detail: status.detail,
        progress: status.progress,
        attention: status.attention,
    }
}

fn wit_status(status: model_terminal::TerminalStatus) -> wit_terminals::TerminalStatus {
    use model_terminal::StatusState as State;
    wit_terminals::TerminalStatus {
        state: match status.state {
            State::Idle => wit_terminals::StatusState::Idle,
            State::Working => wit_terminals::StatusState::Working,
            State::Blocked => wit_terminals::StatusState::Blocked,
            State::Done => wit_terminals::StatusState::Done,
            State::Error => wit_terminals::StatusState::Error,
        },
        label: status.label,
        detail: status.detail,
        progress: status.progress,
        attention: status.attention,
    }
}

fn wit_info(info: model_terminal::TerminalInfo) -> wit_terminals::TerminalInfo {
    wit_terminals::TerminalInfo {
        handle: info.handle.0,
        origin: match info.origin {
            model_terminal::TerminalOrigin::Pseudo => wit_terminals::TerminalOrigin::Pseudo,
            model_terminal::TerminalOrigin::Created => wit_terminals::TerminalOrigin::Created,
            model_terminal::TerminalOrigin::Profile => wit_terminals::TerminalOrigin::Profile,
            model_terminal::TerminalOrigin::User => wit_terminals::TerminalOrigin::User,
        },
        title: info.title,
        cwd: info.cwd,
        status: info.status.map(wit_status),
        account: info.account,
    }
}

/// The 0.4 `io-event` cases of a terminal event.
fn terminal_io_event(event: model_terminal::TerminalEvent) -> Option<wit_io::IoEvent> {
    use model_terminal::TerminalEvent as Event;
    Some(match event {
        Event::Input(handle, bytes) => wit_io::IoEvent::TerminalInput((handle.0, bytes)),
        Event::Resized(handle, cols, rows) => {
            wit_io::IoEvent::TerminalResized((handle.0, cols, rows))
        }
        Event::Output(handle, bytes, dropped) => {
            wit_io::IoEvent::TerminalOutput((handle.0, bytes, dropped))
        }
        Event::Exited(handle, code) => wit_io::IoEvent::TerminalExited((handle.0, code)),
        Event::Created(info) => wit_io::IoEvent::TerminalCreated(wit_info(info)),
        Event::Closed(handle) => wit_io::IoEvent::TerminalClosed(handle.0),
        Event::StatusChanged(info) => wit_io::IoEvent::TerminalStatusChanged(wit_info(info)),
        Event::ReadDone(id, result) => wit_io::IoEvent::TerminalReadDone((id, result)),
        Event::WriteDone(id, result) => wit_io::IoEvent::TerminalWriteDone((id, result)),
    })
}

fn approval_option(option: wit_approvals::ApprovalOption) -> ui_ext::approvals::ApprovalOption {
    use ui_ext::approvals::ApprovalOption as Option;
    match option {
        wit_approvals::ApprovalOption::AllowOnce => Option::AllowOnce,
        wit_approvals::ApprovalOption::AlwaysAllow => Option::AlwaysAllow,
        wit_approvals::ApprovalOption::Deny => Option::Deny,
    }
}

/// The `approval-answered` case; `None` when the card closed unanswered.
fn approval_io_event(
    id: u64,
    answer: Option<ui_ext::approvals::ApprovalOption>,
) -> Option<wit_io::IoEvent> {
    use ui_ext::approvals::ApprovalOption as Answer;
    Some(wit_io::IoEvent::ApprovalAnswered((
        id,
        answer.map(|answer| match answer {
            Answer::AllowOnce => wit_approvals::ApprovalOption::AllowOnce,
            Answer::AlwaysAllow => wit_approvals::ApprovalOption::AlwaysAllow,
            Answer::Deny => wit_approvals::ApprovalOption::Deny,
        }),
    )))
}

/// `approvals` (design.md §B.12): a T0 card; the answer comes back through `handle-io`.
impl wit_approvals::Host for UiState {
    fn ask(
        &mut self,
        tool: String,
        title: String,
        detail: String,
        options: Vec<wit_approvals::ApprovalOption>,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(tool.len() + title.len() + detail.len())?;
        let request = ui_ext::approvals::ApprovalRequest {
            tool: (!tool.is_empty()).then_some(tool),
            title,
            detail,
            options: options.into_iter().map(approval_option).collect(),
        };
        Ok(self.api.ask_approval(request).map_err(Into::into))
    }
}

/// `terminals` (design.md §B.1–§B.5, §B.11, §C.3) over [`crate::terminals::TerminalState`].
impl wit_terminals::Host for UiState {
    fn open_pseudo(
        &mut self,
        request: wit_terminals::PseudoRequest,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(request.title.len())?;
        if let Err(error) = self.permit_t1(model_terminal::CAPABILITY_PSEUDO) {
            return Ok(Err(error.into()));
        }
        let request = model_terminal::PseudoRequest {
            title: request.title,
            cols: request.cols,
            rows: request.rows,
            in_panel: request.in_panel,
        };
        Ok(self
            .terminals
            .open_pseudo(request)
            .map(|handle| handle.0)
            .map_err(Into::into))
    }

    fn write_output(
        &mut self,
        handle: u64,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self
            .terminals
            .write_output(terminal_handle(handle), bytes)
            .map_err(Into::into))
    }

    fn create(
        &mut self,
        request: wit_terminals::CreateRequest,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(
            request.program.len()
                + request.args.iter().map(String::len).sum::<usize>()
                + request.cwd.as_ref().map_or(0, String::len)
                + request.title.as_ref().map_or(0, String::len),
        )?;
        if let Err(error) = self.permit_t2_now(model_terminal::CAPABILITY_CREATE) {
            return Ok(Err(error.into()));
        }
        // §C.2: the program's own checkbox. An undeclared program is the import's error below.
        let item = crate::permissions::ConsentItem::Program {
            capability: model_terminal::CAPABILITY_CREATE.into(),
            name: request.program.clone(),
        };
        if self.item_denied(&item) {
            return Ok(Err(model::HostError {
                code: model::HostErrorCode::Denied,
                message: format!("{} is turned off on the extension page", request.program),
            }
            .into()));
        }
        let env_granted = self.t2_granted(model_terminal::CAPABILITY_ENV);
        let started = (
            request.program.clone(),
            request.args.clone(),
            request.cwd.clone(),
        );
        // A worktree-relative cwd in the active (first) worktree, as `process.spawn`'s `cwd`
        // with no worktree id.
        let cwd = request.cwd.map(|path| process::FileRef {
            worktree: None,
            path,
        });
        let created = self.terminals.create(
            env_granted,
            request.program,
            request.args,
            cwd,
            request.title,
        );
        if created.is_ok() {
            let (program, args, cwd) = started;
            self.journal(crate::journal::JournalKind::ProgramStarted {
                capability: model_terminal::CAPABILITY_CREATE.into(),
                program,
                args,
                cwd,
            });
        }
        Ok(created.map(|handle| handle.0).map_err(Into::into))
    }

    fn send_input(
        &mut self,
        handle: u64,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self
            .terminals
            .send_input(terminal_handle(handle), bytes)
            .map_err(Into::into))
    }

    fn close(&mut self, handle: u64) -> wasmtime::Result<()> {
        self.terminals.close(terminal_handle(handle));
        Ok(())
    }

    fn set_status(
        &mut self,
        handle: u64,
        new_status: Option<wit_terminals::TerminalStatus>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(new_status.as_ref().map_or(0, |status| {
            status.label.len() + status.detail.as_ref().map_or(0, String::len)
        }))?;
        let handle = terminal_handle(handle);
        if !self.terminals.owns(handle)
            && let Err(error) = self.permit_t1(model_terminal::CAPABILITY_STATUS)
        {
            return Ok(Err(error.into()));
        }
        Ok(self
            .terminals
            .set_status(handle, new_status.map(status))
            .map_err(Into::into))
    }

    fn list(
        &mut self,
    ) -> wasmtime::Result<Result<Vec<wit_terminals::TerminalInfo>, wit_host::HostError>> {
        if let Err(error) = self.permit_t2_now(model_terminal::CAPABILITY_OBSERVE) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .terminals
            .list()
            .map(|infos| infos.into_iter().map(wit_info).collect())
            .map_err(Into::into))
    }

    fn paste(
        &mut self,
        handle: u64,
        text: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(text.len())?;
        let gesture = self.api.take_activation();
        Ok(self
            .terminals
            .paste(gesture, terminal_handle(handle), text)
            .map_err(Into::into))
    }

    /// T3 (§C.3): checked and queued now; the card and the read start after the call returns.
    fn request_read(
        &mut self,
        handle: u64,
        scope: wit_terminals::ReadScope,
        lines: u16,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        if !self.api.declares(model_terminal::CAPABILITY_READ) {
            return Ok(Err(not_declared(model_terminal::CAPABILITY_READ)));
        }
        let gesture = self.api.take_activation();
        let scope = match scope {
            wit_terminals::ReadScope::LastCommand => model_terminal::ReadScope::LastCommand,
            wit_terminals::ReadScope::LastLines => model_terminal::ReadScope::LastLines,
        };
        Ok(self
            .terminals
            .request_read(gesture, terminal_handle(handle), scope, lines)
            .map_err(Into::into))
    }

    fn request_write(
        &mut self,
        handle: u64,
        text: String,
        submit: bool,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(text.len())?;
        if !self.api.declares(model_terminal::CAPABILITY_WRITE) {
            return Ok(Err(not_declared(model_terminal::CAPABILITY_WRITE)));
        }
        let gesture = self.api.take_activation();
        Ok(self
            .terminals
            .request_write(gesture, terminal_handle(handle), text, submit)
            .map_err(Into::into))
    }
}

fn not_declared(capability: &str) -> wit_host::HostError {
    model::HostError {
        code: model::HostErrorCode::NotPermitted,
        message: format!("{capability:?} is not in the manifest"),
    }
    .into()
}

conversions::conversions! {
    props: {
        wit::Props::Image(props) => Self::Image(model_image::ImageProps {
            source: match props.source {
                wit::ImageSource::Asset(path) => model_image::ImageSource::Asset(path),
                wit::ImageSource::Generated(handle) => model_image::ImageSource::Generated(handle),
            },
            alt: props.alt,
            width: props.width,
            height: props.height,
            fit: match props.fit {
                wit::ImageFit::Contain => model_image::ImageFit::Contain,
                wit::ImageFit::Cover => model_image::ImageFit::Cover,
            },
            tint: props.tint.map(Into::into),
        }),
        wit::Props::Canvas(props) => Self::Canvas(canvas(props)),
        wit::Props::Terminal(props) => Self::Terminal(model::TerminalNodeProps {
            handle: props.handle,
        }),
    }
    kinds: {}
    events: {
        model::UiEvent::CanvasClick(click) => Self::CanvasClick(wit::CanvasClick {
            canvas: click.canvas,
            x: click.x.0,
            y: click.y.0,
        }),
    }
}

/// The `fyx:ui` 0.4 interfaces [`add_to_linker`] links; the rest trap when called.
pub(crate) const LINKED_INTERFACES: &[&str] = &[
    "types",
    "host",
    "decorations",
    "images",
    "notifications",
    "network",
    "process",
    "workspace-edit",
    "agent-observe",
    "permissions",
    "terminals",
    "approvals",
];

/// The 0.4 linker: WASI and the interfaces above.
pub(crate) fn add_to_linker(
    linker: &mut wasmtime::component::Linker<UiState>,
) -> wasmtime::Result<()> {
    wit::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_host::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_decorations::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_images::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_notifications::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_network::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_process::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_edit::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_observe::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_permissions::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_terminals::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_approvals::add_to_linker::<_, UiState>(linker, |state| state)?;
    Ok(())
}
