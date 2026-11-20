//! The Claude plugin's hooks (`claude-plugin/hooks/hooks.json`), as they reach the extension:
//! `io-event::relay-request(id, channel, terminal, argv, stdin)` with Claude's hook input JSON on
//! stdin (design "The first extensions", items 2 and 4).
//!
//! Every hook but one only reports, so its answer is exit 0 with no output ([`ANSWER`]): Claude
//! goes on as if the hook were not there. The exception is a `Bash` command that zsh would
//! silently change (`"$M:refs/heads/dev"`, see [`crate::zsh`]): its `pre-tool` hook denies it
//! and says how to write it ([`answer`]). [`Sessions::apply`] turns the reports into the effects the
//! extension performs: agent turns (`review.begin-turn`, `review.end-turn`) and the terminal's
//! status (`terminals.set-status`). OSC 7501 already gives the tab its state; the hooks add the
//! tool, the waiting message and the turn.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::ide::FileRef;

/// The extension id, as `--channel <id>/<name>` names it.
pub const EXTENSION_ID: &str = "fyx-claude-code";

/// `relay.respond(id, exit-code, stdout, stderr)` for a hook that only reports: let Claude go
/// on.
pub const ANSWER: (u8, &str, &str) = (0, "", "");

/// The `pre-tool` hook's arguments after the channel: `--shell "$CLAUDE_CODE_SHELL" "$SHELL"`,
/// the variables Claude Code picks its Bash tool's shell from.
pub const PRE_TOOL_ARGS: &str = r#" --shell "${CLAUDE_CODE_SHELL:-}" "${SHELL:-}""#;

/// The answer to one hook. `argv` is the relay request's: for `pre-tool`, [`PRE_TOOL_ARGS`]
/// expanded. A `Bash` command with a zsh modifier hazard, run by zsh, is denied; everything else
/// gets [`ANSWER`].
pub fn answer(input: Option<&HookInput>, argv: &[String]) -> (u8, String, String) {
    let reported = || (ANSWER.0, ANSWER.1.to_owned(), ANSWER.2.to_owned());
    let Some(command) = input
        .filter(|input| input.channel == Channel::PreTool)
        .and_then(|input| input.command.as_deref())
    else {
        return reported();
    };
    let shells = argv
        .iter()
        .position(|argument| argument == "--shell")
        .map_or(&[][..], |position| &argv[position + 1..]);
    if !crate::zsh::shell_is_zsh(shells.iter().map(String::as_str)) {
        return reported();
    }
    match crate::zsh::deny_answer(&crate::zsh::hazards(command)) {
        Some(stdout) => (0, stdout, String::new()),
        None => reported(),
    }
}

/// The `[[fyx_ui.relay_channels]]` of `extension.toml`, with the Claude hook event each carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    SessionStart,
    UserPrompt,
    PreTool,
    PostTool,
    Notification,
    Stop,
    SessionEnd,
}

impl Channel {
    pub const ALL: [Self; 7] = [
        Self::SessionStart,
        Self::UserPrompt,
        Self::PreTool,
        Self::PostTool,
        Self::Notification,
        Self::Stop,
        Self::SessionEnd,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::UserPrompt => "user-prompt",
            Self::PreTool => "pre-tool",
            Self::PostTool => "post-tool",
            Self::Notification => "notification",
            Self::Stop => "stop",
            Self::SessionEnd => "session-end",
        }
    }

    pub fn hook_event(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPrompt => "UserPromptSubmit",
            Self::PreTool => "PreToolUse",
            Self::PostTool => "PostToolUse",
            Self::Notification => "Notification",
            Self::Stop => "Stop",
            Self::SessionEnd => "SessionEnd",
        }
    }

    /// The relay request's channel: the bare name, or `<extension id>/<name>` as the hook
    /// passes it.
    pub fn from_relay(channel: &str) -> Option<Self> {
        let name = channel
            .strip_prefix(EXTENSION_ID)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(channel);
        Self::ALL.into_iter().find(|channel| channel.name() == name)
    }
}

/// One hook's input, reduced to what the extension uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookInput {
    pub channel: Channel,
    pub session: String,
    /// `PreToolUse` and `PostToolUse`.
    pub tool: Option<String>,
    /// `Notification`'s message.
    pub message: Option<String>,
    /// The file a write tool (`Edit`, `MultiEdit`, `Write`, `NotebookEdit`) touched, as Claude
    /// names it (absolute).
    pub edited_path: Option<String>,
    /// `PreToolUse` of `Bash`: the command it runs.
    pub command: Option<String>,
}

