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
//! - `read <handle> [<lines>]`: `request-read` of the last lines (10 by default);
//!   `read-command <handle>`: of the last command; `write <handle> <text>`: `request-write`,
//!   `write-enter <handle> <text>` with `submit`; `close <handle>`: `close`, reported as `done`;
//! - `arm <command>`: keeps the command, which the next `click` on any node runs (a gesture);
//! - `perm <capability>`: `permissions.query` of a 0.4 T2 capability;
//! - `ask <tool|-> <title> <once|always|deny>...`: `approvals.ask` (`-` for no tool), its id
//!   or error code;
//! - `tools <listener> <name>...`: `listeners.register-tools`, each with an object schema;
//!   `respond <call-id> <json>`: `respond` with a result; `respond-err <call-id> <code> <message>`
//!   with an MCP error; `notify <listener> <method>`: `notify` everyone with `{}`;
//!   `send <listener> <connection> <text>`: `send`; `reply <id> <status> <body>`: `reply`;
//! - `watch <on|off>`: `context.watch`; `open <worktree|-> <path> <0|1>`: `workspace.open`
//!   with `focus`; `to-ref <absolute>`: `context.to-file-ref` as `<worktree|->:<path>`;
//!   `abs <worktree|-> <path>`: `context.absolute-path`; `diag <worktree|-> <path>` and
//!   `diag-all`: `context.diagnostics` as `<path>:<e|w|i|h><line>.<column>,...;...` (or `empty`),
//!   then ` truncated` if so, then `|<source>|<code>|<message bytes>` of the first entry;
//!   `watch-diag <on|off>`: `context.watch-diagnostics`;
//! - `relay-respond <id> <exit> <stdout|-> <stderr|->`: `relay.respond`; `relay-long <id>`
//!   answers with 1 MiB and a byte of stdout;
//! - `setting <key>`: `settings.get` as its value, or the error code; `settings`: `settings.all`
//!   as `<key>=<value>[*]`, comma-separated (`*` when `is-default`); `open-settings [<key>]`:
//!   `settings.open`;
//! - `publish <worktree|-> <path> <count>`: `context.publish-diagnostics` of `count` warnings
//!   (line `i`, columns 0..4, source `rust-analyzer`); `clear-diag [<worktree|-> <path>]`:
//!   `clear-diagnostics`; `decorate <worktree|-> <path> <badge> [<color>]`: `decorate-files`
//!   (colour `warning` unless given: `surface` for a refused one); `clear-deco
//!   [<worktree|-> <path>]`: `clear-file-decorations`;
//! - `node`: answers a `terminal` node instead of the report.
//!
//! `handle-io` pushes each terminal event to `probe` as one text node (id 1): `input <h> <text>`,
//! `resized <h> <cols> <rows>`, `output <h> <text> <dropped>`, `exited <h> <code|none>`,
//! `created <h>`, `closed <h>`, `status-changed <h>`, `permission <capability> <state>`,
//! `approval <id> <once|always|deny|none>`, `read-done <id> <text|error: why>`,
//! `write-done <id> <done|error: why>`, `mcp-call <id> <listener> <connection> <tool> <arguments>`,
//! `mcp-cancelled <id>`, `mcp-initialized <listener> <connection> <client-name> <client-version>`,
//! `message <listener> <connection> <text>`,
//! `request <id> <listener> <path> <body>`,
//! `context <active|-> <open,...> <selection bytes|none|-> <trusted|->`,
//! `diagnostics-changed <path,...|*>` (`*` for the empty list),
//! `relay <id> <channel> <terminal|-> <argv,...|-> <stdin>`, `settings-changed <key,...>`.

wit_bindgen::generate!({
    path: "../../../ui_ext/wit/since_v0.4.0",
    world: "ui-extension",
});

use fyx::ui::approvals::{self, ApprovalOption};
use fyx::ui::host::{self, HostError, HostErrorCode};
use fyx::ui::io::IoEvent as Io;
use fyx::ui::listeners;
use fyx::ui::permissions::{self, T2Capability};
use fyx::ui::review;
use fyx::ui::settings::{self, SettingValue};
use fyx::ui::terminals::{self, ReadScope, StatusState, TerminalOrigin, TerminalStatus};
use fyx::ui::types::*;

