//! fyx: tool results of the native agent pass through the host's secret redaction (plan 7.4)
//! before the thread, its database or the model provider see them.

use std::sync::Arc;

use gpui::App;
use language_model::{LanguageModelToolResult, LanguageModelToolResultContent};

/// `result` with every secret value the host put into a process replaced by `secret://name`.
pub(crate) fn redact_tool_result(
    mut result: LanguageModelToolResult,
    cx: &App,
) -> LanguageModelToolResult {
    for content in &mut result.content {
        if let LanguageModelToolResultContent::Text(text) = content
            && let Some(redacted) = acp_thread::fyx_secrets::redact_agent_text(text, cx)
        {
            *text = Arc::from(redacted);
        }
    }
    if let Some(output) = result.output.take() {
        result.output = Some(redact_json(output, cx));
    }
    result
}

fn redact_json(value: serde_json::Value, cx: &App) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(
            acp_thread::fyx_secrets::redact_agent_text(&text, cx).unwrap_or(text),
        ),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|item| redact_json(item, cx))
                .collect(),
        ),
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|(name, item)| (name, redact_json(item, cx)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    use acp_thread::fyx_secrets::{AgentProcess, AgentSecrets, set_agent_secrets};
    use gpui::{Task, TestAppContext};
    use language_model::LanguageModelToolUseId;

    struct FakeSecrets;

    impl AgentSecrets for FakeSecrets {
        fn resolve(&self, process: AgentProcess, _: &mut App) -> Task<AgentProcess> {
            Task::ready(process)
        }

        fn redact(&self, text: &str, _: &App) -> Option<String> {
            text.contains("fake-value-42")
                .then(|| text.replace("fake-value-42", "secret://demo"))
        }
    }

    #[gpui::test]
    fn text_and_raw_output_are_redacted(cx: &mut TestAppContext) {
        cx.update(|cx| set_agent_secrets(Rc::new(FakeSecrets), cx));
        let result = LanguageModelToolResult {
            tool_use_id: LanguageModelToolUseId::from("tool-1"),
            tool_name: Arc::from("fetch"),
            is_error: false,
            content: vec![LanguageModelToolResultContent::Text(Arc::from(
                "token=fake-value-42",
            ))],
            output: Some(serde_json::json!({"lines": ["fake-value-42", 3]})),
        };
        let result = cx.update(|cx| redact_tool_result(result, cx));
        assert_eq!(result.text_contents(), "token=secret://demo");
        assert_eq!(
            result.output,
            Some(serde_json::json!({"lines": ["secret://demo", 3]}))
        );
    }
}
