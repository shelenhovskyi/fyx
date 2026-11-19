//! Gemini CLI's IDE companion interface, the Fyx side (design "The first extensions",
//! `fyx_gemini_cli`). The spec is documented (`docs/ide-integration/ide-companion-spec.md` in
//! google-gemini/gemini-cli). Where the CLI's source differs from the spec, the source wins and
//! the difference is noted.
//!
//! - **Transport** (the host's): MCP over streamable HTTP at `http://127.0.0.1:<port>/mcp`,
//!   `Authorization: Bearer <authToken>`, found through `gemini-ide-server-<pid>-<port>.json`.
//! - **Tools** ([`tools`]): `openDiff` answers at once ([`OPENED`]); the user's decision follows
//!   as `ide/diffAccepted` or `ide/diffRejected` ([`Diffs`]). `closeDiff` returns the diff's text
//!   as JSON `{"content": …}`, which the spec does not say but the CLI parses.
//! - **Context** ([`context_update`]): `ide/contextUpdate` with the open files, newest first, and
//!   the active file's 1-based cursor and selection.
//!
//! Unlike Claude's blocking `openDiff`, this one is "send, then get a notification": the review
//! proposal's outcome becomes a notification to the connection that opened it.
//!
//! [`Server`] is the one dispatcher, over a [`CompanionHost`]: the SDK in the component
//! (`wired.rs`), the transcripts in the compat test.

use std::collections::BTreeMap;

use serde_json::{Value, json};

pub const LISTENER: &str = "ide";

pub const INVALID_PARAMS: i32 = -32602;
pub const UNKNOWN_TOOL: i32 = -32601;

/// The CLI keeps the 10 newest files and 16 KiB of selection; the companion sends no more.
pub const MAX_OPEN_FILES: usize = 10;
pub const MAX_SELECTED_TEXT_BYTES: usize = 16 * 1024;

/// `listeners.tool-def`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: String,
}

/// `listeners.mcp-error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpError {
    pub code: i32,
    pub message: String,
}

/// `types.file-ref`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileRef {
    pub worktree: Option<u64>,
    pub path: String,
}

/// Absolute paths, as Gemini names files, against Fyx's worktree-relative [`FileRef`]s:
/// `context.to-file-ref` and `context.absolute-path` in the component, a prefix in the tests.
pub trait Roots {
    fn to_ref(&self, absolute: &str) -> Option<FileRef>;
    fn to_absolute(&self, file: &FileRef) -> Option<String>;
}

/// The tools the companion must register: both, or the CLI does not use IDE diffs.
pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "openDiff",
            description: "Open a modifiable diff view of a proposed change to a file.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filePath": { "type": "string" },
                    "newContent": { "type": "string" },
                },
                "required": ["filePath", "newContent"],
            })
            .to_string(),
        },
        ToolDef {
            name: "closeDiff",
            description: "Close the diff view of a file and return its text.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filePath": { "type": "string" },
                    "suppressNotification": { "type": "boolean" },
                },
                "required": ["filePath"],
            })
            .to_string(),
        },
    ]
}

/// A `tools/call` from the CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompanionCall {
    OpenDiff {
        file: FileRef,
        new_content: String,
    },
    /// `suppress`: the user answered in the CLI, which resolves the diff itself, so no
    /// notification may follow.
    CloseDiff {
        file: FileRef,
        suppress: bool,
    },
}

impl CompanionCall {
    pub fn parse(tool: &str, arguments: &str, roots: &dyn Roots) -> Result<Self, McpError> {
        let invalid = |message: String| McpError {
            code: INVALID_PARAMS,
            message,
        };
        let arguments: Value = serde_json::from_str(arguments)
            .map_err(|error| invalid(format!("arguments are not JSON: {error}")))?;
        let text = |name: &str| arguments.get(name).and_then(Value::as_str);
        let file = || {
            let path =
                text("filePath").ok_or_else(|| invalid(format!("{tool}: filePath is required")))?;
            roots
                .to_ref(path)
                .ok_or_else(|| invalid(format!("{tool}: {path:?} is not in an open worktree")))
        };
        match tool {
            "openDiff" => Ok(Self::OpenDiff {
                file: file()?,
                new_content: text("newContent")
                    .ok_or_else(|| invalid("openDiff: newContent is required".into()))?
                    .to_owned(),
            }),
            "closeDiff" => Ok(Self::CloseDiff {
                file: file()?,
                suppress: arguments
                    .get("suppressNotification")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            _ => Err(McpError {
                code: UNKNOWN_TOOL,
                message: format!("unknown tool {tool:?}"),
            }),
        }
    }
}

/// `openDiff`'s answer once the review is open: empty content.
pub const OPENED: &str = r#"{"content":[]}"#;

/// A tool error the CLI shows (`isError` with a text block).
pub fn error_result(message: &str) -> String {
    json!({ "isError": true, "content": [{ "type": "text", "text": message }] }).to_string()
}

/// `review.decision`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Accepted,
    Partial,
    Rejected,
}

/// An MCP notification for one connection: `listeners.notify(listener, Some(connection), …)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notify {
    pub connection: u64,
    pub method: &'static str,
    /// JSON, as text.
    pub params: String,
}

