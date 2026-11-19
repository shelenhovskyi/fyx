//! The companion on the SDK's 0.4 `listeners`, `context` and `review` (plan 5.16): Gemini's
//! absolute paths map to worktree files through `context.to-file-ref` and
//! `context.absolute-path`, the editor feed becomes `ide/contextUpdate`, and `openDiff` is a
//! proposal in agent review, withdrawn (with its current text) by `closeDiff`.

use ui_ext_sdk::{LogLevel, context, listeners, log, review};

use crate::companion::{
    self, CompanionHost, FileRef, McpError, Notify, OpenFile, ReviewDecision, Roots, Selection,
};

/// The tool session agent review names `openDiff`'s proposals by, until the hooks (through the
/// relay) name Gemini's own sessions.
pub(crate) const SESSION: &str = "Gemini CLI";

/// A review title: "Gemini CLI: <file name>", at most 256 bytes.
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

fn warn(message: String) {
    log(LogLevel::Warn, &message);
}

/// Offers [`companion::tools`] on the `ide` listener.
pub(crate) fn register_tools() {
    let tools: Vec<listeners::ToolDef> = companion::tools()
        .into_iter()
        .map(|tool| listeners::ToolDef::new(tool.name, tool.description, tool.input_schema))
        .collect();
    if let Err(error) = listeners::register_tools(companion::LISTENER, &tools) {
        warn(format!(
            "the IDE tools were not registered: {}",
            error.message
        ));
    }
}

