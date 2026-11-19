//! Claude Code's IDE protocol, the Fyx side (design "The first extensions", item 3). It is
//! reverse-engineered [RE]: the shapes here were read from Claude Code 2.1.326, which the compat
//! test pins (`compat/README.md`).
//!
//! Claude finds the IDE through `<config dir>/ide/<port>.lock`, connects to
//! `ws://127.0.0.1:<port>` with the subprotocol `mcp` and the lock's `authToken` in
//! `X-Claude-Code-Ide-Authorization`, and speaks MCP. The host owns all of that (the
//! `mcp-websocket` listener `ide`). This module owns what the extension does with it:
//! - the tools the IDE offers ([`tools`]), which Claude calls ([`IdeCall::parse`]);
//! - the answers, above all `openDiff`'s ([`DiffAnswer`]);
//! - the notifications the IDE sends ([`selection_changed`], [`at_mentioned`]).
//!
//! Paths are absolute on Claude's side and worktree-relative in Fyx; [`Roots`] maps them.
//! On Windows Claude sends `C:\a\b` paths and two URI forms (`file:///C:/a/b` and
//! `file://C:\a\b`); [`normalize_path`] and [`uri_to_path`] make one form of them.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// The `ide` listener in `extension.toml`.
pub const LISTENER: &str = "ide";

/// JSON-RPC's "invalid params" and "method not found", as MCP uses them.
pub const INVALID_PARAMS: i32 = -32602;
pub const UNKNOWN_TOOL: i32 = -32601;
/// JSON-RPC's "internal error": the host refused what the call needs.
pub const INTERNAL_ERROR: i32 = -32603;

/// `listeners.tool-def`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// A JSON schema, as text.
    pub input_schema: String,
}

/// `listeners.mcp-error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpError {
    pub code: i32,
    pub message: String,
}

impl McpError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
        }
    }
}

/// `types.file-ref`: a file in a worktree, by a `/`-separated relative path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileRef {
    pub worktree: Option<u64>,
    pub path: String,
}

/// `types.position`: zero-based line and column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// `types.text-range`, half-open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextRange {
    pub start: Position,
    pub end: Position,
}

/// Absolute paths, as Claude names files, against Fyx's worktree-relative [`FileRef`]s.
///
/// Built for 0.4 it is `context.to-file-ref` and `context.absolute-path` (`wired`); the compat
/// replay and the tests fill it with a fixed root.
///
/// `to_ref` gets paths in [`normalize_path`]'s form; `to_absolute` gives the OS's own form, as
/// Claude sends paths (with `\` on Windows).
pub trait Roots {
    fn to_ref(&self, absolute: &str) -> Option<FileRef>;
    fn to_absolute(&self, file: &FileRef) -> Option<String>;
}

/// One form for an absolute path from Claude: `\` becomes `/` and a drive letter is upper case
/// (`c:\a\b` → `C:/a/b`). POSIX paths are kept as they are.
pub fn normalize_path(path: &str) -> String {
    let mut path = path.replace('\\', "/");
    if let [drive, b':', ..] = path.as_bytes()
        && drive.is_ascii_alphabetic()
    {
        let upper = (*drive as char).to_ascii_uppercase().to_string();
        path.replace_range(..1, &upper);
    }
    path
}

/// The path in a `file:` URI, in [`normalize_path`]'s form. Both forms Claude sends on Windows
/// work: `file:///C:/a%20b` (percent-encoded) and `file://C:\a b`. `None` for another scheme.
pub fn uri_to_path(uri: &str) -> Option<String> {
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("FILE://"))?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode(rest)?;
    // `/C:/a` → `C:/a`; `/work/a` stays.
    let path = match decoded.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &decoded[1..],
        _ => &decoded[..],
    };
    Some(normalize_path(path))
}

