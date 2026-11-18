//! fyx: hook that lets the host review an agent's file write hunk by hunk before it lands.
//!
//! The gate is installed by `agent_review` (a Fyx crate that depends on this one). Without a gate,
//! writes behave exactly as upstream.

use std::{path::PathBuf, rc::Rc};

use agent_client_protocol::v2::schema as acp_v2;
use anyhow::Result;
use gpui::{App, AsyncApp, Entity, Global, Task, WeakEntity};
use language::{Buffer, BufferSnapshot};

use crate::AcpThread;

/// A write the agent asked for, before anything was applied to the buffer.
pub struct AgentWrite {
    pub session_id: acp_v2::SessionId,
    pub abs_path: PathBuf,
    pub buffer: Entity<Buffer>,
    pub old_text: String,
    pub new_text: String,
}

/// The host's decision about an [`AgentWrite`].
pub struct AgentWriteOutcome {
    /// The text to write, or `None` to leave the file untouched.
    pub content: Option<String>,
    /// Reported to the agent as an error once the write finished, so it never assumes rejected hunks
    /// landed.
    pub feedback: Option<String>,
}

/// A file operation of an agent other than writing text: Zed's built-in agent deletes, moves and
/// copies files and directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentFileOperation {
    /// `text` is the file's content, when it is a text file.
    Delete {
        path: PathBuf,
        text: Option<String>,
    },
    Move {
        from: PathBuf,
        to: PathBuf,
    },
    Copy {
        from: PathBuf,
        to: PathBuf,
    },
}

impl AgentFileOperation {
    /// Every path the operation reads or writes.
    pub fn paths(&self) -> Vec<&std::path::Path> {
        match self {
            AgentFileOperation::Delete { path, .. } => vec![path],
            AgentFileOperation::Move { from, to } | AgentFileOperation::Copy { from, to } => {
                vec![from, to]
            }
        }
    }

    pub fn describe(&self) -> String {
        match self {
            AgentFileOperation::Delete { path, .. } => format!("Delete {}", path.display()),
            AgentFileOperation::Move { from, to } => {
                format!("Move {} to {}", from.display(), to.display())
            }
            AgentFileOperation::Copy { from, to } => {
                format!("Copy {} to {}", from.display(), to.display())
            }
        }
    }
}

pub trait AgentWriteGate: 'static {
    /// Whether any write may need review now. `false` (review mode off and no project restriction)
    /// lets every write through untouched, without copying the buffer, as upstream.
    fn wants_review(&self, _cx: &App) -> bool {
        true
    }

    fn review_write(&self, write: AgentWrite, cx: &mut App) -> Task<Result<AgentWriteOutcome>>;

    /// Notes for the agent that no answer could carry (a permission answer has no message), taken
    /// once: they go in front of the user's next prompt in the session.
    fn take_prompt_notes(&self, _session_id: &acp_v2::SessionId, _cx: &mut App) -> Option<String> {
        None
    }

    /// Resolves to an error, whose message is the feedback, when the operation must not happen.
    fn review_operation(
        &self,
        _session_id: acp_v2::SessionId,
        _operation: AgentFileOperation,
        _cx: &mut App,
    ) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }
}

struct GlobalAgentWriteGate(Rc<dyn AgentWriteGate>);

impl Global for GlobalAgentWriteGate {}

pub fn set_agent_write_gate(gate: Rc<dyn AgentWriteGate>, cx: &mut App) {
    cx.set_global(GlobalAgentWriteGate(gate));
}

/// Whether a host review gate is installed (outside Fyx, or in tests, there is none).
pub fn has_agent_write_gate(cx: &App) -> bool {
    cx.has_global::<GlobalAgentWriteGate>()
}

/// Whether agent changes may need host review now: a gate is installed and wants them.
pub fn wants_agent_review(cx: &App) -> bool {
    cx.try_global::<GlobalAgentWriteGate>()
        .is_some_and(|gate| gate.0.wants_review(cx))
}

pub(crate) async fn gate_agent_write(
    thread: &WeakEntity<AcpThread>,
    abs_path: PathBuf,
    buffer: &Entity<Buffer>,
    snapshot: &BufferSnapshot,
    new_text: String,
    cx: &mut AsyncApp,
) -> Result<(String, Option<String>)> {
    if !cx.update(|cx| wants_agent_review(cx)) {
        return Ok((new_text, None));
    }
    let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone())?;
    review_agent_edit(session_id, abs_path, buffer, snapshot.text(), new_text, cx).await
}

/// The user's prompt, with the host review's pending notes for the agent in front of it.
pub fn with_review_notes(
    session_id: &acp_v2::SessionId,
    mut message: Vec<acp_v2::ContentBlock>,
    cx: &mut App,
) -> Vec<acp_v2::ContentBlock> {
    let gate = cx
        .try_global::<GlobalAgentWriteGate>()
        .map(|gate| gate.0.clone());
    if let Some(notes) = gate.and_then(|gate| gate.take_prompt_notes(session_id, cx)) {
        message.insert(
            0,
            acp_v2::ContentBlock::Text(acp_v2::TextContent::new(notes)),
        );
    }
    message
}

