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

use serde_json::{Map, Value, json};

/// The `ide` listener in `extension.toml`.
pub const LISTENER: &str = "ide";

/// JSON-RPC's "invalid params" and "method not found", as MCP uses them.
pub const INVALID_PARAMS: i32 = -32602;
pub const UNKNOWN_TOOL: i32 = -32601;

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// `types.text-range`, half-open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRange {
    pub start: Position,
    pub end: Position,
}

/// Absolute paths, as Claude names files, against Fyx's worktree-relative [`FileRef`]s.
///
/// The 0.4 WIT gives an extension no absolute paths, so this needs a host primitive (reported
/// to the coordinator, 5.15); until then it is a trait the tests fill.
pub trait Roots {
    fn to_ref(&self, absolute: &str) -> Option<FileRef>;
    fn to_absolute(&self, file: &FileRef) -> Option<String>;
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
            description: "Open a file in the editor, optionally selecting text between two markers.",
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
            description: "Show a proposed change for review and wait for the user's decision.",
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
            description: "Close a diff opened with openDiff.",
            input_schema: schema(json!({ "tab_name": text }), &["tab_name"]),
        },
        ToolDef {
            name: "closeAllDiffTabs",
            description: "Close every diff opened with openDiff.",
            input_schema: schema(json!({}), &[]),
        },
        ToolDef {
            name: "getDiagnostics",
            description: "Diagnostics for one file URI, or for every open file.",
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
            roots.to_ref(path).ok_or_else(|| {
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
                    Some(uri) => Some(file(uri.strip_prefix("file://").unwrap_or(uri))?),
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
pub fn diagnostics_result(files: &[(String, Vec<Diagnostic>)]) -> String {
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
            json!({ "uri": format!("file://{absolute}"), "diagnostics": diagnostics })
        })
        .collect();
    text_result(&Value::Array(files).to_string())
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
            "fileUrl": format!("file://{absolute}"),
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
}
