// The imports 0.3 and 0.4 share, included by `fyx_ui_v0_3` and `fyx_ui_v0_4` with each world's
// bindings in scope (`wit`, `wit_host`, `wit_images`, ...): both worlds spell these types the same.
// A module that includes it defines `t2_capability`, `t2_from_name`, `terminal_io_event`,
// `listener_io_event` and `approval_io_event`.

impl wit_images::Host for UiState {
    fn add_image(
        &mut self,
        format: wit_images::ImageFormat,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<u32, wit_host::HostError>> {
        if let Err(error) = self.permit(model_image::CAPABILITY_IMAGES) {
            return Ok(Err(error.into()));
        }
        let format = match format {
            wit_images::ImageFormat::Png => model_image::ImageFormat::Png,
            wit_images::ImageFormat::Svg => model_image::ImageFormat::Svg,
        };
        Ok(self.api.add_image(format, bytes).map_err(Into::into))
    }

    fn remove_image(&mut self, image: u32) -> wasmtime::Result<()> {
        self.api.remove_image(image);
        Ok(())
    }
}

impl wit_network::Host for UiState {
    fn fetch(
        &mut self,
        request: wit_network::HttpRequest,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(
            request.url.len()
                + request
                    .headers
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>(),
        )?;
        if !self.api.declares(network::CAPABILITY_NETWORK) {
            return Ok(Err(model::HostError {
                code: model::HostErrorCode::NotPermitted,
                message: format!("{:?} is not in the manifest", network::CAPABILITY_NETWORK),
            }
            .into()));
        }
        // The broker at every call. `ask` is accepted: the prompt comes when the request is
        // applied, after this call returns (design.md §0.3, §7).
        match self.permission_state(network::CAPABILITY_NETWORK) {
            PermissionState::Granted | PermissionState::Ask => {}
            state @ (PermissionState::Denied | PermissionState::Unavailable) => {
                return Ok(Err(model::HostError {
                    code: model::HostErrorCode::Denied,
                    message: format!("{} is {state:?}", network::CAPABILITY_NETWORK),
                }
                .into()));
            }
        }
        let request = network::FetchRequest {
            method: match request.method {
                wit_network::HttpMethod::Get => network::Method::Get,
                wit_network::HttpMethod::Head => network::Method::Head,
                wit_network::HttpMethod::Post => network::Method::Post,
                wit_network::HttpMethod::Put => network::Method::Put,
                wit_network::HttpMethod::Patch => network::Method::Patch,
                wit_network::HttpMethod::Delete => network::Method::Delete,
                wit_network::HttpMethod::Options => network::Method::Options,
            },
            url: request.url,
            headers: request.headers,
            body: request.body,
            timeout_ms: request.timeout_ms,
        };
        Ok(self.network.fetch(request).map_err(Into::into))
    }

    fn cancel_fetch(&mut self, request: u64) -> wasmtime::Result<()> {
        self.network.cancel(request);
        Ok(())
    }
}

impl wit_process::Host for UiState {
    fn spawn(
        &mut self,
        request: wit_process::SpawnRequest,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(
            request.program.len()
                + request.args.iter().map(String::len).sum::<usize>()
                + request
                    .env
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>(),
        )?;
        if let Err(error) = self.permit_t2(process::CAPABILITY_PROCESS) {
            return Ok(Err(error.into()));
        }
        let request = process::SpawnRequest {
            program: request.program,
            args: request.args,
            cwd: request.cwd.map(|cwd| process::FileRef {
                worktree: cwd.worktree,
                path: cwd.path,
            }),
            env: request.env,
            stdin: request.stdin,
        };
        Ok(self.process.spawn(request).map_err(Into::into))
    }

    fn write_stdin(
        &mut self,
        process: u64,
        data: Vec<u8>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        if let Err(error) = self.permit_t2(process::CAPABILITY_PROCESS) {
            return Ok(Err(error.into()));
        }
        Ok(self.process.write_stdin(process, data).map_err(Into::into))
    }

    fn close_stdin(&mut self, process: u64) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.process.close_stdin(process).map_err(Into::into))
    }

    fn kill(&mut self, process: u64) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.process.kill(process).map_err(Into::into))
    }
}

