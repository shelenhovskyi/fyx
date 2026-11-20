//! Host side of the staged `fyx:ui@0.4.0` world (`crates/ui_ext/wit/since_v0.4.0/fyx-ui.wit`,
//! docs/fyx/wit-0.4/design.md): everything 0.3 has (`t2_shared.rs`), the ten T2 cases of
//! `permissions`, `terminals` over [`crate::terminals`], `approvals` (§B.12) and `listeners`
//! over [`crate::listeners`] (§B.7), `context` over [`crate::editor_context`] (§B.11) and `review`
//! over [`crate::proposals`] (§B.10), and `relay` over [`crate::relay`] (§B.9).
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
    agent_observe as wit_observe, approvals as wit_approvals, context as wit_context,
    decorations as wit_decorations, host as wit_host, images as wit_images, io as wit_io,
    listeners as wit_listeners, network as wit_network, notifications as wit_notifications,
    permissions as wit_permissions, process as wit_process, relay as wit_relay,
    review as wit_review, settings as wit_settings, terminals as wit_terminals, types as wit,
    workspace_edit as wit_edit,
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

/// The 0.4 `io-event::relay-request` (design wit-0.4 §B.9).
fn relay_io_event(request: ui_ext::integration::RelayRequest) -> Option<wit_io::IoEvent> {
    Some(wit_io::IoEvent::RelayRequest((
        request.id,
        request.channel,
        request.terminal.map(|handle| handle.0),
        request.argv,
        request.stdin,
    )))
}

/// The 0.4 `io-event` cases of a listener's event (design wit-0.4 §B.7).
fn listener_io_event(event: ui_ext::integration::ListenerEvent) -> Option<wit_io::IoEvent> {
    use ui_ext::integration::ListenerEvent as Event;
    Some(match event {
        Event::McpCall(call) => wit_io::IoEvent::McpCall(wit_listeners::McpCall {
            call_id: call.call_id,
            listener: call.listener,
            connection: call.connection,
            tool: call.tool,
            arguments: call.arguments,
        }),
        Event::McpCancelled(id) => wit_io::IoEvent::McpCancelled(id),
        Event::McpInitialized(session) => {
            wit_io::IoEvent::McpInitialized(wit_listeners::McpSession {
                listener: session.listener,
                connection: session.connection,
                client_name: session.client_name,
                client_version: session.client_version,
            })
        }
        Event::Message {
            listener,
            connection,
            text,
        } => wit_io::IoEvent::ListenerMessage((listener, connection, text)),
        Event::Request {
            id,
            listener,
            path,
            body,
        } => wit_io::IoEvent::ListenerRequest((id, listener, path, body)),
    })
}

fn text_range(range: wit::TextRange) -> ui_ext::edit::TextRange {
    ui_ext::edit::TextRange {
        start: position(range.start),
        end: position(range.end),
    }
}

fn wit_text_range(range: ui_ext::edit::TextRange) -> wit::TextRange {
    let position = |position: ui_ext::edit::Position| wit::Position {
        line: position.line,
        column: position.column,
    };
    wit::TextRange {
        start: position(range.start),
        end: position(range.end),
    }
}

/// The `context-changed` case (design wit-0.4 §B.11).
fn context_io_event(context: ui_ext::context::EditorContext) -> Option<wit_io::IoEvent> {
    Some(wit_io::IoEvent::ContextChanged(
        wit_context::EditorContext {
            active: context.active.map(wit_file_ref),
            open: context.open.into_iter().map(wit_file_ref).collect(),
            selection: context
                .selection
                .map(|(range, text)| (wit_text_range(range), text)),
            trusted: context.trusted,
        },
    ))
}

/// The `diagnostics-changed` case (design wit-0.4 §B.11).
fn diagnostics_io_event(files: Vec<ui_ext::process::FileRef>) -> Option<wit_io::IoEvent> {
    Some(wit_io::IoEvent::DiagnosticsChanged(
        files.into_iter().map(wit_file_ref).collect(),
    ))
}

