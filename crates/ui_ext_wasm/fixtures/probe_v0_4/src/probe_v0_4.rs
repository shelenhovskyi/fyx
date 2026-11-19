//! `fyx:ui@0.4.0` guest for the host tests of the staged 0.4 world (plan 5.5): the `terminals`
//! import and the `terminal` node. Surface `probe` (a panel) is driven by `input-changed`, whose
//! text is a command; the reply is one text node (id 1) reporting each result, joined with ` | `:
//!
//! - `pseudo <title>`: `open-pseudo`, its handle or error code;
//! - `out <handle> <text>`: `write-output`; `out-big <handle>`: 64 KiB + 1 bytes;
//! - `create <program> [<arg>...]`: `create`, its handle or error code;
//! - `input <handle> <text>`: `send-input`;
//! - `status <handle> <label>`: `set-status`;
//! - `list`: `list`, as `<handle>:<origin>:<title>` each, or the error code;
//! - `paste <handle> <text>`: `paste` (`input-changed` is no gesture);
//! - `read <handle>`: `request-read`; `close <handle>`: `close`, reported as `done`;
//! - `perm <capability>`: `permissions.query` of a 0.4 T2 capability;
//! - `ask <tool|-> <title> <once|always|deny>...`: `approvals.ask` (`-` for no tool), its id
//!   or error code;
//! - `listen`: a call into `listeners`, which traps in this build;
//! - `node`: answers a `terminal` node instead of the report.
//!
//! `handle-io` pushes each terminal event to `probe` as one text node (id 1): `input <h> <text>`,
//! `resized <h> <cols> <rows>`, `output <h> <text> <dropped>`, `exited <h> <code|none>`,
//! `created <h>`, `closed <h>`, `status-changed <h>`, `permission <capability> <state>`,
//! `approval <id> <once|always|deny|none>`.

wit_bindgen::generate!({
    path: "../../../ui_ext/wit/since_v0.4.0",
    world: "ui-extension",
});

use fyx::ui::approvals::{self, ApprovalOption};
use fyx::ui::host::{self, HostError, HostErrorCode};
use fyx::ui::io::IoEvent as Io;
use fyx::ui::permissions::{self, T2Capability};
use fyx::ui::terminals::{self, ReadScope, StatusState, TerminalOrigin, TerminalStatus};
use fyx::ui::types::*;

#[cfg(target_arch = "wasm32")]
#[unsafe(link_section = "fyx:ui-api-version")]
#[used]
static FYX_UI_API_VERSION: [u8; 6] = [0, 0, 0, 4, 0, 0];

const SURFACE_ID: &str = "probe";

struct Probe;

fn text(id: NodeId, content: &str) -> Node {
    Node {
        id,
        parent: None,
        props: Props::Text(TextProps {
            content: content.into(),
            color: None,
            bold: false,
            monospace: false,
        }),
    }
}

fn code(error: &HostError) -> &'static str {
    match error.code {
        HostErrorCode::UnknownSurface => "unknown-surface",
        HostErrorCode::LimitExceeded => "limit-exceeded",
        HostErrorCode::InvalidArgument => "invalid-argument",
        HostErrorCode::NotPermitted => "not-permitted",
        HostErrorCode::Unavailable => "unavailable",
        HostErrorCode::Denied => "denied",
        HostErrorCode::UntrustedWorktree => "untrusted-worktree",
        HostErrorCode::NoWorktree => "no-worktree",
    }
}

fn done(result: Result<(), HostError>) -> String {
    match result {
        Ok(()) => "done".into(),
        Err(error) => code(&error).into(),
    }
}

fn id(result: Result<u64, HostError>) -> String {
    match result {
        Ok(id) => id.to_string(),
        Err(error) => code(&error).into(),
    }
}

fn handle(word: &str) -> Result<u64, String> {
    word.parse().map_err(|error| format!("{error}"))
}

fn t2(name: &str) -> Option<T2Capability> {
    Some(match name {
        "terminal-create" => T2Capability::TerminalCreate,
        "terminal-env" => T2Capability::TerminalEnv,
        "terminal-observe" => T2Capability::TerminalObserve,
        "network-listen" => T2Capability::NetworkListen,
        "relay-hooks" => T2Capability::RelayHooks,
        "process-spawn" => T2Capability::ProcessSpawn,
        _ => return None,
    })
}

fn t2_name(capability: T2Capability) -> &'static str {
    match capability {
        T2Capability::NetworkFetch => "network-fetch",
        T2Capability::ProcessSpawn => "process-spawn",
        T2Capability::WorkspaceEdit => "workspace-edit",
        T2Capability::AgentObserve => "agent-observe",
        T2Capability::NetworkListen => "network-listen",
        T2Capability::TerminalCreate => "terminal-create",
        T2Capability::TerminalEnv => "terminal-env",
        T2Capability::TerminalObserve => "terminal-observe",
        T2Capability::FsDiscoveryFile => "fs-discovery-file",
        T2Capability::RelayHooks => "relay-hooks",
    }
}

fn state(state: permissions::PermissionState) -> &'static str {
    match state {
        permissions::PermissionState::Granted => "granted",
        permissions::PermissionState::Denied => "denied",
        permissions::PermissionState::Ask => "ask",
        permissions::PermissionState::Unavailable => "unavailable",
    }
}