#[cfg(target_arch = "wasm32")]
#[unsafe(link_section = "fyx:ui-api-version")]
#[used]
static FYX_UI_API_VERSION: [u8; 6] = [0, 0, 0, 4, 0, 0];

/// The command `arm` keeps for the next click.
static ARMED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

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

fn setting_value(value: &SettingValue) -> String {
    match value {
        SettingValue::Boolean(value) => value.to_string(),
        SettingValue::Integer(value) => value.to_string(),
        SettingValue::Number(value) => value.to_string(),
        SettingValue::Text(value) | SettingValue::Choice(value) => value.clone(),
        SettingValue::TextList(items) => items.join("|"),
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

fn file(worktree: &str, path: &str) -> Result<FileRef, String> {
    Ok(FileRef {
        worktree: match worktree {
            "-" => None,
            id => Some(id.parse().map_err(|_| "worktree")?),
        },
        path: path.into(),
    })
}

fn diagnostics(result: Result<fyx::ui::context::DiagnosticsSnapshot, HostError>) -> String {
    use fyx::ui::context::Severity;
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(error) => return code(&error).into(),
    };
    let files: Vec<String> = snapshot
        .files
        .iter()
        .map(|file| {
            let entries: Vec<String> = file
                .diagnostics
                .iter()
                .map(|entry| {
                    let severity = match entry.severity {
                        Severity::Error => "e",
                        Severity::Warning => "w",
                        Severity::Information => "i",
                        Severity::Hint => "h",
                    };
                    format!(
                        "{severity}{}.{}",
                        entry.range.start.line, entry.range.start.column
                    )
                })
                .collect();
            format!("{}:{}", file.path.path, entries.join(","))
        })
        .collect();
    let mut out = if files.is_empty() {
        "empty".to_string()
    } else {
        files.join(";")
    };
    if snapshot.truncated {
        out.push_str(" truncated");
    }
    if let Some(first) = snapshot
        .files
        .first()
        .and_then(|file| file.diagnostics.first())
    {
        out.push_str(&format!(
            "|{}|{}|{}",
            first.source.as_deref().unwrap_or("-"),
            first.code.as_deref().unwrap_or("-"),
            first.message.len()
        ));
    }
    out
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
        ["read", h, lines] => id(terminals::request_read(
            handle(h)?,
            ReadScope::LastLines,
            lines.parse().map_err(|_| "lines")?,
        )),
        ["read-command", h] => id(terminals::request_read(
            handle(h)?,
            ReadScope::LastCommand,
            0,
        )),
        ["write", h, text @ ..] => id(terminals::request_write(handle(h)?, &text.join(" "), false)),
        ["write-enter", h, text @ ..] => {
            id(terminals::request_write(handle(h)?, &text.join(" "), true))
        }
        ["arm", command @ ..] => {
            *ARMED.lock().unwrap() = command.join(" ");
            "armed".into()
        }
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
        ["tools", listener, names @ ..] => {
            let tools: Vec<listeners::ToolDef> = names
                .iter()
                .map(|name| listeners::ToolDef {
                    name: (*name).into(),
                    description: format!("the {name} tool"),
                    input_schema: r#"{"type":"object"}"#.into(),
                })
                .collect();
            done(listeners::register_tools(listener, &tools))
        }
        ["respond", call, json] => done(listeners::respond(
            call.parse().map_err(|_| "call id")?,
            Ok(json),
        )),
        ["respond-err", call, error_code, message @ ..] => done(listeners::respond(
            call.parse().map_err(|_| "call id")?,
            Err(&listeners::McpError {
                code: error_code.parse().map_err(|_| "code")?,
                message: message.join(" "),
            }),
        )),
        ["notify", listener, method] => done(listeners::notify(listener, None, method, "{}")),
        ["send", listener, connection, text @ ..] => done(listeners::send(
            listener,
            connection.parse().map_err(|_| "connection")?,
            &text.join(" "),
        )),
        ["reply", id, status, body @ ..] => done(listeners::reply(
            id.parse().map_err(|_| "id")?,
            status.parse().map_err(|_| "status")?,
            &body.join(" "),
        )),
        ["setting", key] => match settings::get(key) {
            Ok(value) => setting_value(&value),
            Err(error) => code(&error).into(),
        },
        ["settings"] => settings::all()
            .iter()
            .map(|setting| {
                format!(
                    "{}={}{}",
                    setting.key,
                    setting_value(&setting.value),
                    if setting.is_default { "*" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(","),
        ["open-settings"] => done(settings::open(None)),
        ["open-settings", key] => done(settings::open(Some(key))),
        ["watch", on] => done(fyx::ui::context::watch(*on == "on")),
        ["open", worktree, path, focus] => done(fyx::ui::context::open(
            &file(worktree, path)?,
            None,
            *focus == "1",
        )),
        ["to-ref", absolute] => match fyx::ui::context::to_file_ref(absolute) {
            Ok(file) => format!(
                "{}:{}",
                file.worktree.map_or("-".to_string(), |id| id.to_string()),
                file.path
            ),
            Err(error) => code(&error).into(),
        },
        ["abs", worktree, path] => match fyx::ui::context::absolute_path(&file(worktree, path)?) {
            Ok(path) => path,
            Err(error) => code(&error).into(),
        },
        ["diag", worktree, path] => {
            diagnostics(fyx::ui::context::diagnostics(Some(&file(worktree, path)?)))
        }
        ["diag-all"] => diagnostics(fyx::ui::context::diagnostics(None)),
        ["watch-diag", on] => done(fyx::ui::context::watch_diagnostics(*on == "on")),
        ["publish", worktree, path, count] => {
            let count: u32 = count.parse().map_err(|_| "count")?;
            let entries: Vec<fyx::ui::context::Diagnostic> = (0..count)
                .map(|line| fyx::ui::context::Diagnostic {
                    range: TextRange {
                        start: Position { line, column: 0 },
                        end: Position { line, column: 4 },
                    },
                    severity: fyx::ui::context::Severity::Warning,
                    message: format!("m{line}"),
                    source: Some("rust-analyzer".into()),
                    code: None,
                })
                .collect();
            done(fyx::ui::context::publish_diagnostics(
                &file(worktree, path)?,
                &entries,
            ))
        }
        ["clear-diag"] => done(fyx::ui::context::clear_diagnostics(None)),
        ["clear-diag", worktree, path] => done(fyx::ui::context::clear_diagnostics(Some(
            &file(worktree, path)?,
        ))),
        ["decorate", worktree, path, badge, color @ ..] => {
            let color = match color {
                [] => ColorToken::Warning,
                ["surface"] => ColorToken::Surface,
                _ => return Err("color".into()),
            };
            done(fyx::ui::context::decorate_files(&[
                fyx::ui::context::FileDecoration {
                    path: file(worktree, path)?,
                    badge: Some((*badge).into()),
                    color: Some(color),
                    tooltip: None,
                },
            ]))
        }
        ["clear-deco"] => done(fyx::ui::context::clear_file_decorations(None)),
        ["clear-deco", worktree, path] => done(fyx::ui::context::clear_file_decorations(Some(
            &[file(worktree, path)?],
        ))),
        ["propose", worktree, path, kind, apply, session, text @ ..] => {
            let text = text.join(" ");
            id(review::propose(&review::Proposal {
                path: file(worktree, path)?,
                change: match *kind {
                    "new" => review::Proposed::NewContent(text),
                    "diff" => review::Proposed::UnifiedDiff(text),
                    _ => return Err("kind".into()),
                },
                title: "probe".into(),
                session: (*session).into(),
                apply: *apply == "1",
            }))
        }
        // A proposal of `bytes` bytes, or one whose title is `bytes` long.
        ["propose-big", path, bytes] | ["propose-title", path, bytes] => {
            let bytes: usize = bytes.parse().map_err(|_| "bytes")?;
            let long = words[0] == "propose-title";
            id(review::propose(&review::Proposal {
                path: file("1", path)?,
                change: review::Proposed::NewContent("x".repeat(if long { 1 } else { bytes })),
                title: if long {
                    "t".repeat(bytes)
                } else {
                    "probe".into()
                },
                session: "s".into(),
                apply: false,
            }))
        }
        ["withdraw", id] => match review::withdraw(id.parse().map_err(|_| "id")?) {
            Ok(text) => format!("withdrawn {text}"),
            Err(error) => code(&error).into(),
        },
        ["begin", session] => done(review::begin_turn(session)),
        ["end", session, files @ ..] => {
            let files = files
                .iter()
                .map(|spec| {
                    let (worktree, path) = spec.split_once(':').ok_or("file")?;
                    file(worktree, path)
                })
                .collect::<Result<Vec<_>, String>>()?;
            done(review::end_turn(session, &files))
        }
        ["relay-respond", id, exit, stdout, stderr] => {
            let text = |word: &str| {
                if word == "-" {
                    String::new()
                } else {
                    word.into()
                }
            };
            done(fyx::ui::relay::respond(
                id.parse().map_err(|_| "id")?,
                exit.parse().map_err(|_| "exit")?,
                &text(stdout),
                &text(stderr),
            ))
        }
        ["relay-long", id] => done(fyx::ui::relay::respond(
            id.parse().map_err(|_| "id")?,
            0,
            &"x".repeat((1 << 20) + 1),
            "",
        )),
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
        let command = match event {
            UiEvent::InputChanged((_, command)) => command,
            UiEvent::Click(_) => std::mem::take(&mut *ARMED.lock().unwrap()),
            _ => return Ok(Update::Unchanged),
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
            Io::TerminalReadDone((id, result)) => match result {
                Ok(text) => format!("read-done {id} {text}"),
                Err(why) => format!("read-done {id} error: {why}"),
            },
            Io::TerminalWriteDone((id, result)) => match result {
                Ok(()) => format!("write-done {id} done"),
                Err(why) => format!("write-done {id} error: {why}"),
            },
            Io::McpCall(call) => format!(
                "mcp-call {} {} {} {} {}",
                call.call_id, call.listener, call.connection, call.tool, call.arguments
            ),
            Io::McpCancelled(id) => format!("mcp-cancelled {id}"),
            Io::McpInitialized(session) => format!(
                "mcp-initialized {} {} {} {}",
                session.listener, session.connection, session.client_name, session.client_version
            ),
            Io::DiagnosticsChanged(files) if files.is_empty() => "diagnostics-changed *".into(),
            Io::DiagnosticsChanged(files) => format!(
                "diagnostics-changed {}",
                files
                    .iter()
                    .map(|file| file.path.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Io::ContextChanged(context) => format!(
                "context {} {} {} {}",
                context.active.map_or("-".to_string(), |file| file.path),
                context
                    .open
                    .iter()
                    .map(|file| file.path.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                match context.selection {
                    Some((_, Some(text))) => text.len().to_string(),
                    Some((_, None)) => "none".into(),
                    None => "-".into(),
                },
                context
                    .trusted
                    .map_or("-".to_string(), |trusted| trusted.to_string()),
            ),
            Io::ReviewDecided((id, result)) => match result {
                Ok(outcome) => format!(
                    "decided {id} {} {}",
                    match outcome.decision {
                        review::Decision::Accepted => "accepted",
                        review::Decision::Partial => "partial",
                        review::Decision::Rejected => "rejected",
                    },
                    outcome.final_content
                ),
                Err(why) => format!("decided {id} error: {why}"),
            },
            Io::ListenerMessage((listener, connection, text)) => {
                format!("message {listener} {connection} {text}")
            }
            Io::ListenerRequest((id, listener, path, body)) => {
                format!("request {id} {listener} {path} {body}")
            }
            Io::RelayRequest((id, channel, terminal, argv, stdin)) => format!(
                "relay {id} {channel} {} {} {stdin}",
                terminal.map_or("-".to_string(), |handle| handle.to_string()),
                if argv.is_empty() {
                    "-".to_string()
                } else {
                    argv.join(",")
                },
            ),
            Io::SettingsChanged(keys) => format!("settings-changed {}", keys.join(",")),
            _ => "other".into(),
        };
        host::push_update(SURFACE_ID, &Update::Replace(vec![text(1, &report)]))
            .map_err(|error| code(&error).to_string())
    }
}

export!(Probe);
