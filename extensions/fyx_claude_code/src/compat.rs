//! The compat test (design "The first extensions", item 5): a protocol transcript from a pinned
//! Claude Code version, replayed against the extension's protocol core. The format is in
//! `compat/README.md`. When the SDK and harness gain listeners and the relay (plan 5.14), the
//! same transcripts are replayed through `fyx_ext_harness` against the built component.

use serde_json::{Value, json};

use crate::hooks::{Channel, Effect, HookInput, Sessions, StatusState};
use crate::ide::{
    self, Diagnostic, DiagnosticsCache, DiffAnswer, Diffs, IdeCall, Position, ReviewDecision,
    Roots, Severity, TextRange,
};

pub const FORMAT: &str = "fyx-claude-transcript";
pub const VERSION: u64 = 1;

/// The version every transcript in `compat/` must name, and that the manual check runs before
/// each release. Moving it means recording again.
pub const PINNED_CLAUDE_CODE: &str = "2.1.326";

/// The transcripts, by file name.
pub const TRANSCRIPTS: &[(&str, &str)] = &[
    (
        "turn-with-diff.jsonl",
        include_str!("compat/turn-with-diff.jsonl"),
    ),
    (
        "live-interactive.jsonl",
        include_str!("compat/live-interactive.jsonl"),
    ),
    (
        "live-interactive-windows.jsonl",
        include_str!("compat/live-interactive-windows.jsonl"),
    ),
    (
        "live-print-hooks.jsonl",
        include_str!("compat/live-print-hooks.jsonl"),
    ),
];

/// The first line.
#[derive(Debug)]
pub struct Header {
    pub claude_code: String,
    /// `interactive` or `print` (`claude -p`): an interactive session runs the plugin's hooks
    /// only after the folder is trusted; `-p` runs them at once.
    pub mode: String,
    /// `synthetic` until recorded from the CLI, then the recording date.
    pub recorded: String,
    /// The workspace root the transcript's absolute paths are under.
    pub root: String,
}

/// Replays one transcript; the error names the line that did not match.
pub fn replay(text: &str) -> Result<Header, String> {
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    let (_, first) = lines.next().ok_or("an empty transcript")?;
    let (header, mut replay) = start(first, false)?;
    for (index, line) in lines {
        let at = |why: String| format!("line {}: {why}", index + 1);
        let event: Value = serde_json::from_str(line).map_err(|error| at(error.to_string()))?;
        replay.step(&event).map_err(at)?;
    }
    replay.finish()?;
    Ok(header)
}

/// Turns a draft made from a recording into a transcript. The draft has Claude's messages, the
/// hooks without `expect`, and the `review`, `editor`, `mention` and `diagnostics` events. Each
/// message the extension produces is written after the event that produced it, and each hook
/// gets the effects it has now. The draft's `out` lines are dropped, except the host's own
/// (`initialize`, `tools/list`). Read the result against the recording before committing it:
/// it states what the extension does, not what Claude accepted.
pub fn fill(draft: &str) -> Result<String, String> {
    let mut lines = draft
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    let (_, first) = lines.next().ok_or("an empty draft")?;
    let (_, mut replay) = start(first, true)?;
    let mut out = vec![first.to_owned()];
    for (index, line) in lines {
        let at = |why: String| format!("line {}: {why}", index + 1);
        let mut event: Value = serde_json::from_str(line).map_err(|error| at(error.to_string()))?;
        if event["t"] == "mcp" && event["dir"] == "out" {
            let result = &event["msg"]["result"];
            if result["serverInfo"].is_null() && result["tools"].is_null() {
                continue;
            }
        }
        replay.step(&event).map_err(at)?;
        if event["t"] == "hook" || (event["t"] == "review" && !replay.last_effects.is_empty()) {
            event["expect"] = Value::Array(std::mem::take(&mut replay.last_effects));
        }
        out.push(event.to_string());
        for message in replay.expected_out.drain(..) {
            out.push(json!({ "t": "mcp", "dir": "out", "msg": message }).to_string());
        }
    }
    replay.finish()?;
    out.push(String::new());
    Ok(out.join("\n"))
}

