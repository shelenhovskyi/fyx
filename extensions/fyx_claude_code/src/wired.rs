//! The IDE server on the SDK's 0.4 `listeners` and `context` (plan 5.14): Claude's absolute
//! paths map to worktree files through `context.to-file-ref` and `context.absolute-path`,
//! `openFile` is `context.open`, and the editor feed becomes `selection_changed`.

use ui_ext_sdk::{LogLevel, context, listeners, log};

use crate::ide::{self, Diagnostic, FileRef, IdeHost, McpError, Position, Roots, TextRange};

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

/// Sends `params` to every connection of the `ide` listener.
pub(crate) fn notify_selection(params: &str) {
    if let Err(error) = listeners::notify(ide::LISTENER, None, "selection_changed", params) {
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

    /// The staged 0.4 WIT has no diagnostics source yet (a `context` read is planned).
    fn diagnostics(&self, _only: Option<&str>) -> Vec<(String, Vec<Diagnostic>)> {
        Vec::new()
    }

    fn open(&mut self, file: &FileRef, focus: bool) -> Result<(), String> {
        context::open(&sdk_file_ref(file), None, focus).map_err(|error| error.message)
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
        set_worktree_roots(&[]);
    }
}
