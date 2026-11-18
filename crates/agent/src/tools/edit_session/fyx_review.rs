//! fyx: the host review (`crates/agent_review`) for the native agent's edit and write tools.
//!
//! The agent streams its edit into the buffer while it generates. Before the session saves, the
//! whole change goes through the same gate as an ACP `fs/write_text_file`: the buffer keeps only what
//! the review accepted, and the agent is told about rejected hunks. Without a gate (outside Fyx, or
//! with review mode off and no project rule), the session finishes as upstream.

use gpui::{AsyncApp, Entity};
use language::{Buffer, BufferSnapshot};

use super::{EditSession, EditSessionOutput, EditSessionResult};
use crate::ToolCallEventStream;

pub(super) enum Reviewed {
    /// Nothing changed by the review: upstream's flow continues.
    Unchanged(EditSessionResult),
    /// The review changed what lands; the session is finished.
    Done(Result<EditSessionOutput, EditSessionOutput>),
}

pub(super) async fn review(
    result: EditSessionResult,
    event_stream: &ToolCallEventStream,
    cx: &mut AsyncApp,
) -> Reviewed {
    let (session, error) = match result {
        EditSessionResult::Completed(session) => (session, None),
        EditSessionResult::Failed {
            error,
            session: Some(session),
        } => (session, Some(error)),
        result => return Reviewed::Unchanged(result),
    };
    let unchanged = |session: EditSession, error: Option<String>| {
        Reviewed::Unchanged(match error {
            None => EditSessionResult::Completed(session),
            Some(error) => EditSessionResult::Failed {
                error,
                session: Some(session),
            },
        })
    };
    if !cx.update(|cx| acp_thread::fyx_review::wants_agent_review(cx)) {
        return unchanged(session, error);
    }
    let Ok(session_id) = session
        .context
        .thread
        .read_with(cx, |thread, _| thread.id().clone())
    else {
        return unchanged(session, error);
    };
    let snapshot = session.buffer.read_with(cx, |buffer, _| buffer.snapshot());
    let written = snapshot.text();
    if written == *session.old_text {
        return unchanged(session, error);
    }

    let reviewed = acp_thread::fyx_review::review_agent_edit(
        session_id,
        session.abs_path.clone(),
        &session.buffer,
        session.old_text.to_string(),
        written.clone(),
        cx,
    )
    .await;
    let (kept, feedback) = match reviewed {
        Ok((kept, None)) if kept == written => return unchanged(session, error),
        Ok((kept, feedback)) => (kept, feedback),
        // Rejected, closed, or the buffer changed during review: none of the agent's change stays.
        Err(review_error) => (
            session.old_text.to_string(),
            Some(format!("{review_error:#}")),
        ),
    };

    move_buffer(&session.buffer, &snapshot, &written, &kept, cx);
    session.context.action_log.update(cx, |action_log, cx| {
        action_log.buffer_edited(session.buffer.clone(), cx)
    });
    if kept != *session.old_text {
        session
            .context
            .ensure_buffer_saved(&session.buffer, cx)
            .await;
    }
    let (_, diff) = session.compute_new_text_and_diff(cx).await;
    let message = error
        .into_iter()
        .chain(feedback)
        .collect::<Vec<_>>()
        .join("\n");
    event_stream.update_fields(|update| update.content(vec![message.clone().into()]));
    Reviewed::Done(Err(EditSessionOutput::Error {
        error: message,
        input_path: Some(session.input_path.clone()),
        diff,
    }))
}

/// Turns the agent's text into the kept text. The edits are anchored in the agent's text, so
/// anything typed into the buffer during the review stays.
fn move_buffer(
    buffer: &Entity<Buffer>,
    snapshot: &BufferSnapshot,
    from: &str,
    to: &str,
    cx: &mut AsyncApp,
) {
    if from == to {
        return;
    }
    let edits = language::text_diff(from, to)
        .into_iter()
        .map(|(range, text)| (snapshot.anchor_range_inside(range), text))
        .collect::<Vec<_>>();
    buffer.update(cx, |buffer, cx| buffer.edit(edits, None, cx));
}

#[cfg(test)]
mod tests {
    use std::{rc::Rc, sync::Arc};