fn wit_permission_state(state: PermissionState) -> wit_permissions::PermissionState {
    match state {
        PermissionState::Granted => wit_permissions::PermissionState::Granted,
        PermissionState::Denied => wit_permissions::PermissionState::Denied,
        PermissionState::Ask => wit_permissions::PermissionState::Ask,
        PermissionState::Unavailable => wit_permissions::PermissionState::Unavailable,
    }
}

/// `permissions` (design.md §7): the state, and the prompt on a gesture.
impl wit_permissions::Host for UiState {
    fn query(
        &mut self,
        capability: wit_permissions::T2Capability,
    ) -> wasmtime::Result<wit_permissions::PermissionState> {
        Ok(wit_permission_state(
            self.query_permission(t2_capability(capability)),
        ))
    }

    fn request(
        &mut self,
        capability: wit_permissions::T2Capability,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        let capability = t2_capability(capability);
        if !self.api.declares(capability) {
            return Ok(Err(model::HostError {
                code: model::HostErrorCode::NotPermitted,
                message: format!("{capability:?} is not in the manifest"),
            }
            .into()));
        }
        if !self.api.take_activation() {
            return Ok(Err(model::HostError {
                code: model::HostErrorCode::NotPermitted,
                message: "permissions.request needs a click or Enter: during handle-event for \
                          click, row-click, input-submitted, canvas-click or notification-action, \
                          or shortly after it, once"
                    .into(),
            }
            .into()));
        }
        // Only `ask` has a prompt to show; any other state stays as it is.
        if self.query_permission(capability) == PermissionState::Ask
            && !self.permission_requests.contains(&capability)
        {
            self.permission_requests.push(capability);
        }
        Ok(Ok(()))
    }
}

fn file_ref(file: wit::FileRef) -> process::FileRef {
    process::FileRef {
        worktree: file.worktree,
        path: file.path,
    }
}

fn wit_file_ref(file: process::FileRef) -> wit::FileRef {
    wit::FileRef {
        worktree: file.worktree,
        path: file.path,
    }
}

fn position(position: wit::Position) -> edit::Position {
    edit::Position {
        line: position.line,
        column: position.column,
    }
}

impl wit_edit::Host for UiState {
    fn apply_edit(
        &mut self,
        request: wit_edit::WorkspaceEdit,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        self.count_lifted_text(
            request.label.len()
                + request
                    .files
                    .iter()
                    .map(|file| {
                        file.file.path.len()
                            + file
                                .edits
                                .iter()
                                .map(|edit| edit.new_text.len())
                                .sum::<usize>()
                    })
                    .sum::<usize>(),
        )?;
        if let Err(error) = self.permit_t2(edit::CAPABILITY_EDIT) {
            return Ok(Err(error.into()));
        }
        let request = edit::WorkspaceEdit {
            label: request.label,
            files: request
                .files
                .into_iter()
                .map(|file| edit::FileEdit {
                    // 0.3 edits files only: document targets and versions come with the
                    // document context of 0.4.
                    target: edit::EditTarget::File(file_ref(file.file)),
                    expected_version: None,
                    create: file.create,
                    edits: file
                        .edits
                        .into_iter()
                        .map(|text_edit| edit::TextEdit {
                            range: edit::TextRange {
                                start: position(text_edit.range.start),
                                end: position(text_edit.range.end),
                            },
                            new_text: text_edit.new_text,
                        })
                        .collect(),
                })
                .collect(),
        };
        Ok(self.edit.apply(request).map_err(Into::into))
    }
}

impl wit_observe::Host for UiState {
    fn start(&mut self) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        if let Err(error) = self.permit_t2(observe::CAPABILITY_OBSERVE) {
            return Ok(Err(error.into()));
        }
        Ok(self.observe.start().map_err(Into::into))
    }

    fn stop(&mut self) -> wasmtime::Result<()> {
        self.observe.stop();
        Ok(())
    }
}