/// A `file:` URI for an absolute path, as VS Code writes them: `C:\a b` → `file:///C:/a%20b`,
/// `/work/a` → `file:///work/a`.
pub fn path_to_uri(path: &str) -> String {
    let path = normalize_path(path);
    let mut uri = String::from(if path.starts_with('/') {
        "file://"
    } else {
        "file:///"
    });
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'.' | b'_' | b'~' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The tools Fyx's IDE server offers, which Claude 2.1.326 calls: `openDiff` (blocking),
/// `close_tab`, `closeAllDiffTabs` and `getDiagnostics` from the CLI itself, `openFile` from
/// at-mentions and links.
pub fn tools() -> Vec<ToolDef> {
    let schema = |properties: Value, required: &[&str]| {
        json!({ "type": "object", "properties": properties, "required": required }).to_string()
    };
    let text = json!({ "type": "string" });
    let flag = json!({ "type": "boolean" });
    vec![
        ToolDef {
            name: "openFile",
            description: "Open a file in the user's editor in Fyx. Give its absolute path; to \
                select text, give the text where the selection starts and where it ends.",
            input_schema: schema(
                json!({
                    "filePath": text, "preview": flag, "startText": text, "endText": text,
                    "selectToEndOfLine": flag, "makeFrontmost": flag,
                }),
                &["filePath"],
            ),
        },
        ToolDef {
            name: "openDiff",
            description: "Show the user a proposed change to a file in Fyx's review and wait \
                for their decision.",
            input_schema: schema(
                json!({
                    "old_file_path": text, "new_file_path": text,
                    "new_file_contents": text, "tab_name": text,
                }),
                &[
                    "old_file_path",
                    "new_file_path",
                    "new_file_contents",
                    "tab_name",
                ],
            ),
        },
        ToolDef {
            name: "close_tab",
            description: "Close a proposed change that openDiff showed, by its tab name.",
            input_schema: schema(json!({ "tab_name": text }), &["tab_name"]),
        },
        ToolDef {
            name: "closeAllDiffTabs",
            description: "Close every proposed change that openDiff showed.",
            input_schema: schema(json!({}), &[]),
        },
        ToolDef {
            name: "getDiagnostics",
            description: "Get the errors, warnings and hints that the user's editor in Fyx \
                shows. Pass a file's file:// URI for that file, or nothing for every open file. \
                Use it to check code before and after you change it.",
            input_schema: schema(json!({ "uri": text }), &[]),
        },
    ]
}

/// A `tools/call` from Claude.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdeCall {
    OpenFile {
        file: FileRef,
        /// Select from the first occurrence of `start_text` to the end of `end_text`.
        start_text: Option<String>,
        end_text: Option<String>,
        focus: bool,
    },
    /// Answered with [`DiffAnswer`] once the user decides; the call stays open until then.
    OpenDiff {
        file: FileRef,
        new_contents: String,
        tab: String,
    },
    CloseTab {
        tab: String,
    },
    CloseAllDiffTabs,
    GetDiagnostics {
        /// `None`: every open file.
        file: Option<FileRef>,
    },
}

