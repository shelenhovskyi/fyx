//! Gemini CLI's command hooks, from the settings file the package ships
//! (`gemini-settings/system-defaults.json`, loaded through `GEMINI_CLI_SYSTEM_DEFAULTS_PATH`), as they reach the
//! extension: `io-event::relay-request(id, channel, terminal, argv, stdin)` with Gemini's hook
//! input JSON on stdin.
//!
//! The facts below were read from Gemini CLI's `packages/core/src/hooks` at `17720a507b`
//! (2026-11-18, the source the compat test pins):
//! - Event names are `HookEventName`; each input has `session_id`, `cwd` and
//!   `hook_event_name`. `BeforeAgent` comes once per prompt, `AfterAgent` when the agent is done
//!   with it (`stop_hook_active` when an `AfterAgent` hook made it go on).
//! - Hooks from settings run only in a trusted folder; system defaults merge with the user's
//!   settings (the `hooks` lists are concatenated).
//! - On Windows a hook runs in PowerShell, elsewhere in `bash -c`, so one command must run in
//!   both. The command is `node -e <relay>` ([`COMMAND_SCRIPT`]): Gemini runs on Node, and the
//!   script has neither `$` nor a backtick nor a double quote, which both shells would change.
//!
//! Every hook only reports, so every answer is exit 0 with no output ([`ANSWER`]): Gemini goes
//! on as if the hook were not there. [`Sessions::apply`] turns the reports into agent turns and
//! the terminal's status.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::companion::FileRef;

/// The extension id, as `--channel <id>/<name>` names it.
pub const EXTENSION_ID: &str = "fyx-gemini-cli";

/// `relay.respond(id, exit-code, stdout, stderr)` for every channel: let Gemini go on.
pub const ANSWER: (u8, &str, &str) = (0, "", "");

/// What `node -e` runs: `$FYX_RELAY` with the hook's arguments and stdin, or nothing (exit 0)
/// outside Fyx. Its exit code is the relay's.
pub const COMMAND_SCRIPT: &str = "const relay=process.env.FYX_RELAY;if(relay){const run=require('child_process').spawnSync(relay,process.argv.slice(1),{stdio:'inherit'});process.exit(run.status===null?0:run.status)}";

/// Gemini's hook timeout, in milliseconds; above every channel's `timeout_ms`.
pub const HOOK_TIMEOUT_MS: u64 = 10_000;

/// The `[[fyx_ui.relay_channels]]` of `extension.toml`, with the Gemini hook event each carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    SessionStart,
    BeforeAgent,
    BeforeTool,
    AfterTool,
    Notification,
    AfterAgent,
    SessionEnd,
}

impl Channel {
    pub const ALL: [Self; 7] = [
        Self::SessionStart,
        Self::BeforeAgent,
        Self::BeforeTool,
        Self::AfterTool,
        Self::Notification,
        Self::AfterAgent,
        Self::SessionEnd,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::BeforeAgent => "before-agent",
            Self::BeforeTool => "before-tool",
            Self::AfterTool => "after-tool",
            Self::Notification => "notification",
            Self::AfterAgent => "after-agent",
            Self::SessionEnd => "session-end",
        }
    }

    pub fn hook_event(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::BeforeAgent => "BeforeAgent",
            Self::BeforeTool => "BeforeTool",
            Self::AfterTool => "AfterTool",
            Self::Notification => "Notification",
            Self::AfterAgent => "AfterAgent",
            Self::SessionEnd => "SessionEnd",
        }
    }

    /// The relay request's channel: the bare name, or `<extension id>/<name>`.
    pub fn from_relay(channel: &str) -> Option<Self> {
        let name = channel
            .strip_prefix(EXTENSION_ID)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(channel);
        Self::ALL.into_iter().find(|channel| channel.name() == name)
    }

    /// The hook's command line in the defaults file.
    pub fn command(self) -> String {
        format!(
            "node -e \"{COMMAND_SCRIPT}\" -- --channel {EXTENSION_ID}/{}",
            self.name()
        )
    }
}