/// One open diff: the review proposal and the connection to notify. The CLI keys diffs by file
/// path, so a second `openDiff` for a file replaces the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenDiff {
    pub proposal: u64,
    pub connection: u64,
    pub new_content: String,
}

#[derive(Debug, Default)]
pub struct Diffs {
    open: BTreeMap<FileRef, OpenDiff>,
}

impl Diffs {
    /// After `review.propose` returned `proposal`. Returns the replaced diff's proposal, which
    /// the extension withdraws.
    pub fn opened(
        &mut self,
        file: FileRef,
        proposal: u64,
        connection: u64,
        new_content: String,
    ) -> Option<u64> {
        self.open
            .insert(
                file,
                OpenDiff {
                    proposal,
                    connection,
                    new_content,
                },
            )
            .map(|old| old.proposal)
    }

    /// `io-event::review-decided`: the notification for the CLI, or `None` for a proposal that
    /// is no longer open (closed, or replaced).
    pub fn decided(
        &mut self,
        proposal: u64,
        decision: ReviewDecision,
        final_content: &str,
        roots: &dyn Roots,
    ) -> Option<Notify> {
        let file = self
            .open
            .iter()
            .find(|(_, diff)| diff.proposal == proposal)
            .map(|(file, _)| file.clone())?;
        let diff = self.open.remove(&file)?;
        let path = roots.to_absolute(&file)?;
        Some(match decision {
            ReviewDecision::Accepted | ReviewDecision::Partial => Notify {
                connection: diff.connection,
                method: "ide/diffAccepted",
                params: json!({ "filePath": path, "content": final_content }).to_string(),
            },
            ReviewDecision::Rejected => Notify {
                connection: diff.connection,
                method: "ide/diffRejected",
                params: json!({ "filePath": path }).to_string(),
            },
        })
    }

    /// `closeDiff`: the result and, unless suppressed, `ide/diffRejected`. `current` is the
    /// diff's text with the user's edits (`review.withdraw`'s answer); without it the
    /// proposal's own text stands in.
    pub fn close(
        &mut self,
        file: &FileRef,
        suppress: bool,
        current: Option<&str>,
        roots: &dyn Roots,
    ) -> (String, Option<Notify>) {
        let Some(diff) = self.open.remove(file) else {
            return (text_result(&json!({ "content": null }).to_string()), None);
        };
        let content = current.unwrap_or(&diff.new_content);
        let result = text_result(&json!({ "content": content }).to_string());
        let notify = (!suppress)
            .then(|| roots.to_absolute(file))
            .flatten()
            .map(|path| Notify {
                connection: diff.connection,
                method: "ide/diffRejected",
                params: json!({ "filePath": path }).to_string(),
            });
        (result, notify)
    }

    pub fn is_open(&self, file: &FileRef) -> bool {
        self.open.contains_key(file)
    }

    /// The review proposal of `file`'s open diff.
    pub fn proposal(&self, file: &FileRef) -> Option<u64> {
        self.open.get(file).map(|diff| diff.proposal)
    }

    /// Forgets `file`'s diff without a notification (the CLI replaced it); its proposal.
    pub fn forget(&mut self, file: &FileRef) -> Option<u64> {
        self.open.remove(file).map(|diff| diff.proposal)
    }

    pub fn len(&self) -> usize {
        self.open.len()
    }
}

fn text_result(text: &str) -> String {
    json!({ "content": [{ "type": "text", "text": text }] }).to_string()
}