impl IdeCall {
    /// `arguments` is the call's JSON text; paths outside every worktree are refused.
    pub fn parse(tool: &str, arguments: &str, roots: &dyn Roots) -> Result<Self, McpError> {
        let arguments: Value = serde_json::from_str(arguments)
            .map_err(|error| McpError::invalid(format!("arguments are not JSON: {error}")))?;
        let arguments = arguments
            .as_object()
            .ok_or_else(|| McpError::invalid("arguments must be an object"))?;
        let text = |name: &str| arguments.get(name).and_then(Value::as_str);
        let required = |name: &str| {
            text(name).ok_or_else(|| McpError::invalid(format!("{tool}: {name} is required")))
        };
        let file = |path: &str| {
            roots.to_ref(&normalize_path(path)).ok_or_else(|| {
                McpError::invalid(format!("{tool}: {path:?} is not in an open worktree"))
            })
        };
        match tool {
            "openFile" => Ok(Self::OpenFile {
                file: file(required("filePath")?)?,
                start_text: text("startText").map(str::to_owned),
                end_text: text("endText").map(str::to_owned),
                focus: arguments
                    .get("makeFrontmost")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            }),
            "openDiff" => {
                let old = required("old_file_path")?;
                let new = required("new_file_path")?;
                // Claude sends the same path twice; a rename is not a diff Fyx can review.
                if old != new {
                    return Err(McpError::invalid(
                        "openDiff: old_file_path and new_file_path differ",
                    ));
                }
                Ok(Self::OpenDiff {
                    file: file(new)?,
                    new_contents: required("new_file_contents")?.to_owned(),
                    tab: required("tab_name")?.to_owned(),
                })
            }
            "close_tab" => Ok(Self::CloseTab {
                tab: required("tab_name")?.to_owned(),
            }),
            "closeAllDiffTabs" => Ok(Self::CloseAllDiffTabs),
            "getDiagnostics" => {
                let file = match text("uri") {
                    Some(uri) => Some(file(&uri_to_path(uri).ok_or_else(|| {
                        McpError::invalid(format!("getDiagnostics: {uri:?} is not a file URI"))
                    })?)?),
                    None => None,
                };
                Ok(Self::GetDiagnostics { file })
            }
            _ => Err(McpError {
                code: UNKNOWN_TOOL,
                message: format!("unknown tool {tool:?}"),
            }),
        }
    }
}

/// How an `openDiff` ended, as Claude 2.1.326 reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffAnswer {
    /// Accepted, fully or in part: Claude writes this text. The review proposal therefore uses
    /// `apply = false`, so the file has one writer.
    Saved(String),
    Rejected,
    /// Closed without a decision; Claude keeps its own proposal and asks in the terminal.
    TabClosed,
}

impl DiffAnswer {
    /// From `review.review-outcome`: `accepted` and `partial` carry the final text.
    pub fn from_review(decision: ReviewDecision, final_content: String) -> Self {
        match decision {
            ReviewDecision::Accepted | ReviewDecision::Partial => Self::Saved(final_content),
            ReviewDecision::Rejected => Self::Rejected,
        }
    }

    /// The `tools/call` result, as JSON text.
    pub fn result(&self) -> String {
        let content = match self {
            Self::Saved(text) => json!([
                { "type": "text", "text": "FILE_SAVED" },
                { "type": "text", "text": text },
            ]),
            Self::Rejected => json!([{ "type": "text", "text": "DIFF_REJECTED" }]),
            Self::TabClosed => json!([{ "type": "text", "text": "TAB_CLOSED" }]),
        };
        json!({ "content": content }).to_string()
    }
}

/// `review.decision`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Accepted,
    Partial,
    Rejected,
}

/// A plain-text `tools/call` result, for `close_tab` and the like.
pub fn text_result(text: &str) -> String {
    json!({ "content": [{ "type": "text", "text": text }] }).to_string()
}

/// One diagnostic, for [`diagnostics_result`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: TextRange,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// `getDiagnostics`' result: a text item holding a JSON array of `{uri, diagnostics}` with
/// LSP-shaped ranges, as VS Code's extension answers.
///
/// Stable: files by path, each file's diagnostics by range, then severity and message, with
/// duplicates left out. Claude compares the answers before and after an edit, so the same
/// diagnostics must give the same text.
pub fn diagnostics_result(files: &[(String, Vec<Diagnostic>)]) -> String {
    let mut files: Vec<(String, Vec<Diagnostic>)> = files.to_vec();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);
    for (_, diagnostics) in &mut files {
        diagnostics.sort_by(|a, b| {
            (a.range, a.severity as u8, &a.message, &a.source).cmp(&(
                b.range,
                b.severity as u8,
                &b.message,
                &b.source,
            ))
        });
        diagnostics.dedup();
    }
    let position =
        |position: Position| json!({ "line": position.line, "character": position.column });
    let files: Vec<Value> = files
        .iter()
        .map(|(absolute, diagnostics)| {
            let diagnostics: Vec<Value> = diagnostics
                .iter()
                .map(|diagnostic| {
                    let mut entry = Map::new();
                    entry.insert("message".into(), diagnostic.message.clone().into());
                    entry.insert(
                        "severity".into(),
                        match diagnostic.severity {
                            Severity::Error => "Error",
                            Severity::Warning => "Warning",
                            Severity::Information => "Information",
                            Severity::Hint => "Hint",
                        }
                        .into(),
                    );
                    entry.insert(
                        "range".into(),
                        json!({
                            "start": position(diagnostic.range.start),
                            "end": position(diagnostic.range.end),
                        }),
                    );
                    if let Some(source) = &diagnostic.source {
                        entry.insert("source".into(), source.clone().into());
                    }
                    Value::Object(entry)
                })
                .collect();
            json!({ "uri": path_to_uri(absolute), "diagnostics": diagnostics })
        })
        .collect();
    text_result(&Value::Array(files).to_string())
}