fn resolution(resolution: observe::Resolution) -> wit_observe::Resolution {
    match resolution {
        observe::Resolution::Unchanged => wit_observe::Resolution::Unchanged,
        observe::Resolution::Pending(count) => wit_observe::Resolution::Pending(count),
        observe::Resolution::AcceptedAll => wit_observe::Resolution::AcceptedAll,
        observe::Resolution::RejectedAll => wit_observe::Resolution::RejectedAll,
        observe::Resolution::Partial(accepted, rejected) => {
            wit_observe::Resolution::Partial((accepted, rejected))
        }
    }
}

fn rule(rule: observe::RuleKind) -> wit_observe::RuleKind {
    match rule {
        observe::RuleKind::AutoAccept => wit_observe::RuleKind::AutoAccept,
        observe::RuleKind::AlwaysAsk => wit_observe::RuleKind::AlwaysAsk,
        observe::RuleKind::Deny => wit_observe::RuleKind::Deny,
    }
}

fn review_info(info: observe::ReviewInfo) -> wit_observe::ReviewInfo {
    wit_observe::ReviewInfo {
        review: info.review,
        session: info.session,
        turn: info.turn,
        file: info.file.map(wit_file_ref),
        source: match info.source {
            observe::ReviewSource::Permission => wit_observe::ReviewSource::Permission,
            observe::ReviewSource::Write => wit_observe::ReviewSource::Write,
        },
        rule: info.rule.map(rule),
        resolution: resolution(info.resolution),
        submitted: info.submitted,
        hunks: info
            .hunks
            .into_iter()
            .map(|hunk| wit_observe::HunkInfo {
                old_start: hunk.old_start,
                old_count: hunk.old_count,
                new_start: hunk.new_start,
                new_count: hunk.new_count,
                old_text: hunk.old_text,
                new_text: hunk.new_text,
                decision: match hunk.decision {
                    observe::HunkDecision::Pending => wit_observe::HunkDecision::Pending,
                    observe::HunkDecision::Accepted => wit_observe::HunkDecision::Accepted,
                    observe::HunkDecision::Rejected => wit_observe::HunkDecision::Rejected,
                },
                comment: hunk.comment,
            })
            .collect(),
        truncated: info.truncated,
    }
}

fn agent_event(event: observe::AgentEvent) -> wit_observe::AgentEvent {
    match event {
        observe::AgentEvent::Snapshot {
            reviews,
            older_dropped,
        } => wit_observe::AgentEvent::Snapshot(wit_observe::ReviewSnapshot {
            reviews: reviews.into_iter().map(review_info).collect(),
            older_dropped,
        }),
        observe::AgentEvent::Opened(info) => wit_observe::AgentEvent::Opened(review_info(info)),
        observe::AgentEvent::Updated(info) => wit_observe::AgentEvent::Updated(review_info(info)),
        observe::AgentEvent::Resolved(info) => wit_observe::AgentEvent::Resolved(review_info(info)),
        observe::AgentEvent::Closed(review) => wit_observe::AgentEvent::Closed(review),
        observe::AgentEvent::TurnEnded(turn, files) => wit_observe::AgentEvent::TurnEnded((
            turn,
            files
                .into_iter()
                .map(|file| wit_observe::TurnFile {
                    file: file.file.map(wit_file_ref),
                    resolution: resolution(file.resolution),
                    rule: file.rule.map(rule),
                })
                .collect(),
        )),
        observe::AgentEvent::UnreviewedChanges(turn, files) => {
            wit_observe::AgentEvent::UnreviewedChanges((
                turn,
                files.into_iter().map(wit_file_ref).collect(),
            ))
        }
        observe::AgentEvent::ReviewMode(on) => wit_observe::AgentEvent::ReviewMode(on),
    }
}