fn wit_diagnostics(
    snapshot: ui_ext::context::DiagnosticsSnapshot,
) -> wit_context::DiagnosticsSnapshot {
    use ui_ext::context::DiagnosticSeverity;
    wit_context::DiagnosticsSnapshot {
        files: snapshot
            .files
            .into_iter()
            .map(|file| wit_context::FileDiagnostics {
                path: wit_file_ref(file.path),
                diagnostics: file
                    .diagnostics
                    .into_iter()
                    .map(|entry| wit_context::Diagnostic {
                        range: wit_text_range(entry.range),
                        severity: match entry.severity {
                            DiagnosticSeverity::Error => wit_context::Severity::Error,
                            DiagnosticSeverity::Warning => wit_context::Severity::Warning,
                            DiagnosticSeverity::Information => wit_context::Severity::Information,
                            DiagnosticSeverity::Hint => wit_context::Severity::Hint,
                        },
                        message: entry.message,
                        source: entry.source,
                        code: entry.code,
                    })
                    .collect(),
            })
            .collect(),
        truncated: snapshot.truncated,
    }
}

/// The `review-decided` case (design wit-0.4 §B.10).
fn review_io_event(
    id: u64,
    result: Result<ui_ext::review::ReviewOutcome, String>,
) -> Option<wit_io::IoEvent> {
    use ui_ext::review::Decision;
    Some(wit_io::IoEvent::ReviewDecided((
        id,
        result.map(|outcome| wit_review::ReviewOutcome {
            decision: match outcome.decision {
                Decision::Accepted => wit_review::Decision::Accepted,
                Decision::Partial => wit_review::Decision::Partial,
                Decision::Rejected => wit_review::Decision::Rejected,
            },
            final_content: outcome.final_content,
        }),
    )))
}