/// The tools whose `tool_input.file_path` (or `notebook_path`) is a file they write.
const WRITE_TOOLS: [&str; 4] = ["Edit", "MultiEdit", "Write", "NotebookEdit"];

impl HookInput {
    /// `stdin` is Claude's hook input. Its `hook_event_name` must match the channel, so a hook
    /// wired to the wrong channel is noticed instead of misread.
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
            (Channel::PostTool, Some(tool)) if WRITE_TOOLS.contains(&tool) => input
                .get("tool_input")
                .and_then(|tool_input| {
                    tool_input
                        .get("file_path")
                        .or_else(|| tool_input.get("notebook_path"))
                })
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
        let command = match (channel, tool.as_deref()) {
            (Channel::PreTool, Some("Bash")) => input
                .get("tool_input")
                .and_then(|tool_input| tool_input.get("command"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
        Ok(Self {
            channel,
            session,
            tool,
            message: text("message"),
            edited_path,
            command,
        })
    }
}

/// `terminals.status-state`.
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

/// The Claude sessions the hooks have reported.
#[derive(Debug, Default)]
pub struct Sessions {
    sessions: BTreeMap<String, Session>,
}

#[derive(Debug, Default)]
pub struct Session {
    pub in_turn: bool,
    pub turns: u32,
    pub tool: Option<String>,
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

impl Sessions {
    pub fn get(&self, session: &str) -> Option<&Session> {
        self.sessions.get(session)
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
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

    /// The user rejected an `openDiff` in Fyx, and the extension answered `DIFF_REJECTED`. Claude
    /// 2.1.326 then ends the turn with no `Stop` hook (live recording, 2026-11-19), so the
    /// extension ends it here. The turn is the one session whose current tool is an edit; with
    /// none or several, nothing happens, and the next prompt or `Stop` closes the turn.
    pub fn diff_rejected(&mut self) -> Vec<Effect> {
        let editing: Vec<&String> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.in_turn
                    && session
                        .tool
                        .as_deref()
                        .is_some_and(|tool| WRITE_TOOLS.contains(&tool))
            })
            .map(|(id, _)| id)
            .collect();
        let [id] = editing[..] else {
            return Vec::new();
        };
        let id = id.clone();
        let Some(session) = self.sessions.get_mut(&id) else {
            return Vec::new();
        };
        session.in_turn = false;
        session.tool = None;
        vec![
            Effect::EndTurn {
                session: id,
                changed: std::mem::take(&mut session.changed).into_iter().collect(),
            },
            Effect::Status(Some(Status {
                state: StatusState::Done,
                label: "Edit rejected".into(),
                detail: None,
                attention: false,
            })),
        ]
    }

    /// The effects of one hook. `to_ref` maps Claude's absolute paths, in
    /// [`crate::ide::normalize_path`]'s form, to worktree files; a write
    /// outside every worktree is not part of the turn's changes.
    pub fn apply(
        &mut self,
        input: &HookInput,
        to_ref: impl Fn(&str) -> Option<FileRef>,
    ) -> Vec<Effect> {
        let session = self.sessions.entry(input.session.clone()).or_default();
        match input.channel {
            Channel::SessionStart => vec![Effect::Status(Some(Status {
                state: StatusState::Idle,
                label: "Claude Code".into(),
                detail: None,
                attention: false,
            }))],
            Channel::UserPrompt => {
                let mut effects = Vec::new();
                if session.in_turn {
                    // No Stop came (an interrupted turn): close the old turn first.
                    effects.push(Effect::EndTurn {
                        session: input.session.clone(),
                        changed: std::mem::take(&mut session.changed).into_iter().collect(),
                    });
                }
                session.in_turn = true;
                session.turns += 1;
                session.tool = None;
                effects.push(Effect::BeginTurn {
                    session: input.session.clone(),
                });
                effects.push(Effect::Status(Some(Status {
                    state: StatusState::Working,
                    label: "Thinking".into(),
                    detail: None,
                    attention: false,
                })));
                effects
            }
            Channel::PreTool => {
                session.tool = input.tool.clone();
                vec![Effect::Status(Some(Status {
                    state: StatusState::Working,
                    label: label(input.tool.clone().unwrap_or_else(|| "Tool".into())),
                    detail: None,
                    attention: false,
                }))]
            }
            Channel::PostTool => {
                session.tool = None;
                if let Some(file) = input
                    .edited_path
                    .as_deref()
                    .and_then(|path| to_ref(&crate::ide::normalize_path(path)))
                {
                    session.changed.insert(file);
                }
                vec![Effect::Status(Some(Status {
                    state: StatusState::Working,
                    label: "Thinking".into(),
                    detail: None,
                    attention: false,
                }))]
            }
            Channel::Notification => vec![Effect::Status(Some(Status {
                state: StatusState::Blocked,
                label: "Waiting for you".into(),
                detail: input.message.clone(),
                attention: true,
            }))],
            Channel::Stop => {
                let mut effects = Vec::new();
                if std::mem::take(&mut session.in_turn) {
                    effects.push(Effect::EndTurn {
                        session: input.session.clone(),
                        changed: std::mem::take(&mut session.changed).into_iter().collect(),
                    });
                }
                session.tool = None;
                effects.push(Effect::Status(Some(Status {
                    state: StatusState::Done,
                    label: "Done".into(),
                    detail: None,
                    attention: false,
                })));
                effects
            }
            Channel::SessionEnd => {
                let mut effects = Vec::new();
                if let Some(mut session) = self.sessions.remove(&input.session)
                    && session.in_turn
                {
                    effects.push(Effect::EndTurn {
                        session: input.session.clone(),
                        changed: std::mem::take(&mut session.changed).into_iter().collect(),
                    });
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
            r#"{{"hook_event_name":"{}","session_id":"s1","cwd":"/work/app"{extra}}}"#,
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
    fn channels_match_the_manifest_and_the_plugin() {
        let manifest: toml_lite::Manifest = toml_lite::read();
        assert_eq!(
            manifest.relay_channels,
            Channel::ALL.map(Channel::name).to_vec()
        );
        let hooks: Value =
            serde_json::from_str(include_str!("../claude-plugin/hooks/hooks.json")).unwrap();
        let hooks = hooks["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), Channel::ALL.len());
        for channel in Channel::ALL {
            let command = hooks[channel.hook_event()][0]["hooks"][0]["command"]
                .as_str()
                .unwrap();
            let arguments = if channel == Channel::PreTool {
                PRE_TOOL_ARGS
            } else {
                ""
            };
            assert_eq!(
                command,
                format!(
                    "\"${{FYX_RELAY:-true}}\" --channel {EXTENSION_ID}/{}{arguments}",
                    channel.name()
                )
            );
            assert_eq!(
                Channel::from_relay(&format!("{EXTENSION_ID}/{}", channel.name())),
                Some(channel)
            );
        }
        assert_eq!(Channel::from_relay("other/stop"), None);
    }

    /// Only a zsh `Bash` command with a modifier hazard is denied; the shell comes from the
    /// `pre-tool` hook's `--shell` arguments.
    #[test]
    fn pre_tool_denies_zsh_modifier_hazards_only() {
        let bash = |command: &str| {
            input(
                Channel::PreTool,
                &format!(
                    r#","tool_name":"Bash","tool_input":{{"command":{}}}"#,
                    serde_json::to_string(command).unwrap()
                ),
            )
        };
        let argv = |claude_code_shell: &str, shell: &str| {
            ["--shell", claude_code_shell, shell].map(str::to_owned).to_vec()
        };
        let pushed = bash(r#"git push origin "$M:refs/heads/dev""#);
        assert_eq!(pushed.command.as_deref(), Some(r#"git push origin "$M:refs/heads/dev""#));

        let (exit, stdout, stderr) = answer(Some(&pushed), &argv("", "/bin/zsh"));
        assert_eq!((exit, stderr.as_str()), (0, ""));
        let stdout: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(stdout["hookSpecificOutput"]["permissionDecision"], "deny");

        let reported = (0, String::new(), String::new());
        assert_eq!(answer(Some(&pushed), &argv("/bin/bash", "/bin/zsh")), reported);
        assert_eq!(answer(Some(&pushed), &argv("", "")), reported);
        assert_eq!(answer(Some(&pushed), &[]), reported);
        assert_eq!(answer(None, &argv("", "/bin/zsh")), reported);
        assert_eq!(
            answer(
                Some(&bash(r#"git push origin "${M}:refs/heads/dev""#)),
                &argv("", "/bin/zsh")
            ),
            reported
        );
        let edit = input(
            Channel::PreTool,
            r#","tool_name":"Edit","tool_input":{"command":"echo $M:r"}"#,
        );
        assert_eq!(edit.command, None);
        assert_eq!(answer(Some(&edit), &argv("", "/bin/zsh")), reported);
    }

    #[test]
    fn a_turn_collects_the_files_it_wrote() {
        let mut sessions = Sessions::default();
        sessions.apply(
            &input(Channel::SessionStart, r#","source":"startup""#),
            to_ref,
        );
        let begin = sessions.apply(&input(Channel::UserPrompt, r#","prompt":"fix it""#), to_ref);
        assert_eq!(
            begin[0],
            Effect::BeginTurn {
                session: "s1".into()
            }
        );
        for path in [
            "/work/app/src/a.rs",
            "/elsewhere/b.rs",
            "/work/app/src/a.rs",
        ] {
            sessions.apply(
                &input(
                    Channel::PostTool,
                    &format!(r#","tool_name":"Edit","tool_input":{{"file_path":"{path}"}}"#),
                ),
                to_ref,
            );
        }
        sessions.apply(
            &input(
                Channel::PostTool,
                r#","tool_name":"Read","tool_input":{"file_path":"/work/app/c.rs"}"#,
            ),
            to_ref,
        );
        let stop = sessions.apply(&input(Channel::Stop, ""), to_ref);
        assert_eq!(
            stop[0],
            Effect::EndTurn {
                session: "s1".into(),
                changed: vec![FileRef {
                    worktree: Some(1),
                    path: "src/a.rs".into()
                }],
            }
        );
        // A second Stop (a Stop hook that continued) ends nothing.
        let again = sessions.apply(&input(Channel::Stop, ""), to_ref);
        assert!(
            !again
                .iter()
                .any(|effect| matches!(effect, Effect::EndTurn { .. }))
        );
        assert_eq!(sessions.get("s1").unwrap().turns, 1);
    }

    #[test]
    fn a_rejected_diff_ends_the_editing_turn() {
        let mut sessions = Sessions::default();
        // Nothing is editing: nothing ends.
        assert!(sessions.diff_rejected().is_empty());
        sessions.apply(&input(Channel::UserPrompt, r#","prompt":"fix""#), to_ref);
        // A Windows path from Claude is normalized before `to_ref`.
        let windows = |path: &str| {
            path.strip_prefix("C:/work/app/").map(|path| FileRef {
                worktree: Some(1),
                path: path.into(),
            })
        };
        sessions.apply(
            &input(
                Channel::PostTool,
                r#","tool_name":"Write","tool_input":{"file_path":"c:\\work\\app\\b.rs"}"#,
            ),
            windows,
        );
        sessions.apply(&input(Channel::PreTool, r#","tool_name":"Edit""#), to_ref);
        let effects = sessions.diff_rejected();
        assert_eq!(
            effects[0],
            Effect::EndTurn {
                session: "s1".into(),
                changed: vec![FileRef {
                    worktree: Some(1),
                    path: "b.rs".into()
                }],
            }
        );
        let Effect::Status(Some(status)) = &effects[1] else {
            panic!("{effects:?}");
        };
        assert_eq!(status.state, StatusState::Done);
        // The turn is over: neither a second rejection nor the next prompt ends it again.
        assert!(sessions.diff_rejected().is_empty());
        let next = sessions.apply(&input(Channel::UserPrompt, r#","prompt":"go""#), to_ref);
        assert!(
            !next
                .iter()
                .any(|effect| matches!(effect, Effect::EndTurn { .. }))
        );
    }

    #[test]
    fn waits_and_wrong_channels() {
        let mut sessions = Sessions::default();
        let effects = sessions.apply(
            &input(
                Channel::Notification,
                r#","message":"Claude needs your permission to use Bash""#,
            ),
            to_ref,
        );
        let [Effect::Status(Some(status))] = effects.as_slice() else {
            panic!("{effects:?}");
        };
        assert_eq!(status.state, StatusState::Blocked);
        assert!(status.attention);
        assert_eq!(
            status.detail.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        let wrong = HookInput::parse(
            Channel::Stop,
            r#"{"hook_event_name":"PreToolUse","session_id":"s1"}"#,
        );
        assert!(wrong.unwrap_err().contains("got a PreToolUse hook"));
        assert!(HookInput::parse(Channel::Stop, "{}").is_err());
        assert_eq!(label("é".repeat(40)).len(), 64);
    }

    /// The relay channel names of `extension.toml`, without a TOML parser in the guest.
    mod toml_lite {
        pub struct Manifest {
            pub relay_channels: Vec<&'static str>,
        }

        pub fn read() -> Manifest {
            let text: &'static str = include_str!("../extension.toml");
            let mut relay_channels = Vec::new();
            let mut in_channel = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("[[") || line.starts_with('[') {
                    in_channel = line == "[[fyx_ui.relay_channels]]";
                } else if in_channel && let Some(name) = line.strip_prefix("name = ") {
                    relay_channels.push(name.trim_matches('"'));
                }
            }
            Manifest { relay_channels }
        }
    }
}