/// `gemini-settings/system-defaults.json`: one command hook per channel, nothing else.
pub fn defaults_file() -> Value {
    let hooks: serde_json::Map<String, Value> = Channel::ALL
        .into_iter()
        .map(|channel| {
            (
                channel.hook_event().to_owned(),
                serde_json::json!([{
                    "hooks": [{
                        "type": "command",
                        "name": format!("fyx-{}", channel.name()),
                        "command": channel.command(),
                        "timeout": HOOK_TIMEOUT_MS,
                    }]
                }]),
            )
        })
        .collect();
    serde_json::json!({ "hooks": hooks })
}

/// The tools whose `tool_input.file_path` is a file they write (`EDIT_TOOL_NAMES`).
const WRITE_TOOLS: [&str; 2] = ["replace", "write_file"];

/// One hook's input, reduced to what the extension uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookInput {
    pub channel: Channel,
    pub session: String,
    pub tool: Option<String>,
    pub message: Option<String>,
    /// The file a write tool touched, absolute (a relative `file_path` is joined to `cwd`).
    pub edited_path: Option<String>,
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

impl HookInput {
    /// `stdin` is Gemini's hook input; its `hook_event_name` must match the channel.
    pub fn parse(channel: Channel, stdin: &str) -> Result<Self, String> {
        let input: Value =
            serde_json::from_str(stdin).map_err(|error| format!("hook input: {error}"))?;
        let text = |name: &str| input.get(name).and_then(Value::as_str).map(str::to_owned);
        let event = text("hook_event_name").ok_or("hook input: no hook_event_name")?;
        if event != channel.hook_event() {
            return Err(format!("channel {} got a {event} hook", channel.name()));
        }
        let session = text("session_id").ok_or("hook input: no session_id")?;
        let tool = text("tool_name");
        let edited_path = match (channel, tool.as_deref()) {
            (Channel::AfterTool, Some(tool)) if WRITE_TOOLS.contains(&tool) => input
                .get("tool_input")
                .and_then(|tool_input| tool_input.get("file_path"))
                .and_then(Value::as_str)
                .map(|path| match text("cwd") {
                    Some(cwd) if !is_absolute(path) => {
                        format!("{}/{path}", cwd.trim_end_matches(['/', '\\']))
                    }
                    _ => path.to_owned(),
                }),
            _ => None,
        };
        Ok(Self {
            channel,
            session,
            tool,
            message: text("message"),
            edited_path,
        })
    }
}

/// `terminals.status-state`, as far as the hooks set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusState {
    Idle,
    Working,
    Blocked,
    Done,
}

/// `terminals.terminal-status`, as far as the hooks set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: StatusState,
    /// At most 64 bytes.
    pub label: String,
    pub detail: Option<String>,
    pub attention: bool,
}

/// What the extension does for a hook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    BeginTurn {
        session: String,
    },
    /// With the files the turn wrote, in Fyx's terms.
    EndTurn {
        session: String,
        changed: Vec<FileRef>,
    },
    /// On the relay request's terminal; `None` clears the extension's status.
    Status(Option<Status>),
}

/// The Gemini sessions the hooks have reported.
#[derive(Debug, Default)]
pub struct Sessions {
    sessions: BTreeMap<String, Session>,
}

#[derive(Debug, Default)]
pub struct Session {
    pub in_turn: bool,
    pub turns: u32,
    /// Files written in the current turn.
    pub changed: BTreeSet<FileRef>,
}