/// `context` (design.md §B.11) over [`crate::editor_context::ContextHandle`]: `watch`,
/// `to-file-ref` and `absolute-path` need `context:editor`, `open` needs `workspace:open`,
/// `diagnostics` and `watch-diagnostics` need `workspace:diagnostics`.
impl wit_context::Host for UiState {
    fn watch(&mut self, on: bool) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        if on && let Err(error) = self.permit_t1(ui_ext::context::CAPABILITY_CONTEXT_EDITOR) {
            return Ok(Err(error.into()));
        }
        Ok(self.context.watch(on).map_err(Into::into))
    }

    fn open(
        &mut self,
        path: wit::FileRef,
        range: Option<wit::TextRange>,
        focus: bool,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(path.path.len())?;
        if let Err(error) = self.permit_t1(ui_ext::context::CAPABILITY_WORKSPACE_OPEN) {
            return Ok(Err(error.into()));
        }
        let gesture = focus && self.api.take_activation();
        Ok(self
            .context
            .open(file_ref(path), range.map(text_range), focus, gesture)
            .map_err(Into::into))
    }

    fn to_file_ref(
        &mut self,
        absolute: String,
    ) -> wasmtime::Result<Result<wit::FileRef, wit_host::HostError>> {
        self.count_lifted_text(absolute.len())?;
        if let Err(error) = self.permit_t1(ui_ext::context::CAPABILITY_CONTEXT_EDITOR) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .context
            .to_file_ref(&absolute)
            .map(wit_file_ref)
            .map_err(Into::into))
    }

    fn absolute_path(
        &mut self,
        path: wit::FileRef,
    ) -> wasmtime::Result<Result<String, wit_host::HostError>> {
        self.count_lifted_text(path.path.len())?;
        if let Err(error) = self.permit_t1(ui_ext::context::CAPABILITY_CONTEXT_EDITOR) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .context
            .absolute_path(&file_ref(path))
            .map_err(Into::into))
    }

    fn diagnostics(
        &mut self,
        path: Option<wit::FileRef>,
    ) -> wasmtime::Result<Result<wit_context::DiagnosticsSnapshot, wit_host::HostError>> {
        self.count_lifted_text(path.as_ref().map_or(0, |path| path.path.len()))?;
        if let Err(error) = self.permit_t1(ui_ext::context::CAPABILITY_WORKSPACE_DIAGNOSTICS) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .context
            .diagnostics(path.map(file_ref))
            .map(wit_diagnostics)
            .map_err(Into::into))
    }

    fn watch_diagnostics(&mut self, on: bool) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        if on && let Err(error) = self.permit_t1(ui_ext::context::CAPABILITY_WORKSPACE_DIAGNOSTICS)
        {
            return Ok(Err(error.into()));
        }
        Ok(self.context.watch_diagnostics(on).map_err(Into::into))
    }

    // Staged in the 0.4 WIT ahead of the host side (plan 6.6 §E: W2c checks, W3 collection;
    // plan 6.7 §F.2: W3 project panel).
    fn publish_diagnostics(
        &mut self,
        path: wit::FileRef,
        diagnostics: Vec<wit_context::Diagnostic>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(
            path.path.len()
                + diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.len())
                    .sum::<usize>(),
        )?;
        if let Err(error) = self.permit_t1(ui_ext::integration::CAPABILITY_DIAGNOSTICS_PUBLISH) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .marks
            .publish_diagnostics(
                file_ref(path),
                diagnostics.into_iter().map(model_diagnostic).collect(),
            )
            .map_err(Into::into))
    }

    fn clear_diagnostics(
        &mut self,
        path: Option<wit::FileRef>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(path.as_ref().map_or(0, |path| path.path.len()))?;
        if let Err(error) = self.permit_t1(ui_ext::integration::CAPABILITY_DIAGNOSTICS_PUBLISH) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .marks
            .clear_diagnostics(path.map(file_ref))
            .map_err(Into::into))
    }

    fn decorate_files(
        &mut self,
        decorations: Vec<wit_context::FileDecoration>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(
            decorations
                .iter()
                .map(|decoration| {
                    decoration.path.path.len()
                        + decoration.badge.as_ref().map_or(0, String::len)
                        + decoration.tooltip.as_ref().map_or(0, String::len)
                })
                .sum(),
        )?;
        if let Err(error) = self.permit_t1(ui_ext::integration::CAPABILITY_FILE_DECORATIONS) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .marks
            .decorate_files(
                decorations
                    .into_iter()
                    .map(|decoration| ui_ext::file_decorations::FileDecoration {
                        path: file_ref(decoration.path),
                        badge: decoration.badge,
                        color: decoration.color.map(Into::into),
                        tooltip: decoration.tooltip,
                    })
                    .collect(),
            )
            .map_err(Into::into))
    }

    fn clear_file_decorations(
        &mut self,
        paths: Option<Vec<wit::FileRef>>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(
            paths
                .as_ref()
                .map_or(0, |paths| paths.iter().map(|path| path.path.len()).sum()),
        )?;
        if let Err(error) = self.permit_t1(ui_ext::integration::CAPABILITY_FILE_DECORATIONS) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .marks
            .clear_file_decorations(paths.map(|paths| paths.into_iter().map(file_ref).collect()))
            .map_err(Into::into))
    }
}

/// A published `diagnostic` (§E.1); `source` is dropped by the checks.
fn model_diagnostic(entry: wit_context::Diagnostic) -> ui_ext::context::Diagnostic {
    use ui_ext::context::DiagnosticSeverity;
    ui_ext::context::Diagnostic {
        range: text_range(entry.range),
        severity: match entry.severity {
            wit_context::Severity::Error => DiagnosticSeverity::Error,
            wit_context::Severity::Warning => DiagnosticSeverity::Warning,
            wit_context::Severity::Information => DiagnosticSeverity::Information,
            wit_context::Severity::Hint => DiagnosticSeverity::Hint,
        },
        message: entry.message,
        source: entry.source,
        code: entry.code,
    }
}