fn start(first: &str, filling: bool) -> Result<(Header, Replay), String> {
    let header: Value = serde_json::from_str(first).map_err(|error| format!("line 1: {error}"))?;
    let field = |name: &str| {
        header[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("line 1: no {name:?}"))
    };
    if header["format"] != FORMAT || header["version"] != VERSION {
        return Err(format!("line 1: not {FORMAT} version {VERSION}"));
    }
    let header = Header {
        claude_code: field("claude_code")?,
        mode: field("mode")?,
        recorded: field("recorded")?,
        root: field("root")?,
    };
    if !matches!(header.mode.as_str(), "interactive" | "print") {
        return Err(format!("line 1: mode {:?}", header.mode));
    }
    let replay = Replay {
        roots: PrefixRoot(header.root.clone()),
        sessions: Sessions::default(),
        diffs: Diffs::default(),
        expected_out: Vec::new(),
        diagnostics: Vec::new(),
        cache: DiagnosticsCache::default(),
        filling,
        last_effects: Vec::new(),
    };
    Ok((header, replay))
}

/// The transcript's one workspace root, as worktree 1: `/work/app`, or `C:\\work\\app` for a
/// transcript recorded on Windows, whose paths keep Claude's own forms.
struct PrefixRoot(String);

impl Roots for PrefixRoot {
    fn to_ref(&self, absolute: &str) -> Option<ide::FileRef> {
        let root = ide::normalize_path(&self.0);
        let relative = absolute.strip_prefix(&root)?.strip_prefix('/')?;
        (!relative.is_empty()
            && relative
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
        .then(|| ide::FileRef {
            worktree: Some(1),
            path: relative.to_owned(),
        })
    }

    fn to_absolute(&self, file: &ide::FileRef) -> Option<String> {
        (file.worktree == Some(1)).then(|| {
            if self.0.contains('\\') {
                format!("{}\\{}", self.0, file.path.replace('/', "\\"))
            } else {
                format!("{}/{}", self.0, file.path)
            }
        })
    }
}

struct Replay {
    roots: PrefixRoot,
    sessions: Sessions,
    /// The open `openDiff` calls, by JSON-RPC id.
    diffs: Diffs<String>,
    /// Messages the extension produced that the transcript has not shown yet, in order.
    expected_out: Vec<Value>,
    /// What the host reports for `getDiagnostics`, by absolute path; set by `diagnostics`.
    diagnostics: Vec<(String, Vec<Diagnostic>)>,
    /// Cleared by a `diagnostics` event and at each prompt.
    cache: DiagnosticsCache,
    /// [`fill`]: hooks are not checked, and their effects go to `last_effects`.
    filling: bool,
    last_effects: Vec<Value>,
}

impl Replay {
    fn finish(&self) -> Result<(), String> {
        if let Some(left) = self.expected_out.first() {
            return Err(format!("the extension never sent {left}"));
        }
        if !self.diffs.is_empty() {
            return Err(format!(
                "openDiff calls left open: {:?}",
                self.diffs.keys().collect::<Vec<_>>()
            ));
        }
        Ok(())
    }

    fn step(&mut self, event: &Value) -> Result<(), String> {
        match event["t"].as_str() {
            Some("mcp") => match event["dir"].as_str() {
                Some("in") => self.from_claude(&event["msg"]),
                Some("out") => self.to_claude(&event["msg"]),
                other => Err(format!("mcp dir {other:?}")),
            },
            Some("hook") => self.hook(event),
            Some("review") => self.review(event),
            Some("editor") => self.editor(event),
            Some("mention") => self.mention(event),
            Some("diagnostics") => self.set_diagnostics(event),
            other => Err(format!("unknown event type {other:?}")),
        }
    }

