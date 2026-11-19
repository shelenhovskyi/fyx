//! The compat test: protocol transcripts of a pinned Gemini CLI version, replayed against the
//! companion core. The format is `fyx-gemini-transcript` v1 (`compat/README.md`), the same
//! shape as `fyx_claude_code`'s, without hooks.

use serde_json::{Value, json};

use crate::companion::{
    self, CompanionCall, Diffs, FileRef, McpError, Notify, OpenFile, ReviewDecision, Roots,
    Selection,
};

pub const FORMAT: &str = "fyx-gemini-transcript";
pub const VERSION: u64 = 1;
pub const PINNED_GEMINI_CLI: &str = "0.69.0";

pub const TRANSCRIPTS: &[(&str, &str)] =
    &[("two-diffs.jsonl", include_str!("compat/two-diffs.jsonl"))];

#[derive(Debug)]
pub struct Header {
    pub gemini_cli: String,
    pub recorded: String,
}

pub fn replay(text: &str) -> Result<Header, String> {
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    let (_, first) = lines.next().ok_or("an empty transcript")?;
    let header: Value = serde_json::from_str(first).map_err(|error| format!("line 1: {error}"))?;
    if header["format"] != FORMAT || header["version"] != VERSION {
        return Err(format!("line 1: not {FORMAT} version {VERSION}"));
    }
    let field = |name: &str| {
        header[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("line 1: no {name:?}"))
    };
    let header_out = Header {
        gemini_cli: field("gemini_cli")?,
        recorded: field("recorded")?,
    };
    let mut replay = Replay {
        roots: PrefixRoot(field("root")?),
        diffs: Diffs::default(),
        expected_out: Vec::new(),
    };
    for (index, line) in lines {
        let event: Value =
            serde_json::from_str(line).map_err(|error| format!("line {}: {error}", index + 1))?;
        replay
            .step(&event)
            .map_err(|why| format!("line {}: {why}", index + 1))?;
    }
    if let Some((connection, left)) = replay.expected_out.first() {
        return Err(format!(
            "the extension never sent {left} on connection {connection}"
        ));
    }
    if replay.diffs.len() > 0 {
        return Err(format!("{} diffs left open", replay.diffs.len()));
    }
    Ok(header_out)
}

struct PrefixRoot(String);

impl Roots for PrefixRoot {
    fn to_ref(&self, absolute: &str) -> Option<FileRef> {
        let relative = absolute.strip_prefix(&self.0)?.strip_prefix('/')?;
        (!relative.is_empty()
            && relative
                .split('/')
                .all(|part| !matches!(part, "" | "." | "..")))
        .then(|| FileRef {
            worktree: Some(1),
            path: relative.to_owned(),
        })
    }

    fn to_absolute(&self, file: &FileRef) -> Option<String> {
        (file.worktree == Some(1)).then(|| format!("{}/{}", self.0, file.path))
    }
}

struct Replay {
    roots: PrefixRoot,
    diffs: Diffs,
    /// (connection, message) the extension produced, in order.
    expected_out: Vec<(u64, Value)>,
}

fn connection(event: &Value) -> u64 {
    event["conn"].as_u64().unwrap_or(1)
}

impl Replay {
    fn step(&mut self, event: &Value) -> Result<(), String> {
        match event["t"].as_str() {
            Some("mcp") => match event["dir"].as_str() {
                Some("in") => self.from_cli(connection(event), &event["msg"]),
                Some("out") => self.to_cli(connection(event), &event["msg"]),
                other => Err(format!("mcp dir {other:?}")),
            },
            Some("review") => self.review(event),
            Some("editor") => self.editor(event),
            other => Err(format!("unknown event type {other:?}")),
        }
    }

    fn respond(&mut self, connection: u64, id: &Value, outcome: Result<String, McpError>) {
        let message = match outcome {
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
        };
        self.expected_out.push((connection, message));
    }

    fn notify(&mut self, notify: Notify) {
        self.expected_out.push((
            notify.connection,
            json!({
                "jsonrpc": "2.0",
                "method": notify.method,
                "params": serde_json::from_str::<Value>(&notify.params).unwrap_or(Value::Null),
            }),
        ));
    }