/// `review` (design.md §B.10) over [`crate::proposals::ReviewHandle`]: every call needs
/// `review:propose`; accepting or rejecting stays the user's.
impl wit_review::Host for UiState {
    fn propose(
        &mut self,
        request: wit_review::Proposal,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        let change_len = match &request.change {
            wit_review::Proposed::NewContent(text) | wit_review::Proposed::UnifiedDiff(text) => {
                text.len()
            }
        };
        self.count_lifted_text(
            request.path.path.len() + change_len + request.title.len() + request.session.len(),
        )?;
        if let Err(error) = self.permit_t1(ui_ext::review::CAPABILITY_REVIEW_PROPOSE) {
            return Ok(Err(error.into()));
        }
        let proposal = ui_ext::review::Proposal {
            path: file_ref(request.path),
            change: match request.change {
                wit_review::Proposed::NewContent(text) => {
                    ui_ext::review::Proposed::NewContent(text)
                }
                wit_review::Proposed::UnifiedDiff(text) => {
                    ui_ext::review::Proposed::UnifiedDiff(text)
                }
            },
            title: request.title,
            session: request.session,
            apply: request.apply,
        };
        Ok(self.review.propose(proposal).map_err(Into::into))
    }

    fn withdraw(&mut self, id: u64) -> wasmtime::Result<Result<String, wit_host::HostError>> {
        if let Err(error) = self.permit_t1(ui_ext::review::CAPABILITY_REVIEW_PROPOSE) {
            return Ok(Err(error.into()));
        }
        Ok(self.review.withdraw(id).map_err(Into::into))
    }

    fn begin_turn(&mut self, session: String) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(session.len())?;
        if let Err(error) = self.permit_t1(ui_ext::review::CAPABILITY_REVIEW_PROPOSE) {
            return Ok(Err(error.into()));
        }
        Ok(self.review.begin_turn(&session).map_err(Into::into))
    }

    fn end_turn(
        &mut self,
        session: String,
        changed: Vec<wit::FileRef>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(
            session.len() + changed.iter().map(|file| file.path.len()).sum::<usize>(),
        )?;
        if let Err(error) = self.permit_t1(ui_ext::review::CAPABILITY_REVIEW_PROPOSE) {
            return Ok(Err(error.into()));
        }
        Ok(self
            .review
            .end_turn(&session, changed.into_iter().map(file_ref).collect())
            .map_err(Into::into))
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

/// `settings` (design.md §D.4) over [`crate::extension_settings::SettingsHandle`]: no
/// capability; the keys are the manifest's `[[fyx_ui.settings]]`, the values the user's (user
/// settings file only), checked, else the defaults.
impl wit_settings::Host for UiState {
    fn get(
        &mut self,
        key: String,
    ) -> wasmtime::Result<Result<wit_settings::SettingValue, wit_host::HostError>> {
        self.count_lifted_text(key.len())?;
        Ok(self
            .settings
            .get(&key)
            .map(setting_value)
            .map_err(Into::into))
    }

    fn all(&mut self) -> wasmtime::Result<Vec<wit_settings::Setting>> {
        Ok(self
            .settings
            .all()
            .into_iter()
            .map(|setting| wit_settings::Setting {
                key: setting.key,
                value: setting_value(setting.value),
                is_default: setting.is_default,
            })
            .collect())
    }

    /// Only while the user acts on one of the extension's surfaces (an event call), else
    /// `not-permitted`; `invalid-argument` for an undeclared key.
    fn open(&mut self, key: Option<String>) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(key.as_ref().map_or(0, String::len))?;
        Ok(self
            .api
            .open_settings(&self.settings, key)
            .map_err(Into::into))
    }
}