/// `getDiagnostics` answers within one turn: Claude asks about four times a turn, for one file
/// and for every file. The same question gets the same text until the host's diagnostics change
/// or a turn begins.
#[derive(Debug, Default)]
pub struct DiagnosticsCache {
    answers: BTreeMap<Option<FileRef>, String>,
}

impl DiagnosticsCache {
    /// A turn began, or the host's diagnostics changed.
    pub fn invalidate(&mut self) {
        self.answers.clear();
    }

    /// The answer for `file` (`None`: every file), computed once until [`Self::invalidate`].
    pub fn answer(&mut self, file: Option<FileRef>, compute: impl FnOnce() -> String) -> String {
        self.answers.entry(file).or_insert_with(compute).clone()
    }
}

/// The `openDiff` calls of one connection, by tab name. Claude calls `closeAllDiffTabs` at every
/// prompt and `close_tab` twice after each answered diff, so a close is a no-op unless the
/// proposal is still open; only then does the extension withdraw it (`review.withdraw`) and
/// answer the open call with `TAB_CLOSED`.
#[derive(Debug)]
pub struct Diffs<K> {
    open: Vec<(String, K)>,
}

impl<K> Default for Diffs<K> {
    fn default() -> Self {
        Self { open: Vec::new() }
    }
}

impl<K: Clone + PartialEq> Diffs<K> {
    /// An `openDiff` call, as `key` (the proposal, or the call).
    pub fn opened(&mut self, tab: String, key: K) {
        self.open.retain(|(open, _)| *open != tab);
        self.open.push((tab, key));
    }

    /// The user decided in Fyx, and the call was answered.
    pub fn decided(&mut self, key: &K) -> bool {
        let before = self.open.len();
        self.open.retain(|(_, open)| open != key);
        self.open.len() != before
    }

    /// `close_tab`: the proposal to withdraw, if it is still open.
    pub fn close(&mut self, tab: &str) -> Option<K> {
        let index = self.open.iter().position(|(open, _)| open == tab)?;
        Some(self.open.remove(index).1)
    }

    /// `closeAllDiffTabs`: the proposals still open, to withdraw.
    pub fn close_all(&mut self) -> Vec<K> {
        self.open.drain(..).map(|(_, key)| key).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.open.iter().map(|(_, key)| key)
    }
}

/// What the IDE server needs from the host: the roots, the answers, and the diagnostics.
pub trait IdeHost {
    fn roots(&self) -> &dyn Roots;
    /// The answer to call `call` (`listeners.respond`).
    fn respond(&mut self, call: u64, outcome: Result<String, McpError>);
    /// What the host's diagnostics hold, by absolute path: of the file `only`, or of every file.
    /// A file with nothing is left out.
    fn diagnostics(&self, only: Option<&str>) -> Vec<(String, Vec<Diagnostic>)>;
    /// Opens `file` in the editor (`context.open`); the error is the host's message.
    fn open(&mut self, file: &FileRef, focus: bool) -> Result<(), String>;
    /// Proposes `new_contents` for `file` in agent review (`review.propose`, `apply = false`):
    /// the proposal's id, `None` when the host has no ids to give (the compat replay), or the
    /// host's refusal.
    fn propose(&mut self, file: &FileRef, new_contents: &str) -> Result<Option<u64>, String>;
    /// Closes an undecided proposal (`review.withdraw`).
    fn withdraw(&mut self, proposal: u64);
}