/// Status labels are at most 64 bytes; cut at a char boundary.
fn label(text: String) -> String {
    const MAX: usize = 64;
    if text.len() <= MAX {
        return text;
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn status(state: StatusState, text: &str) -> Effect {
    Effect::Status(Some(Status {
        state,
        label: label(text.to_owned()),
        detail: None,
        attention: false,
    }))
}

impl Sessions {
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn get(&self, session: &str) -> Option<&Session> {
        self.sessions.get(session)
    }

    /// The one session in a turn, which an `openDiff` belongs to; `None` with none or several.
    pub fn current(&self) -> Option<&str> {
        let mut in_turn = self
            .sessions
            .iter()
            .filter(|(_, session)| session.in_turn)
            .map(|(id, _)| id.as_str());
        match (in_turn.next(), in_turn.next()) {
            (Some(id), None) => Some(id),
            _ => None,
        }
    }

    /// The effects of one hook. `to_ref` maps Gemini's absolute paths to worktree files; a
    /// write outside every worktree is not part of the turn's changes.
    pub fn apply(
        &mut self,
        input: &HookInput,
        to_ref: impl Fn(&str) -> Option<FileRef>,
    ) -> Vec<Effect> {
        let end = |session: &mut Session, id: &str| Effect::EndTurn {
            session: id.to_owned(),
            changed: std::mem::take(&mut session.changed).into_iter().collect(),
        };
        let session = self.sessions.entry(input.session.clone()).or_default();
        match input.channel {
            Channel::SessionStart => vec![status(StatusState::Idle, "Gemini CLI")],
            Channel::BeforeAgent => {
                let mut effects = Vec::new();
                if session.in_turn {
                    // No AfterAgent came (an interrupted turn): close the old turn first.
                    effects.push(end(session, &input.session));
                }
                session.in_turn = true;
                session.turns += 1;
                effects.push(Effect::BeginTurn {
                    session: input.session.clone(),
                });
                effects.push(status(StatusState::Working, "Thinking"));
                effects
            }
            Channel::BeforeTool => vec![status(
                StatusState::Working,
                input.tool.as_deref().unwrap_or("Tool"),
            )],
            Channel::AfterTool => {
                if let Some(file) = input.edited_path.as_deref().and_then(&to_ref) {
                    session.changed.insert(file);
                }
                vec![status(StatusState::Working, "Thinking")]
            }
            Channel::Notification => vec![Effect::Status(Some(Status {
                state: StatusState::Blocked,
                label: "Waiting for you".into(),
                detail: input.message.clone(),
                attention: true,
            }))],
            Channel::AfterAgent => {
                let mut effects = Vec::new();
                if std::mem::take(&mut session.in_turn) {
                    effects.push(end(session, &input.session));
                }
                effects.push(status(StatusState::Done, "Done"));
                effects
            }
            Channel::SessionEnd => {
                let mut effects = Vec::new();
                if let Some(mut session) = self.sessions.remove(&input.session)
                    && session.in_turn
                {
                    effects.push(end(&mut session, &input.session));
                }
                effects.push(Effect::Status(None));
                effects
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(channel: Channel, extra: &str) -> HookInput {
        let stdin = format!(
            r#"{{"hook_event_name":"{}","session_id":"g1","cwd":"/work/app","transcript_path":"/tmp/t.json","timestamp":"2026-11-20T00:00:00Z"{extra}}}"#,
            channel.hook_event()
        );
        HookInput::parse(channel, &stdin).unwrap()
    }

    fn to_ref(path: &str) -> Option<FileRef> {
        path.strip_prefix("/work/app/").map(|path| FileRef {
            worktree: Some(1),
            path: path.into(),
        })
    }

    #[test]
    fn the_defaults_file_matches_the_manifest() {
        let shipped: Value =
            serde_json::from_str(include_str!("../gemini-settings/system-defaults.json")).unwrap();
        assert_eq!(
            shipped,
            defaults_file(),
            "regenerate gemini-settings/system-defaults.json"
        );
        let manifest = include_str!("../extension.toml");
        let mut channels = Vec::new();
        let mut in_channel = false;
        for line in manifest.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_channel = line == "[[fyx_ui.relay_channels]]";
            } else if in_channel && let Some(name) = line.strip_prefix("name = ") {
                channels.push(name.trim_matches('"'));
            }
        }
        assert_eq!(channels, Channel::ALL.map(Channel::name).to_vec());
        for channel in Channel::ALL {
            assert_eq!(
                Channel::from_relay(&format!("{EXTENSION_ID}/{}", channel.name())),
                Some(channel)
            );
        }
        assert_eq!(Channel::from_relay("other/after-agent"), None);
        // One command for bash and PowerShell: nothing either would expand or end.
        for forbidden in ['$', '`', '"'] {
            assert!(!COMMAND_SCRIPT.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn a_turn_collects_the_files_it_wrote() {
        let mut sessions = Sessions::default();
        sessions.apply(
            &input(Channel::SessionStart, r#","source":"startup""#),
            to_ref,
        );
        let begin = sessions.apply(&input(Channel::BeforeAgent, r#","prompt":"fix""#), to_ref);
        assert_eq!(
            begin[0],
            Effect::BeginTurn {
                session: "g1".into()
            }
        );
        assert_eq!(sessions.current(), Some("g1"));
        for (tool, path) in [
            ("replace", "/work/app/src/a.rs"),
            ("write_file", "src/b.rs"),
            ("write_file", "/elsewhere/c.rs"),
            ("read_file", "/work/app/d.rs"),
        ] {
            sessions.apply(
                &input(
                    Channel::AfterTool,
                    &format!(
                        r#","tool_name":"{tool}","tool_input":{{"file_path":"{path}"}},"tool_response":{{}}"#
                    ),
                ),
                to_ref,
            );
        }
        let end = sessions.apply(
            &input(
                Channel::AfterAgent,
                r#","prompt":"fix","prompt_response":"done","stop_hook_active":false"#,
            ),
            to_ref,
        );
        assert_eq!(
            end[0],
            Effect::EndTurn {
                session: "g1".into(),
                changed: vec![
                    FileRef {
                        worktree: Some(1),
                        path: "src/a.rs".into()
                    },
                    FileRef {
                        worktree: Some(1),
                        path: "src/b.rs".into()
                    },
                ],
            }
        );
        assert_eq!(sessions.current(), None);
        // A second AfterAgent (a hook made the agent go on) ends nothing.
        let again = sessions.apply(&input(Channel::AfterAgent, ""), to_ref);
        assert!(
            !again
                .iter()
                .any(|effect| matches!(effect, Effect::EndTurn { .. }))
        );
        let ended = sessions.apply(&input(Channel::SessionEnd, r#","reason":"exit""#), to_ref);
        assert_eq!(ended, [Effect::Status(None)]);
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn waits_interrupted_turns_and_wrong_channels() {
        let mut sessions = Sessions::default();
        let effects = sessions.apply(
            &input(
                Channel::Notification,
                r#","notification_type":"ToolPermission","message":"Allow shell?","details":{}"#,
            ),
            to_ref,
        );
        let [Effect::Status(Some(status))] = effects.as_slice() else {
            panic!("{effects:?}");
        };
        assert_eq!(status.state, StatusState::Blocked);
        assert_eq!(status.detail.as_deref(), Some("Allow shell?"));
        sessions.apply(&input(Channel::BeforeAgent, ""), to_ref);
        let next = sessions.apply(&input(Channel::BeforeAgent, ""), to_ref);
        assert!(matches!(next[0], Effect::EndTurn { .. }), "{next:?}");
        assert!(matches!(next[1], Effect::BeginTurn { .. }), "{next:?}");
        let wrong = HookInput::parse(
            Channel::AfterAgent,
            r#"{"hook_event_name":"BeforeAgent","session_id":"g1"}"#,
        );
        assert!(wrong.unwrap_err().contains("got a BeforeAgent hook"));
        assert!(HookInput::parse(Channel::AfterAgent, "{}").is_err());
        assert_eq!(label("é".repeat(40)).len(), 64);
        assert!(is_absolute(r"C:\work\a.rs") && is_absolute("/a") && !is_absolute("a/b"));
    }
}