fn run(words: &[&str]) -> Result<String, String> {
    Ok(match words {
        ["pseudo", title] => id(terminals::open_pseudo(&terminals::PseudoRequest {
            title: (*title).into(),
            cols: Some(80),
            rows: Some(24),
            in_panel: false,
        })),
        ["out", h, text] => done(terminals::write_output(handle(h)?, text.as_bytes())),
        ["out-big", h] => done(terminals::write_output(
            handle(h)?,
            &vec![b'x'; 64 * 1024 + 1],
        )),
        ["create", program, args @ ..] => id(terminals::create(&terminals::CreateRequest {
            program: (*program).into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            cwd: None,
            title: None,
        })),
        ["input", h, text] => done(terminals::send_input(handle(h)?, text.as_bytes())),
        ["status", h, label] => done(terminals::set_status(
            handle(h)?,
            Some(&TerminalStatus {
                state: StatusState::Working,
                label: (*label).into(),
                detail: None,
                progress: Some(50),
                attention: false,
            }),
        )),
        ["list"] => match terminals::list() {
            Ok(infos) => infos
                .iter()
                .map(|info| {
                    let origin = match info.origin {
                        TerminalOrigin::Pseudo => "pseudo",
                        TerminalOrigin::Created => "created",
                        TerminalOrigin::Profile => "profile",
                        TerminalOrigin::User => "user",
                    };
                    format!("{}:{origin}:{}", info.handle, info.title)
                })
                .collect::<Vec<_>>()
                .join(","),
            Err(error) => code(&error).into(),
        },
        ["paste", h, text] => done(terminals::paste(handle(h)?, text)),
        ["read", h] => id(terminals::request_read(
            handle(h)?,
            ReadScope::LastLines,
            10,
        )),
        ["close", h] => {
            terminals::close(handle(h)?);
            "done".into()
        }
        ["perm", name] => {
            let capability = t2(name).ok_or("unknown capability")?;
            state(permissions::query(capability)).into()
        }
        ["ask", tool, title, options @ ..] => {
            let options = options
                .iter()
                .map(|option| match *option {
                    "once" => Ok(ApprovalOption::AllowOnce),
                    "always" => Ok(ApprovalOption::AlwaysAllow),
                    "deny" => Ok(ApprovalOption::Deny),
                    other => Err(format!("unknown option {other}")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let tool = if *tool == "-" { "" } else { tool };
            id(approvals::ask(tool, title, "detail", &options))
        }
        ["listen"] => done(fyx::ui::listeners::reply(1, 200, "")),
        _ => return Err(format!("unknown command {words:?}")),
    })
}

impl Guest for Probe {
    fn surfaces() -> Vec<Surface> {
        vec![Surface {
            id: SURFACE_ID.into(),
            title: "Probe".into(),
            kind: SurfaceKind::Panel(PanelDescriptor {
                position: DockPosition::Right,
                icon: Icon::Terminal,
                default_size: 300,
            }),
        }]
    }

    fn mount(_surface_id: String) -> Result<Vec<Node>, String> {
        Ok(vec![text(1, "probe")])
    }

    fn handle_event(_surface_id: String, event: UiEvent) -> Result<Update, String> {
        let UiEvent::InputChanged((_, command)) = event else {
            return Ok(Update::Unchanged);
        };
        let words: Vec<&str> = command.split_whitespace().collect();
        if words == ["node"] {
            return Ok(Update::Replace(vec![Node {
                id: 1,
                parent: None,
                props: Props::Terminal(TerminalProps { handle: 1 }),
            }]));
        }
        let report = run(&words)?;
        Ok(Update::Replace(vec![text(1, &report)]))
    }

    fn list_rows(
        _surface_id: String,
        _list_node: NodeId,
        _start: u32,
        _count: u32,
    ) -> Vec<ListRow> {
        Vec::new()
    }

    fn tick(_surface_id: String) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }

    fn handle_io(event: Io) -> Result<(), String> {
        let lossy = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let report = match event {
            Io::TerminalInput((h, bytes)) => format!("input {h} {}", lossy(&bytes)),
            Io::TerminalResized((h, cols, rows)) => format!("resized {h} {cols} {rows}"),
            Io::TerminalOutput((h, bytes, dropped)) => {
                format!("output {h} {} {dropped}", lossy(&bytes))
            }
            Io::TerminalExited((h, code)) => format!(
                "exited {h} {}",
                code.map_or("none".to_string(), |code| code.to_string())
            ),
            Io::TerminalCreated(info) => format!("created {}", info.handle),
            Io::TerminalClosed(h) => format!("closed {h}"),
            Io::TerminalStatusChanged(info) => format!("status-changed {}", info.handle),
            Io::PermissionChanged((capability, new_state)) => {
                format!("permission {} {}", t2_name(capability), state(new_state))
            }
            Io::ApprovalAnswered((id, answer)) => format!(
                "approval {id} {}",
                match answer {
                    Some(ApprovalOption::AllowOnce) => "once",
                    Some(ApprovalOption::AlwaysAllow) => "always",
                    Some(ApprovalOption::Deny) => "deny",
                    None => "none",
                }
            ),
            _ => "other".into(),
        };
        host::push_update(SURFACE_ID, &Update::Replace(vec![text(1, &report)]))
            .map_err(|error| code(&error).to_string())
    }
}

export!(Probe);