/// The extension's side of the `ide` listener: Claude's `tools/call`s by the host's call id,
/// the `openDiff` calls still open, and the diagnostics answers of this turn. The WASM side and
/// the compat replay drive the same server.
#[derive(Debug, Default)]
pub struct Server {
    diffs: Diffs<u64>,
    /// The review proposal of each open `openDiff` call.
    proposals: BTreeMap<u64, u64>,
    cache: DiagnosticsCache,
}

impl Server {
    /// `io-event::mcp-call` on [`LISTENER`]. Every call is answered now except `openDiff`, which
    /// becomes a proposal in agent review and stays open until [`Server::review_decided`] (or
    /// [`Server::decided`]) or a close.
    pub fn call(&mut self, host: &mut dyn IdeHost, call: u64, tool: &str, arguments: &str) {
        match IdeCall::parse(tool, arguments, host.roots()) {
            Ok(IdeCall::OpenDiff {
                file,
                new_contents,
                tab,
            }) => {
                // A second diff in the same tab replaces the first.
                if let Some(open) = self.diffs.close(&tab) {
                    self.close(host, open);
                }
                match host.propose(&file, &new_contents) {
                    Ok(proposal) => {
                        self.diffs.opened(tab, call);
                        if let Some(proposal) = proposal {
                            self.proposals.insert(call, proposal);
                        }
                    }
                    // Without a review Claude keeps its edit and asks in the terminal.
                    Err(_) => host.respond(call, Ok(DiffAnswer::TabClosed.result())),
                }
            }
            // A close withdraws only a proposal that is still open, whose call then ends with
            // `TAB_CLOSED`; after a decision it is a no-op.
            Ok(IdeCall::CloseTab { tab }) => {
                if let Some(open) = self.diffs.close(&tab) {
                    self.close(host, open);
                }
                host.respond(call, Ok(text_result("TAB_CLOSED")));
            }
            Ok(IdeCall::CloseAllDiffTabs) => {
                for open in self.diffs.close_all() {
                    self.close(host, open);
                }
                host.respond(call, Ok(text_result("CLOSED_ALL_DIFF_TABS")));
            }
            Ok(IdeCall::GetDiagnostics { file }) => {
                let only = file
                    .as_ref()
                    .and_then(|file| host.roots().to_absolute(file));
                let answer = self.cache.answer(file, || {
                    diagnostics_result(&host.diagnostics(only.as_deref()))
                });
                host.respond(call, Ok(answer));
            }
            // The selection Claude asks for (`startText`, `endText`) needs the file's text,
            // which the extension cannot read; the file opens without one.
            Ok(IdeCall::OpenFile { file, focus, .. }) => {
                let outcome =
                    host.open(&file, focus)
                        .map(|()| text_result("OK"))
                        .map_err(|message| McpError {
                            code: INTERNAL_ERROR,
                            message: format!("openFile: {message}"),
                        });
                host.respond(call, outcome);
            }
            Err(error) => host.respond(call, Err(error)),
        }
    }

    /// Withdraws the open `openDiff` call `call`'s proposal and answers it `TAB_CLOSED`.
    fn close(&mut self, host: &mut dyn IdeHost, call: u64) {
        if let Some(proposal) = self.proposals.remove(&call) {
            host.withdraw(proposal);
        }
        host.respond(call, Ok(DiffAnswer::TabClosed.result()));
    }

    /// The user decided the `openDiff` call `call` in Fyx; false when it is not open.
    pub fn decided(&mut self, host: &mut dyn IdeHost, call: u64, answer: &DiffAnswer) -> bool {
        if !self.diffs.decided(&call) {
            return false;
        }
        self.proposals.remove(&call);
        host.respond(call, Ok(answer.result()));
        true
    }