    use acp_thread::fyx_review::{
        AgentWrite, AgentWriteGate, AgentWriteOutcome, set_agent_write_gate,
    };
    use fs::Fs as _;
    use gpui::{App, AppContext as _, Task, TestAppContext};
    use language_model::LanguageModelRegistry;
    use project::Project;
    use prompt_store::ProjectContext;
    use serde_json::json;
    use settings::SettingsStore;
    use util::path;

    use crate::{
        AgentTool, ContextServerRegistry, Edit, EditFileTool, EditFileToolInput,
        EditFileToolOutput, Templates, ToolCallEventStream, ToolInput,
    };

    const OLD: &str = "line 1\nline 2\nline 3\n";

    /// Stands in for `agent_review`: answers every write the same way.
    struct FakeGate(fn(&AgentWrite) -> AgentWriteOutcome);

    impl AgentWriteGate for FakeGate {
        fn review_write(
            &self,
            write: AgentWrite,
            _: &mut App,
        ) -> Task<anyhow::Result<AgentWriteOutcome>> {
            Task::ready(Ok((self.0)(&write)))
        }
    }

    async fn edit_two_lines(
        gate: fn(&AgentWrite) -> AgentWriteOutcome,
        cx: &mut TestAppContext,
    ) -> (
        Result<EditFileToolOutput, EditFileToolOutput>,
        Arc<project::FakeFs>,
    ) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            set_agent_write_gate(Rc::new(FakeGate(gate)), cx);
        });
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree("/root", json!({ "file.txt": OLD })).await;
        let project = Project::test(fs.clone(), [path!("/root").as_ref()], cx).await;
        let language_registry = project.read_with(cx, |project, _| project.languages().clone());
        let context_server_registry =
            cx.new(|cx| ContextServerRegistry::new(project.read(cx).context_server_store(), cx));
        let model = cx.update(|cx| LanguageModelRegistry::test(cx).model("fake"));
        let thread = cx.new(|cx| {
            crate::Thread::new(
                project.clone(),
                cx.new(|_| ProjectContext::default()),
                context_server_registry,
                Templates::new(),
                Some(model),
                cx,
            )
        });
        let action_log = thread.read_with(cx, |thread, _| thread.action_log().clone());
        let tool = Arc::new(EditFileTool::new(
            project,
            thread.downgrade(),
            action_log,
            language_registry,
        ));
        let result = cx
            .update(|cx| {
                tool.run(
                    ToolInput::resolved(EditFileToolInput {
                        path: "root/file.txt".into(),
                        edits: vec![
                            Edit {
                                old_text: "line 1".into(),
                                new_text: "LINE 1".into(),
                            },
                            Edit {
                                old_text: "line 3".into(),
                                new_text: "LINE 3".into(),
                            },
                        ],
                    }),
                    ToolCallEventStream::test().0,
                    cx,
                )
            })
            .await;
        (result, fs)
    }

    #[gpui::test]
    async fn a_rejected_edit_is_not_saved(cx: &mut TestAppContext) {
        let (result, fs) = edit_two_lines(
            |_| AgentWriteOutcome {
                content: None,
                feedback: Some("The user rejected your edit; the file is unchanged.".into()),
            },
            cx,
        )
        .await;
        let Err(EditFileToolOutput::Error { error, .. }) = result else {
            panic!("a rejected edit must fail");
        };
        assert!(error.contains("rejected"), "{error}");
        let on_disk = fs.load(path!("/root/file.txt").as_ref()).await.unwrap();
        assert_eq!(on_disk, OLD);
    }

    #[gpui::test]
    async fn only_accepted_hunks_are_saved(cx: &mut TestAppContext) {
        let (result, fs) = edit_two_lines(
            |write| {
                assert_eq!(write.old_text, OLD);
                assert_eq!(write.new_text, "LINE 1\nline 2\nLINE 3\n");
                AgentWriteOutcome {
                    content: Some("LINE 1\nline 2\nline 3\n".into()),
                    feedback: Some("Hunk 2 (line 3) rejected: keep it".into()),
                }
            },
            cx,
        )
        .await;
        let Err(EditFileToolOutput::Error { error, diff, .. }) = result else {
            panic!("a partly accepted edit reports the rejected hunks");
        };
        assert!(error.contains("Hunk 2"), "{error}");
        assert!(
            diff.contains("+LINE 1") && !diff.contains("+LINE 3"),
            "{diff}"
        );
        let on_disk = fs.load(path!("/root/file.txt").as_ref()).await.unwrap();
        assert_eq!(on_disk, "LINE 1\nline 2\nline 3\n");
    }
}