/// Starts the editor feed that [`context_update`] reads.
pub(crate) fn watch_context() {
    if let Err(error) = context::watch(true) {
        warn(format!("the editor context is off: {}", error.message));
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

/// `review-decided`'s outcome for [`companion::Server::review_decided`].
pub(crate) fn decision(
    outcome: &Result<review::ReviewOutcome, String>,
) -> Option<(ReviewDecision, &str)> {
    let outcome = outcome.as_ref().ok()?;
    let decision = match outcome.decision {
        review::Decision::Accepted => ReviewDecision::Accepted,
        review::Decision::Partial => ReviewDecision::Partial,
        review::Decision::Rejected => ReviewDecision::Rejected,
    };
    Some((decision, &outcome.final_content))
}

/// `ide/contextUpdate`'s params for the editor's context. The feed orders the open files, most
/// recently focused first, and has no times, so each file's `timestamp` is its rank (the
/// newest is the largest), which is all the CLI reads it for. The caret is the selection's end.
pub(crate) fn context_update(editor: context::EditorContext) -> String {
    let mut open: Vec<FileRef> = editor.open.into_iter().map(file_ref).collect();
    if let Some(active) = editor.active.map(file_ref) {
        open.retain(|file| *file != active);
        open.insert(0, active);
    }
    let count = open.len() as u64;
    let files: Vec<OpenFile> = open
        .into_iter()
        .enumerate()
        .map(|(rank, file)| OpenFile {
            file,
            focused_at: count - rank as u64,
        })
        .collect();
    let selection = editor.selection.map(|(range, text)| Selection {
        line: range.end.line,
        column: range.end.column,
        text,
    });
    companion::context_update(&files, selection.as_ref(), editor.trusted, &ContextRoots)
}

/// The host's worktrees. A path the host refuses (outside every worktree, or private) maps to
/// nothing, and Gemini's call is refused as outside.
struct ContextRoots;

impl Roots for ContextRoots {
    fn to_ref(&self, absolute: &str) -> Option<FileRef> {
        context::to_file_ref(absolute).ok().map(file_ref)
    }

    fn to_absolute(&self, file: &FileRef) -> Option<String> {
        context::absolute_path(&sdk_file_ref(file)).ok()
    }
}

/// The host, through the SDK.
pub(crate) struct SdkHost;

impl CompanionHost for SdkHost {
    fn roots(&self) -> &dyn Roots {
        &ContextRoots
    }

    fn respond(&mut self, call: u64, outcome: Result<String, McpError>) {
        let result = match &outcome {
            Ok(result) => listeners::respond(call, Ok(result)),
            Err(error) => listeners::respond(
                call,
                Err(&listeners::McpError::new(error.code, error.message.clone())),
            ),
        };
        if let Err(error) = result {
            warn(format!(
                "the answer to call {call} was refused: {}",
                error.message
            ));
        }
    }

    fn notify(&mut self, notify: Notify) {
        if let Err(error) = listeners::notify(
            companion::LISTENER,
            Some(notify.connection),
            notify.method,
            &notify.params,
        ) {
            warn(format!("{} was not sent: {}", notify.method, error.message));
        }
    }

    fn broadcast(&mut self, method: &'static str, params: &str) {
        if let Err(error) = listeners::notify(companion::LISTENER, None, method, params) {
            warn(format!("{method} was not sent: {}", error.message));
        }
    }

    fn propose(&mut self, file: &FileRef, new_content: &str) -> Result<u64, String> {
        let proposal =
            review::Proposal::new_content(sdk_file_ref(file), new_content, title(file), SESSION);
        review::propose(&proposal).map_err(|error| {
            warn(format!(
                "openDiff of {:?} is not in review: {}",
                file.path, error.message
            ));
            error.message
        })
    }

    /// An outcome on its way is no error: the close came first.
    fn withdraw(&mut self, proposal: u64) -> Option<String> {
        review::withdraw(proposal).ok()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use ui_ext_sdk::context::{ContextEffect, EditorContext, set_worktree_roots};
    use ui_ext_sdk::listeners::{ListenerEffect, McpCall};
    use ui_ext_sdk::review::{Decision, ReviewEffect, ReviewOutcome};
    use ui_ext_sdk::{Extension as _, HostEffect, IoEvent, take_host_effects};

    use crate::{GeminiCli, companion};

    fn call(call_id: u64, connection: u64, tool: &str, arguments: Value) -> IoEvent {
        IoEvent::McpCall(McpCall {
            call_id,
            listener: companion::LISTENER.into(),
            connection,
            tool: tool.into(),
            arguments: arguments.to_string(),
        })
    }

    /// The answers and notifications among `effects`: (call or method, connection, JSON).
    fn sent(effects: Vec<HostEffect>) -> Vec<(String, Option<u64>, Value)> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                HostEffect::Listener(ListenerEffect::Respond { call_id, outcome }) => Some((
                    call_id.to_string(),
                    None,
                    serde_json::from_str(&outcome.ok()?).ok()?,
                )),
                HostEffect::Listener(ListenerEffect::Notify {
                    method,
                    connection,
                    params,
                    ..
                }) => Some((method, connection, serde_json::from_str(&params).ok()?)),
                _ => None,
            })
            .collect()
    }

    fn proposed(effects: &[HostEffect]) -> Vec<u64> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Review(ReviewEffect::Propose { id, .. }) => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_tools_are_registered_and_the_context_watched() {
        take_host_effects();
        GeminiCli::new();
        let effects = take_host_effects();
        let [
            HostEffect::Listener(ListenerEffect::RegisterTools { listener, tools }),
            HostEffect::Context(ContextEffect::Watch(true)),
        ] = effects.as_slice()
        else {
            panic!("{effects:?}");
        };
        assert_eq!(listener, companion::LISTENER);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(names, ["openDiff", "closeDiff"]);
    }

    #[test]
    fn diffs_are_reviews_and_decisions_notify_their_connection() {
        set_worktree_roots(&[(1, "/work/app")]);
        let mut extension = GeminiCli::new();
        take_host_effects();
        let open = |extension: &mut GeminiCli, call_id, connection, text: &str| {
            extension
                .handle_io(call(
                    call_id,
                    connection,
                    "openDiff",
                    serde_json::json!({ "filePath": "/work/app/a.py", "newContent": text }),
                ))
                .unwrap();
            take_host_effects()
        };

        let effects = open(&mut extension, 1, 7, "one\n");
        let [first] = proposed(&effects)[..] else {
            panic!("{effects:?}");
        };
        assert_eq!(
            sent(effects),
            [("1".to_owned(), None, serde_json::json!({ "content": [] }))]
        );

        // A second openDiff for the file replaces the first, without a notification.
        let effects = open(&mut extension, 2, 7, "two\n");
        assert!(
            effects.contains(&HostEffect::Review(ReviewEffect::Withdraw(first))),
            "{effects:?}"
        );
        let [second] = proposed(&effects)[..] else {
            panic!("{effects:?}");
        };
        extension
            .handle_io(IoEvent::ReviewDecided((
                first,
                Ok(ReviewOutcome {
                    decision: Decision::Accepted,
                    final_content: "one\n".into(),
                }),
            )))
            .unwrap();
        assert!(sent(take_host_effects()).is_empty(), "the replaced diff");

        extension
            .handle_io(IoEvent::ReviewDecided((
                second,
                Ok(ReviewOutcome {
                    decision: Decision::Partial,
                    final_content: "kept\n".into(),
                }),
            )))
            .unwrap();
        assert_eq!(
            sent(take_host_effects()),
            [(
                "ide/diffAccepted".to_owned(),
                Some(7),
                serde_json::json!({ "filePath": "/work/app/a.py", "content": "kept\n" })
            )]
        );

        // closeDiff withdraws and returns the text; an undecided end is a rejection.
        let effects = open(&mut extension, 3, 8, "three\n");
        let [third] = proposed(&effects)[..] else {
            panic!("{effects:?}");
        };
        extension
            .handle_io(call(
                4,
                8,
                "closeDiff",
                serde_json::json!({ "filePath": "/work/app/a.py" }),
            ))
            .unwrap();
        let effects = take_host_effects();
        assert!(effects.contains(&HostEffect::Review(ReviewEffect::Withdraw(third))));
        let closed = sent(effects);
        let text = closed[0].2["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(text).unwrap()["content"],
            "three\n"
        );
        assert_eq!(closed[1].0, "ide/diffRejected");

        let effects = open(&mut extension, 5, 8, "four\n");
        let [fourth] = proposed(&effects)[..] else {
            panic!("{effects:?}");
        };
        extension
            .handle_io(IoEvent::ReviewDecided((fourth, Err("gone".into()))))
            .unwrap();
        assert_eq!(sent(take_host_effects())[0].0, "ide/diffRejected");

        // Outside every worktree: refused as invalid params.
        extension
            .handle_io(call(
                6,
                8,
                "openDiff",
                serde_json::json!({ "filePath": "/etc/hosts", "newContent": "" }),
            ))
            .unwrap();
        let effects = take_host_effects();
        assert!(proposed(&effects).is_empty());
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HostEffect::Listener(ListenerEffect::Respond {
                call_id: 6,
                outcome: Err(error)
            }) if error.code == companion::INVALID_PARAMS
        )));
        set_worktree_roots(&[]);
    }

    #[test]
    fn the_editor_feed_is_the_context_update() {
        set_worktree_roots(&[(1, "/work/app")]);
        let mut extension = GeminiCli::new();
        take_host_effects();
        let file = |path: &str| ui_ext_sdk::FileRef {
            worktree: Some(1),
            path: path.into(),
        };
        let editor = EditorContext {
            active: Some(file("src/app.py")),
            open: vec![file("src/app.py"), file("README.md")],
            selection: Some((
                ui_ext_sdk::TextRange {
                    start: ui_ext_sdk::Position { line: 0, column: 4 },
                    end: ui_ext_sdk::Position { line: 0, column: 9 },
                },
                Some("greet".into()),
            )),
            trusted: Some(true),
        };
        extension
            .handle_io(IoEvent::ContextChanged(editor.clone()))
            .unwrap();
        let update = sent(take_host_effects());
        let [(method, None, params)] = update.as_slice() else {
            panic!("{update:?}");
        };
        assert_eq!(method, "ide/contextUpdate");
        assert_eq!(
            *params,
            serde_json::json!({ "workspaceState": {
                "openFiles": [
                    { "path": "/work/app/src/app.py", "timestamp": 2, "isActive": true,
                      "cursor": { "line": 1, "character": 10 }, "selectedText": "greet" },
                    { "path": "/work/app/README.md", "timestamp": 1 },
                ],
                "isTrusted": true,
            }})
        );
        // The same context is not sent again.
        extension
            .handle_io(IoEvent::ContextChanged(editor))
            .unwrap();
        assert!(sent(take_host_effects()).is_empty());
        set_worktree_roots(&[]);
    }
}