fn setting_value(value: ui_ext::settings::SettingValue) -> wit_settings::SettingValue {
    use ui_ext::settings::SettingValue as Value;
    match value {
        Value::Boolean(value) => wit_settings::SettingValue::Boolean(value),
        Value::Integer(value) => wit_settings::SettingValue::Integer(value),
        Value::Number(value) => wit_settings::SettingValue::Number(value),
        Value::Text(value) => wit_settings::SettingValue::Text(value),
        Value::Choice(value) => wit_settings::SettingValue::Choice(value),
        Value::TextList(value) => wit_settings::SettingValue::TextList(value),
    }
}

/// The `settings-changed` case.
fn settings_io_event(keys: Vec<String>) -> Option<wit_io::IoEvent> {
    Some(wit_io::IoEvent::SettingsChanged(keys))
}

/// `listeners` (design.md §B.7) over [`crate::listeners::ListenersHandle`]: a listener the
/// manifest does not declare is `not-permitted`, one that is not granted `denied`, and one that
/// is granted but not open (yet) `unavailable`. Tools registered before it opens apply when it
/// does.
impl wit_listeners::Host for UiState {
    fn register_tools(
        &mut self,
        listener: String,
        tools: Vec<wit_listeners::ToolDef>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(
            listener.len()
                + tools
                    .iter()
                    .map(|tool| tool.name.len() + tool.description.len() + tool.input_schema.len())
                    .sum::<usize>(),
        )?;
        let tools = tools
            .into_iter()
            .map(|tool| ui_ext::integration::ToolDef {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            })
            .collect();
        Ok(self
            .listeners
            .register_tools(&listener, tools)
            .map_err(Into::into))
    }

    fn respond(
        &mut self,
        call_id: u64,
        outcome: Result<String, wit_listeners::McpError>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(match &outcome {
            Ok(text) => text.len(),
            Err(error) => error.message.len(),
        })?;
        let outcome = outcome.map_err(|error| ui_ext::integration::McpError {
            code: error.code,
            message: error.message,
        });
        Ok(self.listeners.respond(call_id, outcome).map_err(Into::into))
    }

    fn notify(
        &mut self,
        listener: String,
        connection: Option<u64>,
        method: String,
        params: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(listener.len() + method.len() + params.len())?;
        Ok(self
            .listeners
            .notify(&listener, connection, &method, &params)
            .map_err(Into::into))
    }

    fn send(
        &mut self,
        listener: String,
        connection: u64,
        text: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(listener.len() + text.len())?;
        Ok(self
            .listeners
            .send(&listener, connection, &text)
            .map_err(Into::into))
    }

    fn reply(
        &mut self,
        request_id: u64,
        status: u16,
        body: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(body.len())?;
        Ok(self
            .listeners
            .reply(request_id, status, body)
            .map_err(Into::into))
    }
}

/// `relay` (design.md §B.9) over [`crate::relay::RelayHandle`].
impl wit_relay::Host for UiState {
    fn respond(
        &mut self,
        id: u64,
        exit_code: u8,
        stdout: String,
        stderr: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(stdout.len() + stderr.len())?;
        self.permission_state(ui_ext::integration::CAPABILITY_RELAY_HOOKS);
        Ok(self
            .relay
            .respond(
                id,
                ui_ext::integration::RelayAnswer {
                    exit: exit_code,
                    stdout,
                    stderr,
                },
            )
            .map_err(Into::into))
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
        model::UiEvent::RowToggled(id, row, expand) => Self::RowToggled((id, row, expand)),
    }
}

/// A 0.4 row's chevron (§F.4).
fn list_row_expansion(row: &wit::ListRow) -> model::Expansion {
    match row.expansion {
        wit::Expansion::Leaf => model::Expansion::Leaf,
        wit::Expansion::Collapsed => model::Expansion::Collapsed,
        wit::Expansion::Expanded => model::Expansion::Expanded,
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
    "listeners",
    "context",
    "review",
    "relay",
    "settings",
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
    wit_listeners::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_context::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_review::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_relay::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_settings::add_to_linker::<_, UiState>(linker, |state| state)?;
    Ok(())
}