    /// Claude → Fyx. The host answers `initialize`, `tools/list` and Claude's notifications
    /// (`notifications/initialized`, `ide_connected`); the extension answers `tools/call`.
    fn from_claude(&mut self, message: &Value) -> Result<(), String> {
        let method = message["method"]
            .as_str()
            .ok_or("a message without method")?;
        if method != "tools/call" {
            return Ok(());
        }
        let id = message["id"].to_string();
        let params = &message["params"];
        let tool = params["name"].as_str().ok_or("tools/call without a name")?;
        let arguments = params["arguments"].to_string();
        match IdeCall::parse(tool, &arguments, &self.roots) {
            Ok(IdeCall::OpenDiff { tab, .. }) => {
                self.diffs.opened(tab, id);
            }
            // A close withdraws only a proposal that is still open, whose call then ends with
            // `TAB_CLOSED`; after a decision it is a no-op.
            Ok(IdeCall::CloseTab { tab }) => {
                if let Some(call) = self.diffs.close(&tab) {
                    self.respond(&call, Ok(DiffAnswer::TabClosed.result()));
                }
                self.respond(&id, Ok(ide::text_result("TAB_CLOSED")));
            }
            Ok(IdeCall::CloseAllDiffTabs) => {
                for call in self.diffs.close_all() {
                    self.respond(&call, Ok(DiffAnswer::TabClosed.result()));
                }
                self.respond(&id, Ok(ide::text_result("CLOSED_ALL_DIFF_TABS")));
            }
            Ok(IdeCall::GetDiagnostics { file }) => {
                // One file, or every file; a file the host has nothing for is left out.
                let only = file.as_ref().and_then(|file| self.roots.to_absolute(file));
                let diagnostics = &self.diagnostics;
                let answer = self.cache.answer(file, || {
                    let files: Vec<_> = diagnostics
                        .iter()
                        .filter(|(path, _)| only.as_ref().is_none_or(|only| only == path))
                        .cloned()
                        .collect();
                    ide::diagnostics_result(&files)
                });
                self.respond(&id, Ok(answer));
            }
            Ok(IdeCall::OpenFile { .. }) => {
                self.respond(&id, Ok(ide::text_result("OK")));
            }
            Err(error) => self.respond(&id, Err(error)),
        }
        Ok(())
    }