    /// `io-event::review-decided` for `proposal`: answers its `openDiff` call. False when no
    /// open call has it (an outcome that arrives after a close).
    pub fn review_decided(
        &mut self,
        host: &mut dyn IdeHost,
        proposal: u64,
        answer: &DiffAnswer,
    ) -> bool {
        let Some(call) = self
            .proposals
            .iter()
            .find_map(|(call, open)| (*open == proposal).then_some(*call))
        else {
            return false;
        };
        self.decided(host, call, answer)
    }

    /// `io-event::mcp-cancelled`: the call's connection closed or Claude gave up; its proposal
    /// is withdrawn.
    pub fn cancelled(&mut self, host: &mut dyn IdeHost, call: u64) {
        if self.diffs.decided(&call)
            && let Some(proposal) = self.proposals.remove(&call)
        {
            host.withdraw(proposal);
        }
    }

    /// A turn began, or the host's diagnostics changed.
    pub fn invalidate_diagnostics(&mut self) {
        self.cache.invalidate();
    }

    /// The `openDiff` calls still open.
    pub fn open_diffs(&self) -> Vec<u64> {
        self.diffs.keys().copied().collect()
    }
}

/// `selection_changed`'s params, from `context.editor-context`'s active file and selection.
/// `None` when the file is outside every root.
pub fn selection_changed(
    file: &FileRef,
    range: TextRange,
    text: Option<&str>,
    roots: &dyn Roots,
) -> Option<String> {
    let absolute = roots.to_absolute(file)?;
    let position =
        |position: Position| json!({ "line": position.line, "character": position.column });
    Some(
        json!({
            "text": text.unwrap_or_default(),
            "filePath": absolute,
            "fileUrl": path_to_uri(&absolute),
            "selection": {
                "start": position(range.start),
                "end": position(range.end),
                "isEmpty": range.start == range.end,
            },
        })
        .to_string(),
    )
}