/// An outcome for the `handle-io` export; `None` for one this world has no name for.
pub(crate) fn io_event(event: network::IoEvent) -> Option<wit_io::IoEvent> {
    Some(match event {
        network::IoEvent::ProcessOutput(output) => {
            wit_io::IoEvent::ProcessOutput(wit_process::ProcessOutput {
                process: output.process,
                stream: match output.stream {
                    process::OutputStream::Stdout => wit_process::OutputStream::Stdout,
                    process::OutputStream::Stderr => wit_process::OutputStream::Stderr,
                },
                data: output.data,
            })
        }
        network::IoEvent::ProcessExited(exit) => {
            wit_io::IoEvent::ProcessExited(wit_process::ProcessExit {
                process: exit.process,
                code: exit.code,
                killed: exit.killed,
            })
        }
        network::IoEvent::SpawnFailed(id, error) => wit_io::IoEvent::SpawnFailed((
            id,
            match error {
                process::SpawnError::Denied => wit_process::SpawnError::Denied,
                process::SpawnError::NotFound(why) => wit_process::SpawnError::NotFound(why),
                process::SpawnError::LimitExceeded => wit_process::SpawnError::LimitExceeded,
                process::SpawnError::Io(why) => wit_process::SpawnError::Io(why),
                process::SpawnError::Other(why) => wit_process::SpawnError::Other(why),
            },
        )),
        network::IoEvent::PermissionChanged(capability, state) => {
            wit_io::IoEvent::PermissionChanged((
                t2_from_name(&capability)?,
                wit_permission_state(state),
            ))
        }
        network::IoEvent::Terminal(event) => return terminal_io_event(event),
        network::IoEvent::Listener(event) => return listener_io_event(event),
        network::IoEvent::Context(context) => return context_io_event(context),
        network::IoEvent::DiagnosticsChanged(files) => return diagnostics_io_event(files),
        network::IoEvent::ReviewDecided(id, result) => return review_io_event(id, result),
        network::IoEvent::RelayRequest(request) => return relay_io_event(request),
        network::IoEvent::ApprovalAnswered(id, answer) => {
            return approval_io_event(id, answer);
        }
        network::IoEvent::EditDone(id, result) => wit_io::IoEvent::EditDone((
            id,
            result.map_err(|error| match error {
                edit::EditError::Denied => wit_edit::EditError::Denied,
                edit::EditError::Conflict(why) => wit_edit::EditError::Conflict(why),
                edit::EditError::NotAllowed(why) => wit_edit::EditError::NotAllowed(why),
                edit::EditError::NotFound(why) => wit_edit::EditError::NotFound(why),
                edit::EditError::Invalid(why) => wit_edit::EditError::Invalid(why),
                edit::EditError::Cancelled => wit_edit::EditError::Cancelled,
                edit::EditError::UntrustedWorktree => wit_edit::EditError::UntrustedWorktree,
                edit::EditError::Other(why) => wit_edit::EditError::Other(why),
            }),
        )),
        network::IoEvent::Agent(event) => wit_io::IoEvent::Agent(agent_event(event)),
        network::IoEvent::FetchDone(id, result) => wit_io::IoEvent::FetchDone((
            id,
            result
                .map(|response| wit_network::HttpResponse {
                    status: response.status,
                    headers: response.headers,
                    body: response.body,
                    url: response.url,
                })
                .map_err(|error| match error {
                    network::FetchError::Denied => wit_network::FetchError::Denied,
                    network::FetchError::HostNotAllowed(host) => {
                        wit_network::FetchError::HostNotAllowed(host)
                    }
                    network::FetchError::RedirectNotAllowed(host) => {
                        wit_network::FetchError::RedirectNotAllowed(host)
                    }
                    network::FetchError::Dns(error) => wit_network::FetchError::Dns(error),
                    network::FetchError::Connect(error) => wit_network::FetchError::Connect(error),
                    network::FetchError::Tls(error) => wit_network::FetchError::Tls(error),
                    network::FetchError::Timeout => wit_network::FetchError::Timeout,
                    network::FetchError::TooLarge => wit_network::FetchError::TooLarge,
                    network::FetchError::Cancelled => wit_network::FetchError::Cancelled,
                    network::FetchError::Other(error) => wit_network::FetchError::Other(error),
                }),
        )),
    })
}