    fn respond(&mut self, id: &str, outcome: Result<String, ide::McpError>) {
        let id: Value = serde_json::from_str(id).unwrap_or(Value::Null);
        self.expected_out.push(match outcome {
            Ok(result) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": serde_json::from_str::<Value>(&result).unwrap_or(Value::Null),
            }),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": error.code, "message": error.message },
            }),
        });
    }

    /// Fyx → Claude. A response or notification the extension produced must come next, as it
    /// is; the host's own (`initialize`, `tools/list`) are checked for what the extension
    /// registered.
    fn to_claude(&mut self, message: &Value) -> Result<(), String> {
        if let Some(tools) = message["result"]["tools"].as_array() {
            let names: Vec<&str> = tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect();
            let ours: Vec<&str> = ide::tools().iter().map(|tool| tool.name).collect();
            return (names == ours).then_some(()).ok_or_else(|| {
                format!("tools/list has {names:?}, the extension registers {ours:?}")
            });
        }
        if message["result"]["serverInfo"].is_object() {
            return Ok(());
        }
        if self.expected_out.is_empty() {
            return Err(format!(
                "the extension sent nothing, the transcript has {message}"
            ));
        }
        let ours = self.expected_out.remove(0);
        (ours == *message)
            .then_some(())
            .ok_or_else(|| format!("the extension sent {ours}, the transcript has {message}"))
    }

    /// The user's decision on the open `openDiff` (in Fyx's agent review).
    fn review(&mut self, event: &Value) -> Result<(), String> {
        let id = event["call"].to_string();
        if !self.diffs.decided(&id) {
            return Err(format!("no open openDiff with id {id}"));
        }
        let answer = match event["decision"].as_str() {
            Some("accepted") => DiffAnswer::from_review(
                ReviewDecision::Accepted,
                event["final_content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            ),
            Some("partial") => DiffAnswer::from_review(
                ReviewDecision::Partial,
                event["final_content"]
                    .as_str()
                    .ok_or("partial needs final_content")?
                    .to_owned(),
            ),
            Some("rejected") => DiffAnswer::Rejected,
            Some("closed") => DiffAnswer::TabClosed,
            other => return Err(format!("review decision {other:?}")),
        };
        self.respond(&id, Ok(answer.result()));
        // Claude ends the turn after a rejection without `Stop`; the extension ends it.
        let effects: Vec<Value> = if answer == DiffAnswer::Rejected {
            self.sessions.diff_rejected().iter().map(describe).collect()
        } else {
            Vec::new()
        };
        if self.filling {
            self.last_effects = effects;
            return Ok(());
        }
        let expected = event["expect"].as_array().cloned().unwrap_or_default();
        (effects == expected)
            .then_some(())
            .ok_or_else(|| format!("effects {effects:?}, expected {expected:?}"))
    }

    /// The editor's selection changed (`io-event::context-changed`).
    fn editor(&mut self, event: &Value) -> Result<(), String> {
        let selection = &event["selection"];
        let number = |value: &Value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or("a selection position")
        };
        let range = TextRange {
            start: Position {
                line: number(&selection["start"][0])?,
                column: number(&selection["start"][1])?,
            },
            end: Position {
                line: number(&selection["end"][0])?,
                column: number(&selection["end"][1])?,
            },
        };
        let file = ide::FileRef {
            worktree: Some(1),
            path: event["path"]
                .as_str()
                .ok_or("editor event without path")?
                .to_owned(),
        };
        let params = ide::selection_changed(&file, range, selection["text"].as_str(), &self.roots)
            .ok_or("the selection's file is outside the root")?;
        self.expected_out.push(json!({
            "jsonrpc": "2.0",
            "method": "selection_changed",
            "params": serde_json::from_str::<Value>(&params).unwrap_or(Value::Null),
        }));
        Ok(())
    }

    /// The user sent a file or lines to Claude (`at_mentioned`); `lines` is zero-based.
    fn mention(&mut self, event: &Value) -> Result<(), String> {
        let file = worktree_file(event)?;
        let lines = match event["lines"].as_array().map(Vec::as_slice) {
            Some([start, end]) => Some((position_number(start)?, position_number(end)?)),
            _ => None,
        };
        let params = ide::at_mentioned(&file, lines, &self.roots)
            .ok_or("the mention's file is outside the root")?;
        self.expected_out.push(json!({
            "jsonrpc": "2.0",
            "method": "at_mentioned",
            "params": serde_json::from_str::<Value>(&params).unwrap_or(Value::Null),
        }));
        Ok(())
    }

    /// What the host's diagnostics hold from now on: `files` as `{path, diagnostics}`, each
    /// diagnostic `{range: {start: [line, column], end}, severity, message, source}`.
    fn set_diagnostics(&mut self, event: &Value) -> Result<(), String> {
        let files = event["files"]
            .as_array()
            .ok_or("diagnostics without files")?;
        self.diagnostics = files
            .iter()
            .map(|entry| {
                let file = worktree_file(entry)?;
                let absolute = self
                    .roots
                    .to_absolute(&file)
                    .ok_or("a diagnostics file outside the root")?;
                let diagnostics = entry["diagnostics"]
                    .as_array()
                    .ok_or("a file without diagnostics")?
                    .iter()
                    .map(diagnostic)
                    .collect::<Result<Vec<_>, String>>()?;
                Ok((absolute, diagnostics))
            })
            .collect::<Result<_, String>>()?;
        self.cache.invalidate();
        Ok(())
    }

    /// A relay request; `expect` lists the effects, in [`describe`]'s form.
    fn hook(&mut self, event: &Value) -> Result<(), String> {
        let channel = event["channel"].as_str().ok_or("hook without channel")?;
        let channel =
            Channel::from_relay(channel).ok_or_else(|| format!("unknown channel {channel:?}"))?;
        let input = HookInput::parse(channel, &event["stdin"].to_string())?;
        if channel == Channel::UserPrompt {
            self.cache.invalidate();
        }
        let roots = &self.roots;
        let effects = self.sessions.apply(&input, |path| roots.to_ref(path));
        let effects: Vec<Value> = effects.iter().map(describe).collect();
        if self.filling {
            self.last_effects = effects;
            return Ok(());
        }
        let expected = event["expect"].as_array().cloned().unwrap_or_default();
        (effects == expected)
            .then_some(())
            .ok_or_else(|| format!("effects {effects:?}, expected {expected:?}"))
    }
}

/// `path` as a file of worktree 1.
fn worktree_file(event: &Value) -> Result<ide::FileRef, String> {
    Ok(ide::FileRef {
        worktree: Some(1),
        path: event["path"].as_str().ok_or("no path")?.to_owned(),
    })
}