/// Reviews an agent's file operation; without a gate it may happen, as upstream.
pub async fn review_agent_operation(
    session_id: acp_v2::SessionId,
    operation: AgentFileOperation,
    cx: &mut AsyncApp,
) -> Result<()> {
    let gate = cx.update(|cx| {
        cx.try_global::<GlobalAgentWriteGate>()
            .map(|gate| gate.0.clone())
    });
    let Some(gate) = gate else {
        return Ok(());
    };
    cx.update(|cx| gate.review_operation(session_id, operation, cx))
        .await
        .map_err(|error| feedback_error(format!("{error:#}")))
}

/// Reviews an agent's change of `buffer` from `old_text` to `new_text` (ACP writes, and the native
/// agent's edit sessions). Returns the text to keep and feedback for the agent, or an error, whose
/// message is the feedback, when nothing may be written: the review rejected it, was closed, or the
/// buffer changed while the review was open.
pub async fn review_agent_edit(
    session_id: acp_v2::SessionId,
    abs_path: PathBuf,
    buffer: &Entity<Buffer>,
    old_text: String,
    new_text: String,
    cx: &mut AsyncApp,
) -> Result<(String, Option<String>)> {
    let gate = cx.update(|cx| {
        cx.try_global::<GlobalAgentWriteGate>()
            .map(|gate| gate.0.clone())
    });
    let Some(gate) = gate.filter(|gate| cx.update(|cx| gate.wants_review(cx))) else {
        return Ok((new_text, None));
    };
    // The file as the buffer names it, compared after the review: the agent's spelling (`src/../a`,
    // `\\?\C:\a`) may differ from it without the file having moved.
    let (version_before, path_before) = cx.update(|cx| {
        let buffer = buffer.read(cx);
        let path = buffer
            .file()
            .and_then(|file| file.as_local())
            .map(|file| file.abs_path(cx));
        (buffer.version(), path)
    });
    let write = AgentWrite {
        session_id,
        abs_path: abs_path.clone(),
        buffer: buffer.clone(),
        old_text,
        new_text,
    };
    let outcome = cx
        .update(|cx| gate.review_write(write, cx))
        .await
        .map_err(|error| feedback_error(format!("{error:#}")))?;
    let Some(content) = outcome.content else {
        return Err(feedback_error(outcome.feedback.unwrap_or_else(|| {
            "The user rejected this edit; the file is unchanged.".into()
        })));
    };
    // A review can take as long as the user likes. The hunks were decided against the file as it was;
    // if it changed meanwhile (the user, another agent, a reload) or went away, writing them would
    // land changes nobody reviewed.
    cx.update(|cx| {
        let buffer = buffer.read(cx);
        let path = abs_path.display();
        if let Some(file) = buffer.file() {
            if file.disk_state() == language::DiskState::Deleted {
                return Err(feedback_error(format!(
                    "{path} was deleted while your edit was under review, so nothing was written."
                )));
            }
            if let Some(current) = file.as_local().map(|file| file.abs_path(cx))
                && path_before
                    .as_ref()
                    .is_some_and(|before| *before != current)
            {
                return Err(feedback_error(format!(
                    "{path} was moved to {} while your edit was under review, so nothing was \
                     written.",
                    current.display()
                )));
            }
        }
        if buffer.version().changed_since(&version_before) {
            return Err(feedback_error(format!(
                "{path} changed while your edit was under review, so nothing was written. Read \
                 the file again and propose the edit against its current content."
            )));
        }
        Ok(())
    })?;
    Ok((content, outcome.feedback))
}

/// An error that reaches the agent with the review feedback as its JSON-RPC message (and, as for any
/// other error, its data). A plain `anyhow` error would reach it as "Internal error" with the text
/// only in `data`, which many agents never show to the model.
pub(crate) fn feedback_error(feedback: String) -> anyhow::Error {
    anyhow::Error::new(
        agent_client_protocol::Error::new(
            agent_client_protocol::ErrorCode::INTERNAL_ERROR,
            feedback.clone(),
        )
        .data(feedback),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_reaches_the_agent_as_the_error_message() {
        // The conversion `agent_servers` applies to a failed `fs/write_text_file`.
        let error = agent_client_protocol::Error::from(feedback_error("Hunk 2 rejected".into()));
        assert_eq!(error.code, agent_client_protocol::ErrorCode::INTERNAL_ERROR);
        assert_eq!(error.message, "Hunk 2 rejected");
        assert_eq!(error.data, Some(serde_json::json!("Hunk 2 rejected")));
    }
}
