//! fyx: hook that lets the host review an agent's file write hunk by hunk before it lands.
//!
//! The gate is installed by `agent_review` (a Fyx crate that depends on this one). Without a gate,
//! writes behave exactly as upstream.

use std::{path::PathBuf, rc::Rc};

use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Result, anyhow};
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

pub trait AgentWriteGate: 'static {
    fn review_write(&self, write: AgentWrite, cx: &mut App) -> Task<Result<AgentWriteOutcome>>;
}

struct GlobalAgentWriteGate(Rc<dyn AgentWriteGate>);

impl Global for GlobalAgentWriteGate {}

pub fn set_agent_write_gate(gate: Rc<dyn AgentWriteGate>, cx: &mut App) {
    cx.set_global(GlobalAgentWriteGate(gate));
}

pub(crate) async fn gate_agent_write(
    thread: &WeakEntity<AcpThread>,
    abs_path: PathBuf,
    buffer: &Entity<Buffer>,
    snapshot: &BufferSnapshot,
    new_text: String,
    cx: &mut AsyncApp,
) -> Result<(String, Option<String>)> {
    let gate = cx.update(|cx| {
        cx.try_global::<GlobalAgentWriteGate>()
            .map(|gate| gate.0.clone())
    });
    let Some(gate) = gate else {
        return Ok((new_text, None));
    };
    let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone())?;
    let write = AgentWrite {
        session_id,
        abs_path,
        buffer: buffer.clone(),
        old_text: snapshot.text(),
        new_text,
    };
    let outcome = cx.update(|cx| gate.review_write(write, cx)).await?;
    match outcome.content {
        Some(content) => Ok((content, outcome.feedback)),
        None => Err(anyhow!(outcome.feedback.unwrap_or_else(|| {
            "The user rejected this edit; the file is unchanged.".into()
        }))),
    }
}
