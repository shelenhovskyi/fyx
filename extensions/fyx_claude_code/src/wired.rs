//! The IDE server on the SDK's 0.4 `listeners`, `context` and `review` (plan 5.14): Claude's
//! absolute paths map to worktree files through `context.to-file-ref` and
//! `context.absolute-path`, `openFile` is `context.open`, the editor feed becomes
//! `selection_changed`, and `openDiff` is a proposal in agent review.

use ui_ext_sdk::{LogLevel, context, listeners, log, review};

use crate::ide::{
    self, Diagnostic, DiffAnswer, FileRef, IdeHost, McpError, Position, ReviewDecision, Roots,
    Severity, TextRange,
};

/// The tool session agent review names `openDiff`'s proposals by, until the hooks (through the
/// relay) name Claude's own sessions.
pub(crate) const SESSION: &str = "Claude Code";

/// A review title: "Claude Code: <file name>", at most 256 bytes.
fn title(file: &FileRef) -> String {
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let mut title = format!("{SESSION}: {name}");
    if title.len() > 256 {
        let mut end = 256;
        while !title.is_char_boundary(end) {
            end -= 1;
        }
        title.truncate(end);
    }
    title
}

/// `io-event::review-decided`'s outcome as Claude's answer: an undecided end (the file went
/// away) is `TAB_CLOSED`.
pub(crate) fn diff_answer(outcome: Result<review::ReviewOutcome, String>) -> DiffAnswer {
    match outcome {
        Ok(outcome) => DiffAnswer::from_review(
            match outcome.decision {
                review::Decision::Accepted => ReviewDecision::Accepted,
                review::Decision::Partial => ReviewDecision::Partial,
                review::Decision::Rejected => ReviewDecision::Rejected,
            },
            outcome.final_content,
        ),
        Err(_) => DiffAnswer::TabClosed,
    }
}

/// Offers [`ide::tools`] on the `ide` listener.
pub(crate) fn register_tools() {
    let tools: Vec<listeners::ToolDef> = ide::tools()
        .into_iter()
        .map(|tool| listeners::ToolDef::new(tool.name, tool.description, tool.input_schema))
        .collect();
    if let Err(error) = listeners::register_tools(ide::LISTENER, &tools) {
        log(
            LogLevel::Warn,
            &format!("the IDE tools were not registered: {}", error.message),
        );
    }
}

/// Starts `diagnostics-changed`, which refreshes the cached `getDiagnostics` answers.
pub(crate) fn watch_diagnostics() {
    if let Err(error) = context::watch_diagnostics(true) {
        log(
            LogLevel::Warn,
            &format!("diagnostics changes are not watched: {}", error.message),
        );
    }
}

fn severity(severity: context::Severity) -> Severity {
    match severity {
        context::Severity::Error => Severity::Error,
        context::Severity::Warning => Severity::Warning,
        context::Severity::Information => Severity::Information,
        context::Severity::Hint => Severity::Hint,
    }
}

/// Starts the editor feed that [`selection_changed`] reads.
pub(crate) fn watch_context() {
    if let Err(error) = context::watch(true) {
        log(
            LogLevel::Warn,
            &format!("the editor context is off: {}", error.message),
        );
    }
}

fn file_ref(file: ui_ext_sdk::FileRef) -> FileRef {
    FileRef {
        worktree: file.worktree,
        path: file.path,
    }
}

fn sdk_file_ref(file: &FileRef) -> ui_ext_sdk::FileRef {
    ui_ext_sdk::FileRef {
        worktree: file.worktree,
        path: file.path.clone(),
    }
}

fn text_range(range: ui_ext_sdk::TextRange) -> TextRange {
    let position = |position: ui_ext_sdk::Position| Position {
        line: position.line,
        column: position.column,
    };
    TextRange {
        start: position(range.start),
        end: position(range.end),
    }
}

/// The host's worktrees. A path the host refuses (outside every worktree, or private) maps to
/// nothing, and Claude's call is refused as outside.
struct ContextRoots;

impl Roots for ContextRoots {
    fn to_ref(&self, absolute: &str) -> Option<FileRef> {
        context::to_file_ref(absolute).ok().map(file_ref)
    }

    fn to_absolute(&self, file: &FileRef) -> Option<String> {
        context::absolute_path(&sdk_file_ref(file)).ok()
    }
}

/// `selection_changed`'s params for the editor's context: `None` without an active file with a
/// selection, or when it maps to no absolute path.
pub(crate) fn selection_changed(editor: context::EditorContext) -> Option<String> {
    let active = file_ref(editor.active?);
    let (range, text) = editor.selection?;
    ide::selection_changed(&active, text_range(range), text.as_deref(), &ContextRoots)
}

