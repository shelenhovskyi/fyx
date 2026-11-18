//! fyx: the host review (`crates/agent_review`) for the built-in agent's delete, move and copy
//! tools. Each tool calls [`review`] once, after its own authorization and before the operation;
//! a rejection stops the tool with the reason.

use std::path::PathBuf;

use gpui::{AsyncApp, Entity};
use project::Project;

use crate::ToolCallEventStream;
use acp_thread::fyx_review::AgentFileOperation;

pub(crate) async fn delete(
    project: &Entity<Project>,
    event_stream: &ToolCallEventStream,
    path: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    review(project, event_stream, Operation::Delete(path), cx).await
}

pub(crate) async fn move_path(
    project: &Entity<Project>,
    event_stream: &ToolCallEventStream,
    from: &str,
    to: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    review(project, event_stream, Operation::Move(from, to), cx).await
}

pub(crate) async fn copy_path(
    project: &Entity<Project>,
    event_stream: &ToolCallEventStream,
    from: &str,
    to: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    review(project, event_stream, Operation::Copy(from, to), cx).await
}

/// A tool's operation, with its paths as the tool received them (project paths).
enum Operation<'a> {
    Delete(&'a str),
    Move(&'a str, &'a str),
    Copy(&'a str, &'a str),
}

/// Holds the operation for the host review; `Err` carries what the agent is told.
async fn review(
    project: &Entity<Project>,
    event_stream: &ToolCallEventStream,
    operation: Operation<'_>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    if !cx.update(|cx| acp_thread::fyx_review::wants_agent_review(cx)) {
        return Ok(());
    }
    // Operations outside a thread (none in the app) have no session to review them in.
    let Some(session_id) = event_stream
        .owning_thread()
        .and_then(|thread| thread.read_with(cx, |thread, _| thread.id().clone()).ok())
    else {
        return Ok(());
    };
    review_in_session(project, session_id, operation, cx).await
}

async fn review_in_session(
    project: &Entity<Project>,
    session_id: agent_client_protocol::v2::schema::SessionId,
    operation: Operation<'_>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let resolve = |path: &str, cx: &mut AsyncApp| {
        project.read_with(cx, |project, cx| {
            project
                .find_project_path(path, cx)
                .and_then(|project_path| project.absolute_path(&project_path, cx))
                .unwrap_or_else(|| PathBuf::from(path))
        })
    };
    let operation = match operation {
        Operation::Delete(path) => {
            let path = resolve(path, cx);
            let fs = project.read_with(cx, |project, _| project.fs().clone());
            let is_file = fs
                .metadata(&path)
                .await
                .ok()
                .flatten()
                .is_some_and(|metadata| !metadata.is_dir);
            let text = if is_file {
                fs.load(&path).await.ok()
            } else {
                None
            };
            AgentFileOperation::Delete { path, text }
        }
        Operation::Move(from, to) => AgentFileOperation::Move {
            from: resolve(from, cx),
            to: resolve(to, cx),
        },
        Operation::Copy(from, to) => AgentFileOperation::Copy {
            from: resolve(from, cx),
            to: resolve(to, cx),
        },
    };
    acp_thread::fyx_review::review_agent_operation(session_id, operation, cx)
        .await
        .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::Path, rc::Rc};

    use acp_thread::fyx_review::{
        AgentFileOperation, AgentWrite, AgentWriteGate, AgentWriteOutcome, set_agent_write_gate,
    };
    use agent_client_protocol::v2::schema::SessionId;
    use gpui::{App, Task, TestAppContext};
    use project::Project;
    use serde_json::json;
    use settings::SettingsStore;
    use util::path;

    use super::{Operation, review_in_session};

    /// Stands in for `agent_review`: records operations and rejects them.
    struct RejectingGate(Rc<RefCell<Vec<AgentFileOperation>>>);

    impl AgentWriteGate for RejectingGate {
        fn review_write(
            &self,
            _: AgentWrite,
            _: &mut App,
        ) -> Task<anyhow::Result<AgentWriteOutcome>> {
            Task::ready(Err(anyhow::anyhow!("not used")))
        }

        fn review_operation(
            &self,
            _: SessionId,
            operation: AgentFileOperation,
            _: &mut App,
        ) -> Task<anyhow::Result<()>> {
            self.0.borrow_mut().push(operation);
            Task::ready(Err(anyhow::anyhow!("The user rejected this.")))
        }
    }

    #[gpui::test]
    async fn operations_reach_the_gate_with_absolute_paths(cx: &mut TestAppContext) {
        let seen = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            set_agent_write_gate(Rc::new(RejectingGate(seen.clone())), cx);
        });
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            "/root",
            json!({ "a.txt": "keep me
", "dir": {} }),
        )
        .await;
        let project = Project::test(fs, [path!("/root").as_ref()], cx).await;
        let session = SessionId::new("session");
        let mut async_cx = cx.to_async();

        let error = review_in_session(
            &project,
            session.clone(),
            Operation::Delete("root/a.txt"),
            &mut async_cx,
        )
        .await
        .unwrap_err();
        assert!(error.contains("rejected"), "{error}");
        review_in_session(
            &project,
            session.clone(),
            Operation::Delete("root/dir"),
            &mut async_cx,
        )
        .await
        .unwrap_err();
        review_in_session(
            &project,
            session,
            Operation::Move("root/a.txt", "root/b.txt"),
            &mut async_cx,
        )
        .await
        .unwrap_err();

        let root = Path::new(path!("/root"));
        assert_eq!(
            *seen.borrow(),
            [
                AgentFileOperation::Delete {
                    path: root.join("a.txt"),
                    text: Some(
                        "keep me
"
                        .into()
                    ),
                },
                AgentFileOperation::Delete {
                    path: root.join("dir"),
                    text: None,
                },
                AgentFileOperation::Move {
                    from: root.join("a.txt"),
                    to: root.join("b.txt"),
                },
            ]
        );
    }
}