/// `at_mentioned`'s params: the user sends a file or lines to Claude from the editor. Lines are
/// zero-based, as Claude's other messages.
pub fn at_mentioned(
    file: &FileRef,
    lines: Option<(u32, u32)>,
    roots: &dyn Roots,
) -> Option<String> {
    let absolute = roots.to_absolute(file)?;
    let (start, end) = lines.map_or((Value::Null, Value::Null), |(start, end)| {
        (start.into(), end.into())
    });
    Some(json!({ "filePath": absolute, "lineStart": start, "lineEnd": end }).to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One worktree, id 1, at `root`.
    pub struct OneRoot(pub &'static str);

    impl Roots for OneRoot {
        fn to_ref(&self, absolute: &str) -> Option<FileRef> {
            let relative = absolute.strip_prefix(self.0)?.strip_prefix('/')?;
            let clean = !relative.is_empty()
                && relative
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | ".."));
            clean.then(|| FileRef {
                worktree: Some(1),
                path: relative.to_owned(),
            })
        }

        fn to_absolute(&self, file: &FileRef) -> Option<String> {
            (file.worktree == Some(1)).then(|| format!("{}/{}", self.0, file.path))
        }
    }

    const ROOT: OneRoot = OneRoot("/work/app");

    #[test]
    fn tools_have_object_schemas() {
        let tools = tools();
        assert_eq!(
            tools.iter().map(|tool| tool.name).collect::<Vec<_>>(),
            [
                "openFile",
                "openDiff",
                "close_tab",
                "closeAllDiffTabs",
                "getDiagnostics"
            ]
        );
        for tool in tools {
            let schema: Value = serde_json::from_str(&tool.input_schema).unwrap();
            assert_eq!(schema["type"], "object", "{}", tool.name);
        }
    }

    #[test]
    fn open_diff_maps_to_a_worktree_file() {
        let call = IdeCall::parse(
            "openDiff",
            r#"{"old_file_path":"/work/app/src/a.rs","new_file_path":"/work/app/src/a.rs",
               "new_file_contents":"fn a() {}\n","tab_name":"✻ [Claude Code] a.rs ⧉"}"#,
            &ROOT,
        )
        .unwrap();
        assert_eq!(
            call,
            IdeCall::OpenDiff {
                file: FileRef {
                    worktree: Some(1),
                    path: "src/a.rs".into()
                },
                new_contents: "fn a() {}\n".into(),
                tab: "✻ [Claude Code] a.rs ⧉".into(),
            }
        );
    }

    #[test]
    fn calls_outside_the_worktrees_or_malformed_are_refused() {
        let refused = [
            ("openFile", r#"{"filePath":"/etc/passwd"}"#, INVALID_PARAMS),
            (
                "openFile",
                r#"{"filePath":"/work/app/../x"}"#,
                INVALID_PARAMS,
            ),
            ("openFile", r#"{}"#, INVALID_PARAMS),
            ("openFile", r#"[]"#, INVALID_PARAMS),
            ("openFile", r#"not json"#, INVALID_PARAMS),
            (
                "openDiff",
                r#"{"old_file_path":"/work/app/a","new_file_path":"/work/app/b","new_file_contents":"","tab_name":"t"}"#,
                INVALID_PARAMS,
            ),
            ("executeCode", r#"{"code":"1"}"#, UNKNOWN_TOOL),
        ];
        for (tool, arguments, code) in refused {
            let error = IdeCall::parse(tool, arguments, &ROOT).unwrap_err();
            assert_eq!(error.code, code, "{tool} {arguments}: {error:?}");
        }
    }

    #[test]
    fn diff_answers_match_what_claude_reads() {
        let saved: Value = serde_json::from_str(
            &DiffAnswer::from_review(ReviewDecision::Partial, "kept\n".into()).result(),
        )
        .unwrap();
        assert_eq!(saved["content"][0]["text"], "FILE_SAVED");
        assert_eq!(saved["content"][1]["text"], "kept\n");
        let rejected: Value = serde_json::from_str(
            &DiffAnswer::from_review(ReviewDecision::Rejected, String::new()).result(),
        )
        .unwrap();
        assert_eq!(rejected["content"][0]["text"], "DIFF_REJECTED");
        assert_eq!(
            rejected["content"].as_array().map(Vec::len),
            Some(1),
            "{rejected}"
        );
    }

    #[test]
    fn notifications_use_absolute_paths() {
        let file = FileRef {
            worktree: Some(1),
            path: "src/a.rs".into(),
        };
        let range = TextRange {
            start: Position { line: 2, column: 0 },
            end: Position { line: 4, column: 3 },
        };
        let params: Value =
            serde_json::from_str(&selection_changed(&file, range, Some("abc"), &ROOT).unwrap())
                .unwrap();
        assert_eq!(params["filePath"], "/work/app/src/a.rs");
        assert_eq!(params["fileUrl"], "file:///work/app/src/a.rs");
        assert_eq!(params["selection"]["end"]["character"], 3);
        assert_eq!(params["selection"]["isEmpty"], false);
        let mention: Value =
            serde_json::from_str(&at_mentioned(&file, Some((2, 4)), &ROOT).unwrap()).unwrap();
        assert_eq!(mention["lineStart"], 2);
        let elsewhere = FileRef {
            worktree: Some(2),
            path: "x".into(),
        };
        assert!(selection_changed(&elsewhere, range, None, &ROOT).is_none());
    }

    #[test]
    fn windows_paths_and_both_uri_forms() {
        assert_eq!(normalize_path(r"c:\work\app\a b.rs"), "C:/work/app/a b.rs");
        assert_eq!(normalize_path("/work/app/a.rs"), "/work/app/a.rs");
        for uri in [
            "file:///C:/work/app/a%20b.rs",
            "file:///c:/work/app/a%20b.rs",
            r"file://C:\work\app\a b.rs",
            "file://localhost/C:/work/app/a b.rs",
        ] {
            assert_eq!(
                uri_to_path(uri).as_deref(),
                Some("C:/work/app/a b.rs"),
                "{uri}"
            );
        }
        assert_eq!(
            uri_to_path("file:///work/app/a.rs").as_deref(),
            Some("/work/app/a.rs")
        );
        assert_eq!(uri_to_path("https://example.com/a"), None);
        assert_eq!(uri_to_path("file:///C:/a%2"), None);
        assert_eq!(
            path_to_uri(r"C:\work\app\a b.rs"),
            "file:///C:/work/app/a%20b.rs"
        );
        assert_eq!(path_to_uri("/work/app/ü.rs"), "file:///work/app/%C3%BC.rs");

        let windows = OneRoot("C:/work/app");
        let expected = Some(FileRef {
            worktree: Some(1),
            path: "src/a.rs".into(),
        });
        for uri in [
            r"file://C:\work\app\src\a.rs",
            "file:///C:/work/app/src/a.rs",
        ] {
            let call = IdeCall::parse(
                "getDiagnostics",
                &serde_json::to_string(&json!({ "uri": uri })).unwrap(),
                &windows,
            )
            .unwrap();
            assert_eq!(
                call,
                IdeCall::GetDiagnostics {
                    file: expected.clone()
                },
                "{uri}"
            );
        }
        let call = IdeCall::parse(
            "openFile",
            r#"{"filePath":"C:\\work\\app\\src\\a.rs"}"#,
            &windows,
        );
        assert!(call.is_ok(), "{call:?}");
        let error = IdeCall::parse("getDiagnostics", r#"{"uri":"untitled:1"}"#, &windows);
        assert_eq!(error.unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn diagnostics_answers_are_stable_and_cached() {
        let diagnostic = |line, severity, message: &str| Diagnostic {
            range: TextRange {
                start: Position { line, column: 0 },
                end: Position { line, column: 1 },
            },
            severity,
            message: message.into(),
            source: None,
        };
        let a = vec![
            diagnostic(3, Severity::Hint, "unused"),
            diagnostic(1, Severity::Error, "typo"),
            diagnostic(3, Severity::Hint, "unused"),
        ];
        let one = diagnostics_result(&[
            ("/work/b.rs".into(), vec![]),
            ("/work/a.rs".into(), a.clone()),
        ]);
        let reversed: Vec<_> = a.into_iter().rev().collect();
        let other = diagnostics_result(&[
            ("/work/a.rs".into(), reversed),
            ("/work/b.rs".into(), vec![]),
        ]);
        assert_eq!(one, other);
        let text: Value = serde_json::from_str(&one).unwrap();
        let files: Value =
            serde_json::from_str(text["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(files[0]["uri"], "file:///work/a.rs");
        assert_eq!(files[0]["diagnostics"].as_array().map(Vec::len), Some(2));
        assert_eq!(files[0]["diagnostics"][0]["message"], "typo");

        let mut cache = DiagnosticsCache::default();
        let mut computed = 0;
        for _ in 0..3 {
            cache.answer(None, || {
                computed += 1;
                "all".into()
            });
        }
        assert_eq!(computed, 1);
        cache.invalidate();
        assert_eq!(cache.answer(None, || "new".into()), "new");
    }

    #[test]
    fn closes_withdraw_only_open_proposals() {
        let mut diffs = Diffs::default();
        diffs.opened("tab a".into(), 1);
        diffs.opened("tab b".into(), 2);
        // Decided in Fyx: Claude's two close_tab calls after it are no-ops.
        assert!(diffs.decided(&1));
        assert!(!diffs.decided(&1));
        assert_eq!(diffs.close("tab a"), None);
        assert_eq!(diffs.close("tab a"), None);
        // closeAllDiffTabs at the next prompt withdraws only what is still open.
        assert_eq!(diffs.close_all(), [2]);
        assert_eq!(diffs.close_all(), Vec::<i32>::new());
        diffs.opened("tab c".into(), 3);
        assert_eq!(diffs.close("tab c"), Some(3));
        assert!(diffs.is_empty());
    }
}
