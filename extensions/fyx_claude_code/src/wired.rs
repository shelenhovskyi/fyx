//! The IDE server on the SDK's 0.4 `listeners` (plan 5.14).

use ui_ext_sdk::{LogLevel, listeners, log};

use crate::ide::{self, Diagnostic, FileRef, IdeHost, McpError, Roots};

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

/// No absolute path maps to a worktree until the SDK has `context.to-file-ref` and
/// `context.absolute-path`.
struct NoRoots;

impl Roots for NoRoots {
    fn to_ref(&self, _absolute: &str) -> Option<FileRef> {
        None
    }

    fn to_absolute(&self, _file: &FileRef) -> Option<String> {
        None
    }
}

/// The host, through the SDK.
pub(crate) struct SdkHost;

impl IdeHost for SdkHost {
    fn roots(&self) -> &dyn Roots {
        &NoRoots
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

    /// The staged 0.4 WIT has no diagnostics source.
    fn diagnostics(&self, _only: Option<&str>) -> Vec<(String, Vec<Diagnostic>)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use ui_ext_sdk::listeners::{ListenerEffect, McpCall};
    use ui_ext_sdk::{Extension as _, HostEffect, IoEvent, take_host_effects};

    use crate::{ClaudeCode, ide};

    #[test]
    fn the_ide_tools_are_registered_and_calls_answered() {
        take_host_effects();
        let mut extension = ClaudeCode::new();
        let effects = take_host_effects();
        let [HostEffect::Listener(ListenerEffect::RegisterTools { listener, tools })] =
            effects.as_slice()
        else {
            panic!("{effects:?}");
        };
        assert_eq!(listener, ide::LISTENER);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        let ours: Vec<&str> = ide::tools().iter().map(|tool| tool.name).collect();
        assert_eq!(names, ours);

        let call = |call_id, tool: &str, arguments: &str| {
            IoEvent::McpCall(McpCall {
                call_id,
                listener: ide::LISTENER.into(),
                connection: 1,
                tool: tool.into(),
                arguments: arguments.into(),
            })
        };
        extension
            .handle_io(call(5, "closeAllDiffTabs", "{}"))
            .unwrap();
        extension
            .handle_io(call(6, "getDiagnostics", "{}"))
            .unwrap();
        extension
            .handle_io(call(7, "openFile", r#"{"filePath":"/work/app/a.rs"}"#))
            .unwrap();
        let outcomes: Vec<(u64, Result<String, i32>)> = take_host_effects()
            .into_iter()
            .map(|effect| match effect {
                HostEffect::Listener(ListenerEffect::Respond { call_id, outcome }) => {
                    (call_id, outcome.map_err(|error| error.code))
                }
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                (5, Ok(ide::text_result("CLOSED_ALL_DIFF_TABS"))),
                (6, Ok(ide::text_result("[]"))),
                (7, Err(ide::INVALID_PARAMS)),
            ]
        );
    }
}