fn position_number(value: &Value) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("a line or column, not {value}"))
}

/// One diagnostic of a `diagnostics` event.
fn diagnostic(value: &Value) -> Result<Diagnostic, String> {
    let at = |end: &str| -> Result<Position, String> {
        Ok(Position {
            line: position_number(&value["range"][end][0])?,
            column: position_number(&value["range"][end][1])?,
        })
    };
    Ok(Diagnostic {
        range: TextRange {
            start: at("start")?,
            end: at("end")?,
        },
        severity: match value["severity"].as_str() {
            Some("Error") => Severity::Error,
            Some("Warning") => Severity::Warning,
            Some("Information") => Severity::Information,
            Some("Hint") => Severity::Hint,
            other => return Err(format!("severity {other:?}")),
        },
        message: value["message"]
            .as_str()
            .ok_or("a diagnostic without message")?
            .to_owned(),
        source: value["source"].as_str().map(str::to_owned),
    })
}

/// An effect as a transcript's `expect` writes it.
pub fn describe(effect: &Effect) -> Value {
    match effect {
        Effect::BeginTurn { session } => json!({ "begin-turn": session }),
        Effect::EndTurn { session, changed } => json!({
            "end-turn": {
                "session": session,
                "changed": changed.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            }
        }),
        Effect::Status(None) => json!({ "status": null }),
        Effect::Status(Some(status)) => {
            let state = match status.state {
                StatusState::Idle => "idle",
                StatusState::Working => "working",
                StatusState::Blocked => "blocked",
                StatusState::Done => "done",
            };
            json!({ "status": { "state": state, "label": status.label } })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transcript_replays_against_the_pinned_version() {
        for (name, text) in TRANSCRIPTS {
            let header = replay(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(header.claude_code, PINNED_CLAUDE_CODE, "{name}");
            let date = header.recorded.len() == 10
                && header.recorded.bytes().enumerate().all(|(index, byte)| {
                    if index == 4 || index == 7 {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit()
                    }
                });
            assert!(
                header.recorded == "synthetic" || date,
                "{name}: recorded must be \"synthetic\" or YYYY-MM-DD, not {:?}",
                header.recorded
            );
        }
    }

    /// Not a check: fills a draft from `src/compat/recorder/convert.py` into a transcript.
    /// `FYX_CLAUDE_DRAFT=<draft> FYX_CLAUDE_TRANSCRIPT=<out> cargo test -p fyx_claude_code
    /// fill_draft -- --ignored`.
    #[test]
    #[ignore = "a tool for new recordings"]
    fn fill_draft() {
        let draft = std::env::var("FYX_CLAUDE_DRAFT").expect("FYX_CLAUDE_DRAFT");
        let out = std::env::var("FYX_CLAUDE_TRANSCRIPT").expect("FYX_CLAUDE_TRANSCRIPT");
        let text = std::fs::read_to_string(&draft).expect("the draft");
        let transcript = fill(&text).unwrap_or_else(|error| panic!("{draft}: {error}"));
        replay(&transcript).unwrap_or_else(|error| panic!("the filled transcript: {error}"));
        std::fs::write(&out, transcript).expect("writing the transcript");
    }

    #[test]
    fn a_changed_protocol_fails_loudly() {
        let (_, text) = TRANSCRIPTS[0];
        let broken = [
            // Claude renamed the diff tool.
            text.replacen(
                "\"name\":\"openDiff\",\"arguments\"",
                "\"name\":\"openDiff2\",\"arguments\"",
                1,
            ),
            // The tool list the extension registers no longer matches.
            text.replacen(",{\"name\":\"getDiagnostics\"}", "", 1),
            // Claude reads a different answer.
            text.replacen("FILE_SAVED", "SAVED", 1),
            // A hook event was renamed.
            text.replacen(
                "\"hook_event_name\":\"Stop\"",
                "\"hook_event_name\":\"Stopped\"",
                1,
            ),
            // Another version.
            text.replacen("\"version\":1", "\"version\":2", 1),
        ];
        for (index, transcript) in broken.iter().enumerate() {
            assert_ne!(transcript, text, "case {index} changed nothing");
            assert!(replay(transcript).is_err(), "case {index} replayed");
        }
    }
}