    /// The host answers `initialize`, `tools/list` and notifications; the extension answers
    /// `tools/call`. A replayed `openDiff` gets the call's id as its proposal id.
    fn from_cli(&mut self, connection: u64, message: &Value) -> Result<(), String> {
        if message["method"] != "tools/call" {
            return Ok(());
        }
        let id = &message["id"];
        let tool = message["params"]["name"]
            .as_str()
            .ok_or("tools/call without a name")?;
        let arguments = message["params"]["arguments"].to_string();
        match CompanionCall::parse(tool, &arguments, &self.roots) {
            Ok(CompanionCall::OpenDiff { file, new_content }) => {
                let proposal = id.as_u64().ok_or("openDiff needs a numeric id here")?;
                self.diffs.opened(file, proposal, connection, new_content);
                self.respond(connection, id, Ok(companion::OPENED.to_owned()));
            }
            Ok(CompanionCall::CloseDiff { file, suppress }) => {
                let (result, notify) = self.diffs.close(&file, suppress, None, &self.roots);
                self.respond(connection, id, Ok(result));
                if let Some(notify) = notify {
                    self.notify(notify);
                }
            }
            Err(error) => self.respond(connection, id, Err(error)),
        }
        Ok(())
    }

    fn to_cli(&mut self, connection: u64, message: &Value) -> Result<(), String> {
        if let Some(tools) = message["result"]["tools"].as_array() {
            let names: Vec<&str> = tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect();
            let ours: Vec<&str> = companion::tools().iter().map(|tool| tool.name).collect();
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
        let (ours_connection, ours) = self.expected_out.remove(0);
        if ours_connection != connection || ours != *message {
            return Err(format!(
                "the extension sent {ours} on connection {ours_connection}, the transcript has \
                 {message} on {connection}"
            ));
        }
        Ok(())
    }

    fn review(&mut self, event: &Value) -> Result<(), String> {
        let proposal = event["call"].as_u64().ok_or("review without call")?;
        let decision = match event["decision"].as_str() {
            Some("accepted") => ReviewDecision::Accepted,
            Some("partial") => ReviewDecision::Partial,
            Some("rejected") => ReviewDecision::Rejected,
            other => return Err(format!("review decision {other:?}")),
        };
        let final_content = event["final_content"].as_str().unwrap_or_default();
        let notify = self
            .diffs
            .decided(proposal, decision, final_content, &self.roots)
            .ok_or_else(|| format!("no open diff for proposal {proposal}"))?;
        self.notify(notify);
        Ok(())
    }

    /// `editor`: `files` as `[path, focused_at]`, newest last or in any order; `selection` of
    /// the active file as `{line, column, text}`, zero-based; `trusted`. The extension sends
    /// `ide/contextUpdate` to `conn`.
    fn editor(&mut self, event: &Value) -> Result<(), String> {
        let files = event["files"]
            .as_array()
            .ok_or("editor without files")?
            .iter()
            .map(|entry| {
                Some(OpenFile {
                    file: FileRef {
                        worktree: Some(1),
                        path: entry[0].as_str()?.to_owned(),
                    },
                    focused_at: entry[1].as_u64()?,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or("files must be [path, focused_at]")?;
        let selection = event.get("selection").map(|selection| Selection {
            line: selection["line"].as_u64().unwrap_or(0) as u32,
            column: selection["column"].as_u64().unwrap_or(0) as u32,
            text: selection["text"].as_str().map(str::to_owned),
        });
        let params = companion::context_update(
            &files,
            selection.as_ref(),
            event["trusted"].as_bool(),
            &self.roots,
        );
        self.expected_out.push((
            connection(event),
            json!({
                "jsonrpc": "2.0",
                "method": "ide/contextUpdate",
                "params": serde_json::from_str::<Value>(&params).unwrap_or(Value::Null),
            }),
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transcript_replays_against_the_pinned_version() {
        for (name, text) in TRANSCRIPTS {
            let header = replay(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(header.gemini_cli, PINNED_GEMINI_CLI, "{name}");
            assert!(
                header.recorded == "synthetic"
                    || (header.recorded.len() == 10 && header.recorded.as_bytes()[4] == b'-'),
                "{name}: recorded {:?}",
                header.recorded
            );
        }
    }

    #[test]
    fn a_changed_protocol_fails_loudly() {
        let (_, text) = TRANSCRIPTS[0];
        let broken = [
            text.replacen("\"ide/diffAccepted\"", "\"ide/diffSaved\"", 1),
            text.replacen("\"newContent\"", "\"content\"", 1),
            text.replacen("\"character\":5", "\"character\":4", 1),
            text.replacen(
                "\"suppressNotification\":true",
                "\"suppressNotification\":false",
                1,
            ),
            text.replacen("\"version\":1", "\"version\":2", 1),
        ];
        for (index, transcript) in broken.iter().enumerate() {
            assert_ne!(transcript, text, "case {index} changed nothing");
            assert!(replay(transcript).is_err(), "case {index} replayed");
        }
    }
}