/// What the companion needs from the host.
pub trait CompanionHost {
    fn roots(&self) -> &dyn Roots;
    /// `listeners.respond`.
    fn respond(&mut self, call: u64, outcome: Result<String, McpError>);
    /// `listeners.notify` to one connection.
    fn notify(&mut self, notify: Notify);
    /// `listeners.notify` to every connection of [`LISTENER`].
    fn broadcast(&mut self, method: &'static str, params: &str);
    /// `review.propose` of `new_content` for `file`: the proposal's id, or why it was refused.
    fn propose(&mut self, file: &FileRef, new_content: &str) -> Result<u64, String>;
    /// `review.withdraw`: the proposal's current text, with the user's edits, or `None` when it
    /// already ended.
    fn withdraw(&mut self, proposal: u64) -> Option<String>;
}

/// The companion's state: the open diffs and the last context sent.
#[derive(Debug, Default)]
pub struct Server {
    diffs: Diffs,
    context: Option<String>,
}

impl Server {
    /// The number of diffs waiting for review.
    pub fn open_diffs(&self) -> usize {
        self.diffs.len()
    }

    /// A `tools/call` on `connection`.
    pub fn call(
        &mut self,
        host: &mut dyn CompanionHost,
        connection: u64,
        call: u64,
        tool: &str,
        arguments: &str,
    ) {
        match CompanionCall::parse(tool, arguments, host.roots()) {
            Ok(CompanionCall::OpenDiff { file, new_content }) => {
                // The CLI keys diffs by path: a second one for the file replaces the first,
                // which ends without a notification.
                if let Some(old) = self.diffs.forget(&file) {
                    host.withdraw(old);
                }
                match host.propose(&file, &new_content) {
                    Ok(proposal) => {
                        self.diffs.opened(file, proposal, connection, new_content);
                        host.respond(call, Ok(OPENED.to_owned()));
                    }
                    Err(why) => host.respond(call, Ok(error_result(&format!("openDiff: {why}")))),
                }
            }
            Ok(CompanionCall::CloseDiff { file, suppress }) => {
                let current = self
                    .diffs
                    .proposal(&file)
                    .and_then(|proposal| host.withdraw(proposal));
                let (result, notify) =
                    self.diffs
                        .close(&file, suppress, current.as_deref(), host.roots());
                host.respond(call, Ok(result));
                if let Some(notify) = notify {
                    host.notify(notify);
                }
            }
            Err(error) => host.respond(call, Err(error)),
        }
    }

    /// `io-event::review-decided`: `None` is a review that ended undecided (the file went
    /// away), which the CLI hears as a rejection.
    pub fn review_decided(
        &mut self,
        host: &mut dyn CompanionHost,
        proposal: u64,
        outcome: Option<(ReviewDecision, &str)>,
    ) {
        let (decision, final_content) = outcome.unwrap_or((ReviewDecision::Rejected, ""));
        if let Some(notify) = self
            .diffs
            .decided(proposal, decision, final_content, host.roots())
        {
            host.notify(notify);
        }
    }

    /// `io-event::mcp-initialized`: the last context, to the connection that just finished
    /// the handshake, unless no file is open (a new connection knows nothing either way).
    pub fn initialized(&mut self, host: &mut dyn CompanionHost, connection: u64) {
        let Some(params) = &self.context else {
            return;
        };
        let has_files = serde_json::from_str::<Value>(params).is_ok_and(|params| {
            params["workspaceState"]["openFiles"]
                .as_array()
                .is_some_and(|files| !files.is_empty())
        });
        if has_files {
            host.notify(Notify {
                connection,
                method: "ide/contextUpdate",
                params: params.clone(),
            });
        }
    }

    /// `io-event::context-changed`, as [`context_update`]'s params: sent to every connection
    /// unless it equals the last one sent.
    pub fn context_changed(&mut self, host: &mut dyn CompanionHost, params: String) {
        if self.context.as_ref() != Some(&params) {
            host.broadcast("ide/contextUpdate", &params);
            self.context = Some(params);
        }
    }
}

/// One open file for [`context_update`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenFile {
    pub file: FileRef,
    /// When the file had focus last, newer larger. The CLI orders files by it and uses no
    /// other meaning; the editor context gives an order, not times (`wired.rs`).
    pub focused_at: u64,
}