/// Sends `params` to `connection`, or to every connection of the `ide` listener.
pub(crate) fn notify_selection(connection: Option<u64>, params: &str) {
    if let Err(error) = listeners::notify(ide::LISTENER, connection, "selection_changed", params) {
        log(
            LogLevel::Warn,
            &format!("selection_changed was not sent: {}", error.message),
        );
    }
}

/// The host, through the SDK.
pub(crate) struct SdkHost;

impl IdeHost for SdkHost {
    fn roots(&self) -> &dyn Roots {
        &ContextRoots
    }

    fn respond(&mut self, call: u64, outcome: Result<String, McpError>) {
        let error = outcome
            .as_ref()
            .err()
            .map(|error| listeners::McpError::new(error.code, error.message.clone()));
        let outcome = match (&outcome, &error) {
            (Ok(result), _) => Ok(result.as_str()),
            (Err(_), Some(error)) => Err(error),
            (Err(_), None) => unreachable!("an error outcome has an McpError"),
        };
        if let Err(error) = listeners::respond(call, outcome) {
            log(
                LogLevel::Warn,
                &format!("the answer to call {call} was refused: {}", error.message),
            );
        }
    }

    /// `context.diagnostics`, by absolute path; a refused read answers no files.
    fn diagnostics(&self, only: Option<&FileRef>) -> Vec<(String, Vec<Diagnostic>)> {
        let only = only.map(sdk_file_ref);
        let snapshot = match context::diagnostics(only.as_ref()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                log(
                    LogLevel::Warn,
                    &format!("diagnostics are not readable: {}", error.message),
                );
                return Vec::new();
            }
        };
        snapshot
            .files
            .into_iter()
            .filter_map(|file| {
                let absolute = context::absolute_path(&file.path).ok()?;
                let entries = file
                    .diagnostics
                    .into_iter()
                    .map(|entry| Diagnostic {
                        range: text_range(entry.range),
                        severity: severity(entry.severity),
                        message: entry.message,
                        source: entry.source,
                    })
                    .collect();
                Some((absolute, entries))
            })
            .collect()
    }

    fn open(&mut self, file: &FileRef, focus: bool) -> Result<(), String> {
        context::open(&sdk_file_ref(file), None, focus).map_err(|error| error.message)
    }

    fn propose(&mut self, file: &FileRef, new_contents: &str) -> Result<Option<u64>, String> {
        let proposal =
            review::Proposal::new_content(sdk_file_ref(file), new_contents, title(file), SESSION);
        match review::propose(&proposal) {
            Ok(id) => Ok(Some(id)),
            Err(error) => {
                log(
                    LogLevel::Warn,
                    &format!(
                        "openDiff of {:?} is not in review: {}",
                        file.path, error.message
                    ),
                );
                Err(error.message)
            }
        }
    }

    fn withdraw(&mut self, proposal: u64) {
        // An outcome on its way is no error: the close came first.
        review::withdraw(proposal).ok();
    }
}

#[cfg(test)]
mod tests {
    use ui_ext_sdk::context::{ContextEffect, EditorContext, set_worktree_roots};
    use ui_ext_sdk::listeners::{ListenerEffect, McpCall};
    use ui_ext_sdk::{Extension as _, HostEffect, IoEvent, take_host_effects};

    use crate::{ClaudeCode, ide};

    fn call(call_id: u64, tool: &str, arguments: &str) -> IoEvent {
        IoEvent::McpCall(McpCall {
            call_id,
            listener: ide::LISTENER.into(),
            connection: 1,
            tool: tool.into(),
            arguments: arguments.into(),
        })
    }