fn paint(paint: wit::Paint) -> model_canvas::Paint {
    model_canvas::Paint {
        color: match paint.color {
            wit::CanvasColor::Token(token) => model_canvas::CanvasColor::Token(token.into()),
            wit::CanvasColor::Data(data) => model_canvas::CanvasColor::Data(match data {
                wit::DataColor::Series1 => model_canvas::DataColor::Series1,
                wit::DataColor::Series2 => model_canvas::DataColor::Series2,
                wit::DataColor::Series3 => model_canvas::DataColor::Series3,
                wit::DataColor::Series4 => model_canvas::DataColor::Series4,
                wit::DataColor::Series5 => model_canvas::DataColor::Series5,
                wit::DataColor::Series6 => model_canvas::DataColor::Series6,
                wit::DataColor::Series7 => model_canvas::DataColor::Series7,
                wit::DataColor::Series8 => model_canvas::DataColor::Series8,
            }),
        },
        opacity: paint.opacity,
    }
}

fn stroke(stroke: wit::Stroke) -> model_canvas::Stroke {
    model_canvas::Stroke {
        paint: paint(stroke.paint),
        width: stroke.width.into(),
    }
}

fn point(point: wit::Point) -> model_canvas::Point {
    model_canvas::Point {
        x: point.x.into(),
        y: point.y.into(),
    }
}

fn rect(rect: wit::Rect) -> model_canvas::Rect {
    model_canvas::Rect {
        x: rect.x.into(),
        y: rect.y.into(),
        width: rect.width.into(),
        height: rect.height.into(),
    }
}

/// A 0.3 canvas as the model holds it; [`model_canvas::check_canvas_props`] runs when the update
/// or the reply is applied ([`crate::effects::ApiState::check_images`]).
fn canvas(props: wit::CanvasProps) -> model_canvas::CanvasProps {
    model_canvas::CanvasProps {
        width: props.width,
        height: props.height,
        description: props.description,
        ops: props
            .ops
            .into_iter()
            .map(|op| match op {
                wit::DrawOp::Line(op) => model_canvas::DrawOp::Line {
                    from: point(op.from),
                    to: point(op.to),
                    stroke: stroke(op.stroke),
                },
                wit::DrawOp::Rect(op) => model_canvas::DrawOp::Rect {
                    bounds: rect(op.bounds),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::RoundedRect(op) => model_canvas::DrawOp::RoundedRect {
                    bounds: rect(op.bounds),
                    radius: op.radius.into(),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::Circle(op) => model_canvas::DrawOp::Circle {
                    center: point(op.center),
                    radius: op.radius.into(),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::Path(op) => model_canvas::DrawOp::Path {
                    segments: op
                        .segments
                        .into_iter()
                        .map(|segment| match segment {
                            wit::PathSegment::MoveTo(at) => {
                                model_canvas::PathSegment::MoveTo(point(at))
                            }
                            wit::PathSegment::LineTo(to) => {
                                model_canvas::PathSegment::LineTo(point(to))
                            }
                            wit::PathSegment::QuadTo((control, to)) => {
                                model_canvas::PathSegment::QuadTo(point(control), point(to))
                            }
                            wit::PathSegment::CubicTo((first, second, to)) => {
                                model_canvas::PathSegment::CubicTo(
                                    point(first),
                                    point(second),
                                    point(to),
                                )
                            }
                            wit::PathSegment::Close => model_canvas::PathSegment::Close,
                        })
                        .collect(),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::Text(op) => model_canvas::DrawOp::Text {
                    at: point(op.at),
                    text: op.text,
                    paint: paint(op.paint),
                    size: match op.size {
                        wit::CanvasTextSize::Small => model_canvas::CanvasTextSize::Small,
                        wit::CanvasTextSize::Medium => model_canvas::CanvasTextSize::Medium,
                        wit::CanvasTextSize::Large => model_canvas::CanvasTextSize::Large,
                    },
                    anchor: match op.anchor {
                        wit::TextAnchor::Start => model_canvas::TextAnchor::Start,
                        wit::TextAnchor::Middle => model_canvas::TextAnchor::Middle,
                        wit::TextAnchor::End => model_canvas::TextAnchor::End,
                    },
                    monospace: op.monospace,
                },
            })
            .collect(),
        clickable: props.clickable,
        fit: match props.fit {
            wit::CanvasFit::Clip => model_canvas::CanvasFit::Clip,
            wit::CanvasFit::ScaleDown => model_canvas::CanvasFit::ScaleDown,
        },
    }
}