/// The active file's caret and selection, zero-based as Fyx's `types.position`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub line: u32,
    pub column: u32,
    pub text: Option<String>,
}

/// `ide/contextUpdate`'s params: the newest [`MAX_OPEN_FILES`] files that are in a worktree,
/// newest first; the newest is active and carries the 1-based cursor and the selection, cut to
/// [`MAX_SELECTED_TEXT_BYTES`]. `trusted` is the active worktree's trust; `None` leaves
/// `isTrusted` out.
pub fn context_update(
    files: &[OpenFile],
    selection: Option<&Selection>,
    trusted: Option<bool>,
    roots: &dyn Roots,
) -> String {
    let mut files: Vec<(&OpenFile, String)> = files
        .iter()
        .filter_map(|open| Some((open, roots.to_absolute(&open.file)?)))
        .collect();
    files.sort_by_key(|(open, _)| std::cmp::Reverse(open.focused_at));
    files.truncate(MAX_OPEN_FILES);
    let open_files: Vec<Value> = files
        .iter()
        .enumerate()
        .map(|(index, (open, path))| {
            let mut entry = json!({ "path": path, "timestamp": open.focused_at });
            if index == 0 {
                entry["isActive"] = true.into();
                if let Some(selection) = selection {
                    entry["cursor"] = json!({
                        "line": selection.line + 1,
                        "character": selection.column + 1,
                    });
                    if let Some(text) = selection.text.as_deref().filter(|text| !text.is_empty()) {
                        entry["selectedText"] = cut(text, MAX_SELECTED_TEXT_BYTES).into();
                    }
                }
            }
            entry
        })
        .collect();
    let mut state = json!({ "openFiles": open_files });
    if let Some(trusted) = trusted {
        state["isTrusted"] = trusted.into();
    }
    json!({ "workspaceState": state }).to_string()
}

fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The discovery file the CLI reads: `port` a number, `workspacePath` the roots joined with the
/// OS path delimiter, and `ideInfo` an object. The manifest's template says the same
/// (`${port}`, `${worktrees.joined}`); this is the reference the tests check it against.
pub fn discovery_content(port: u16, roots: &[&str], token: &str, windows: bool) -> String {
    let delimiter = if windows { ";" } else { ":" };
    json!({
        "port": port,
        "workspacePath": roots.join(delimiter),
        "authToken": token,
        "ideInfo": { "name": "fyx", "displayName": "Fyx" },
    })
    .to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct OneRoot(pub &'static str);

    impl Roots for OneRoot {
        fn to_ref(&self, absolute: &str) -> Option<FileRef> {
            let relative = absolute.strip_prefix(self.0)?.strip_prefix('/')?;
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

    const ROOT: OneRoot = OneRoot("/work/app");

    fn file(path: &str) -> FileRef {
        FileRef {
            worktree: Some(1),
            path: path.into(),
        }
    }

    #[test]
    fn calls_parse_and_refuse() {
        assert_eq!(
            CompanionCall::parse(
                "openDiff",
                r#"{"filePath":"/work/app/a.py","newContent":"x\n"}"#,
                &ROOT
            ),
            Ok(CompanionCall::OpenDiff {
                file: file("a.py"),
                new_content: "x\n".into()
            })
        );
        assert_eq!(
            CompanionCall::parse(
                "closeDiff",
                r#"{"filePath":"/work/app/a.py","suppressNotification":true}"#,
                &ROOT
            ),
            Ok(CompanionCall::CloseDiff {
                file: file("a.py"),
                suppress: true
            })
        );
        for (tool, arguments, code) in [
            (
                "openDiff",
                r#"{"filePath":"/etc/hosts","newContent":""}"#,
                INVALID_PARAMS,
            ),
            (
                "openDiff",
                r#"{"filePath":"/work/app/a.py"}"#,
                INVALID_PARAMS,
            ),
            ("closeDiff", r#"{}"#, INVALID_PARAMS),
            ("openFile", r#"{"filePath":"/work/app/a.py"}"#, UNKNOWN_TOOL),
        ] {
            let error = CompanionCall::parse(tool, arguments, &ROOT).unwrap_err();
            assert_eq!(error.code, code, "{tool} {arguments}");
        }
    }

    #[test]
    fn a_decision_notifies_the_connection_that_opened_the_diff() {
        let mut diffs = Diffs::default();
        assert_eq!(diffs.opened(file("a.py"), 7, 3, "new\n".into()), None);
        let accepted = diffs
            .decided(7, ReviewDecision::Partial, "kept\n", &ROOT)
            .unwrap();
        assert_eq!(accepted.connection, 3);
        assert_eq!(accepted.method, "ide/diffAccepted");
        let params: Value = serde_json::from_str(&accepted.params).unwrap();
        assert_eq!(params["filePath"], "/work/app/a.py");
        assert_eq!(params["content"], "kept\n");
        assert!(
            diffs
                .decided(7, ReviewDecision::Rejected, "", &ROOT)
                .is_none()
        );

        // A second openDiff for the file replaces the first; the first's outcome is dropped.
        diffs.opened(file("a.py"), 8, 3, "one\n".into());
        assert_eq!(diffs.opened(file("a.py"), 9, 4, "two\n".into()), Some(8));
        assert!(
            diffs
                .decided(8, ReviewDecision::Accepted, "one\n", &ROOT)
                .is_none()
        );
        let rejected = diffs
            .decided(9, ReviewDecision::Rejected, "", &ROOT)
            .unwrap();
        assert_eq!(
            (rejected.connection, rejected.method),
            (4, "ide/diffRejected")
        );
        assert_eq!(diffs.len(), 0);
    }

    #[test]
    fn close_diff_returns_the_text_as_the_cli_parses_it() {
        let mut diffs = Diffs::default();
        diffs.opened(file("a.py"), 1, 2, "proposed\n".into());
        let (result, notify) = diffs.close(&file("a.py"), true, Some("edited\n"), &ROOT);
        let result: Value = serde_json::from_str(&result).unwrap();
        let inner: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(inner["content"], "edited\n");
        assert!(notify.is_none(), "suppressed");

        diffs.opened(file("a.py"), 3, 2, "proposed\n".into());
        let (_, notify) = diffs.close(&file("a.py"), false, None, &ROOT);
        assert_eq!(notify.unwrap().method, "ide/diffRejected");
        let (result, _) = diffs.close(&file("a.py"), false, None, &ROOT);
        assert!(result.contains(r#"\"content\":null"#), "{result}");
    }

    #[test]
    fn context_is_newest_first_and_one_based() {
        let mut files: Vec<OpenFile> = (0..12)
            .map(|index| OpenFile {
                file: file(&format!("f{index}.py")),
                focused_at: 1_000 + index,
            })
            .collect();
        files.push(OpenFile {
            file: FileRef {
                worktree: Some(2),
                path: "elsewhere.py".into(),
            },
            focused_at: 9_999,
        });
        let selection = Selection {
            line: 0,
            column: 4,
            text: Some("é".repeat(10_000)),
        };
        let params: Value =
            serde_json::from_str(&context_update(&files, Some(&selection), Some(true), &ROOT))
                .unwrap();
        let open = params["workspaceState"]["openFiles"].as_array().unwrap();
        assert_eq!(open.len(), MAX_OPEN_FILES);
        assert_eq!(open[0]["path"], "/work/app/f11.py");
        assert_eq!(open[0]["isActive"], true);
        assert_eq!(open[0]["cursor"], json!({ "line": 1, "character": 5 }));
        assert_eq!(
            open[0]["selectedText"].as_str().unwrap().len(),
            MAX_SELECTED_TEXT_BYTES
        );
        assert!(open[1].get("cursor").is_none() && open[1].get("isActive").is_none());
        assert_eq!(params["workspaceState"]["isTrusted"], true);
        let untold: Value =
            serde_json::from_str(&context_update(&files, None, None, &ROOT)).unwrap();
        assert!(untold["workspaceState"].get("isTrusted").is_none());
    }

    #[test]
    fn the_discovery_file_has_the_cli_shape() {
        let posix: Value =
            serde_json::from_str(&discovery_content(4242, &["/a", "/b"], "t", false)).unwrap();
        assert_eq!(posix["port"], 4242);
        assert_eq!(posix["workspacePath"], "/a:/b");
        assert_eq!(posix["ideInfo"]["displayName"], "Fyx");
        let windows: Value =
            serde_json::from_str(&discovery_content(1, &["C:\\a", "D:\\b"], "t", true)).unwrap();
        assert_eq!(windows["workspacePath"], "C:\\a;D:\\b");
    }
}