    fn responses(effects: Vec<HostEffect>) -> Vec<(u64, Result<String, i32>)> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                HostEffect::Listener(ListenerEffect::Respond { call_id, outcome }) => {
                    Some((call_id, outcome.map_err(|error| error.code)))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_ide_tools_are_registered_and_calls_answered() {
        set_worktree_roots(&[]);
        take_host_effects();
        let mut extension = ClaudeCode::new();
        let effects = take_host_effects();
        let [
            HostEffect::Listener(ListenerEffect::RegisterTools { listener, tools }),
            HostEffect::Context(ContextEffect::Watch(true)),
            HostEffect::Context(ContextEffect::WatchDiagnostics(true)),
        ] = effects.as_slice()
        else {
            panic!("{effects:?}");
        };
        assert_eq!(listener, ide::LISTENER);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        let ours: Vec<&str> = ide::tools().iter().map(|tool| tool.name).collect();
        assert_eq!(names, ours);

        extension
            .handle_io(call(5, "closeAllDiffTabs", "{}"))
            .unwrap();
        extension
            .handle_io(call(6, "getDiagnostics", "{}"))
            .unwrap();
        extension
            .handle_io(call(7, "openFile", r#"{"filePath":"/work/app/a.rs"}"#))
            .unwrap();
        assert_eq!(
            responses(take_host_effects()),
            [
                (5, Ok(ide::text_result("CLOSED_ALL_DIFF_TABS"))),
                (6, Ok(ide::text_result("[]"))),
                (7, Err(ide::INVALID_PARAMS)),
            ]
        );
    }

    #[test]
    fn paths_map_through_context() {
        set_worktree_roots(&[(4, r"C:\work\app")]);
        let mut extension = ClaudeCode::new();
        take_host_effects();
        extension
            .handle_io(call(
                1,
                "openFile",
                r#"{"filePath":"c:\\work\\app\\src\\a.rs","makeFrontmost":false}"#,
            ))
            .unwrap();
        let effects = take_host_effects();
        let file = ui_ext_sdk::FileRef {
            worktree: Some(4),
            path: "src/a.rs".into(),
        };
        assert!(
            effects.contains(&HostEffect::Context(ContextEffect::Open {
                path: file.clone(),
                range: None,
                focus: false,
            })),
            "{effects:?}"
        );
        assert_eq!(responses(effects), [(1, Ok(ide::text_result("OK")))]);

        let range = ui_ext_sdk::TextRange {
            start: ui_ext_sdk::Position { line: 2, column: 1 },
            end: ui_ext_sdk::Position { line: 2, column: 4 },
        };
        let editor = EditorContext {
            active: Some(file.clone()),
            open: vec![file],
            selection: Some((range, Some("abc".into()))),
            trusted: Some(true),
        };
        extension
            .handle_io(IoEvent::ContextChanged(editor.clone()))
            .unwrap();
        let effects = take_host_effects();
        let [
            HostEffect::Listener(ListenerEffect::Notify {
                listener,
                connection: None,
                method,
                params,
            }),
        ] = effects.as_slice()
        else {
            panic!("{effects:?}");
        };
        assert_eq!(
            (listener.as_str(), method.as_str()),
            (ide::LISTENER, "selection_changed")
        );
        let params: serde_json::Value = serde_json::from_str(params).unwrap();
        assert_eq!(params["filePath"], r"C:\work\app\src\a.rs");
        assert_eq!(params["text"], "abc");
        assert_eq!(params["selection"]["start"]["character"], 1);
        // The same selection again sends nothing.
        extension
            .handle_io(IoEvent::ContextChanged(editor))
            .unwrap();
        assert!(take_host_effects().is_empty());
        // A client that finishes the handshake later gets it on its connection.
        extension
            .handle_io(IoEvent::McpInitialized(ui_ext_sdk::listeners::McpSession {
                listener: ide::LISTENER.into(),
                connection: 5,
                client_name: "claude-code".into(),
                client_version: "2.1.326".into(),
            }))
            .unwrap();
        assert!(
            matches!(
                take_host_effects().as_slice(),
                [HostEffect::Listener(ListenerEffect::Notify {
                    connection: Some(5),
                    method,
                    ..
                })] if method == "selection_changed"
            ),
            "the selection again"
        );
        set_worktree_roots(&[]);
    }

    #[test]
    fn diagnostics_are_read_and_refreshed() {
        use ui_ext_sdk::context::{Diagnostic, FileDiagnostics, Severity, set_diagnostics};
        set_worktree_roots(&[(1, "/work/app")]);
        let report = |message: &str| {
            vec![FileDiagnostics {
                path: ui_ext_sdk::FileRef {
                    worktree: Some(1),
                    path: "src/a.rs".into(),
                },
                diagnostics: vec![Diagnostic {
                    range: ui_ext_sdk::TextRange {
                        start: ui_ext_sdk::Position { line: 3, column: 4 },
                        end: ui_ext_sdk::Position { line: 3, column: 9 },
                    },
                    severity: Severity::Error,
                    message: message.into(),
                    source: Some("rustc".into()),
                    code: Some("E0308".into()),
                }],
            }]
        };
        let mut extension = ClaudeCode::new();
        set_diagnostics(report("mismatched types"));
        take_host_effects();
        let answer = |extension: &mut ClaudeCode, call_id| {
            extension
                .handle_io(call(call_id, "getDiagnostics", "{}"))
                .unwrap();
            let answers = responses(take_host_effects());
            let [(_, Ok(result))] = answers.as_slice() else {
                panic!("one answer");
            };
            let result: serde_json::Value = serde_json::from_str(result).unwrap();
            let text = result["content"][0]["text"].as_str().unwrap().to_owned();
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        };
        let first = answer(&mut extension, 1);
        assert_eq!(first[0]["uri"], "file:///work/app/src/a.rs", "{first}");
        assert_eq!(first[0]["diagnostics"][0]["message"], "mismatched types");
        // Cached until the host says the diagnostics changed.
        set_diagnostics(report("fixed differently"));
        assert_eq!(answer(&mut extension, 2), first);
        extension
            .handle_io(IoEvent::DiagnosticsChanged(Vec::new()))
            .unwrap();
        let refreshed = answer(&mut extension, 3);
        assert_eq!(
            refreshed[0]["diagnostics"][0]["message"],
            "fixed differently"
        );
        set_diagnostics(Vec::new());
        set_worktree_roots(&[]);
    }

    #[test]
    fn open_diffs_are_reviews() {
        use ui_ext_sdk::review::{Decision, Proposed, ReviewEffect, ReviewOutcome};
        set_worktree_roots(&[(1, "/work/app")]);
        let mut extension = ClaudeCode::new();
        take_host_effects();
        let open_diff = |call_id, tab: &str| {
            call(
                call_id,
                "openDiff",
                &serde_json::json!({
                    "old_file_path": "/work/app/a.rs",
                    "new_file_path": "/work/app/a.rs",
                    "new_file_contents": "new\n",
                    "tab_name": tab,
                })
                .to_string(),
            )
        };
        let proposed = |effects: &[HostEffect]| {
            effects.iter().find_map(|effect| match effect {
                HostEffect::Review(ReviewEffect::Propose { id, proposal }) => {
                    Some((*id, proposal.clone()))
                }
                _ => None,
            })
        };

        // Decided in Fyx: the call is answered once; the later close_tab is a no-op.
        extension.handle_io(open_diff(1, "tab-a")).unwrap();
        let effects = take_host_effects();
        let (id, proposal) = proposed(&effects).expect("a proposal");
        assert_eq!(proposal.path.path, "a.rs");
        assert_eq!(proposal.change, Proposed::NewContent("new\n".into()));
        assert!(!proposal.apply, "Claude writes the file");
        assert!(responses(effects).is_empty(), "the call stays open");
        let outcome = ReviewOutcome {
            decision: Decision::Partial,
            final_content: "kept\n".into(),
        };
        extension
            .handle_io(IoEvent::ReviewDecided((id, Ok(outcome.clone()))))
            .unwrap();
        assert_eq!(
            responses(take_host_effects()),
            [(1, Ok(ide::DiffAnswer::Saved("kept\n".into()).result()))]
        );
        extension
            .handle_io(IoEvent::ReviewDecided((id, Ok(outcome))))
            .unwrap();
        extension
            .handle_io(call(2, "close_tab", r#"{"tab_name":"tab-a"}"#))
            .unwrap();
        let effects = take_host_effects();
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, HostEffect::Review(ReviewEffect::Withdraw(_)))),
            "{effects:?}"
        );
        assert_eq!(
            responses(effects),
            [(2, Ok(ide::text_result("TAB_CLOSED")))]
        );

        // Closed before a decision: withdrawn, and both calls end.
        extension.handle_io(open_diff(3, "tab-b")).unwrap();
        let (id, _) = proposed(&take_host_effects()).expect("a proposal");
        extension
            .handle_io(call(4, "close_tab", r#"{"tab_name":"tab-b"}"#))
            .unwrap();
        let effects = take_host_effects();
        assert!(effects.contains(&HostEffect::Review(ReviewEffect::Withdraw(id))));
        assert_eq!(
            responses(effects),
            [
                (3, Ok(ide::DiffAnswer::TabClosed.result())),
                (4, Ok(ide::text_result("TAB_CLOSED"))),
            ]
        );

        // Claude went away: the proposal is withdrawn, nothing answered.
        extension.handle_io(open_diff(5, "tab-c")).unwrap();
        let (id, _) = proposed(&take_host_effects()).expect("a proposal");
        extension.handle_io(IoEvent::McpCancelled(5)).unwrap();
        assert_eq!(
            take_host_effects(),
            [HostEffect::Review(ReviewEffect::Withdraw(id))]
        );
        set_worktree_roots(&[]);
    }
}
