//! What the Extension Inspector shows about an instance (plan 1.7, spec decision 9): its recent
//! calls with their duration against the budget and push count, the guest's `log` output, and the
//! capability checks host imports made, and the 0.3 operations that ran outside a call (fetches,
//! processes, edits, the agent feed). Bounded ring buffers, written on the extension's call queue
//! and its tokio tasks, and read by the host.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime},
};

/// Entries each buffer keeps; older ones are dropped.
pub const DIAGNOSTICS_CAPACITY: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRecord {
    /// `mount`, `handle-event` or `list-rows`.
    pub kind: &'static str,
    pub surface_id: String,
    pub at: SystemTime,
    /// Wall time, from the call being taken off the queue to its end.
    pub duration: Duration,
    /// The guest's own execution, in epochs of `UI_EPOCH_INTERVAL`: what the budget counts, and
    /// what a tick CPU-share throttle should count (`ticks::TickBudget::record`).
    pub executed: Duration,
    pub budget: Duration,
    pub pushes: usize,
    /// A trap, a time budget or a refused call; the guest's own `Err` replies are not errors here.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRecord {
    pub at: SystemTime,
    pub level: log::Level,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRecord {
    pub at: SystemTime,
    pub capability: String,
    /// `None` when allowed.
    pub denied: Option<String>,
}

/// One 0.3 operation that runs after its call (design.md §0.3): from when it started to its
/// outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoRecord {
    pub at: SystemTime,
    /// `network:fetch`, `process:spawn`, `workspace:edit` or `agent:observe`.
    pub capability: &'static str,
    /// The id the import returned; 0 for the agent feed.
    pub id: u64,
    /// What it does, without secrets: a URL has no query, fragment or user info, a process shows
    /// its program and argument count, an edit its label and file count.
    pub what: String,
    /// `None` while it runs.
    pub outcome: Option<String>,
    /// From the start to the outcome.
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub calls: VecDeque<CallRecord>,
    pub logs: VecDeque<LogRecord>,
    pub capability_checks: VecDeque<CapabilityRecord>,
    pub io: VecDeque<IoRecord>,
    /// Bytes of linear memory the instance holds (plan 7.7): memories only grow, so this is the
    /// most it has used.
    pub memory_bytes: usize,
}

fn push_bounded<T>(queue: &mut VecDeque<T>, item: T) {
    if queue.len() == DIAGNOSTICS_CAPACITY {
        queue.pop_front();
    }
    queue.push_back(item);
}

/// Shared between an instance's store and its [`crate::UiExtension`] handle.
#[derive(Clone, Default)]
pub(crate) struct DiagnosticsBuffer(Arc<Mutex<Diagnostics>>);

impl DiagnosticsBuffer {
    fn update(&self, f: impl FnOnce(&mut Diagnostics)) {
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner));
    }

    pub(crate) fn call(&self, record: CallRecord) {
        self.update(|diagnostics| push_bounded(&mut diagnostics.calls, record));
    }

    pub(crate) fn memory(&self, bytes: usize) {
        self.update(|diagnostics| diagnostics.memory_bytes = bytes);
    }

    pub(crate) fn log(&self, level: log::Level, message: &str) {
        self.update(|diagnostics| {
            push_bounded(
                &mut diagnostics.logs,
                LogRecord {
                    at: SystemTime::now(),
                    level,
                    message: message.to_string(),
                },
            )
        });
    }

    pub(crate) fn capability_check(&self, capability: &str, denied: Option<String>) {
        self.update(|diagnostics| {
            push_bounded(
                &mut diagnostics.capability_checks,
                CapabilityRecord {
                    at: SystemTime::now(),
                    capability: capability.to_string(),
                    denied,
                },
            )
        });
    }

    /// A 0.3 operation started, after its call returned.
    pub(crate) fn io_started(&self, capability: &'static str, id: u64, what: String) {
        self.update(|diagnostics| {
            push_bounded(
                &mut diagnostics.io,
                IoRecord {
                    at: SystemTime::now(),
                    capability,
                    id,
                    what,
                    outcome: None,
                    duration: None,
                },
            )
        });
    }

    /// Its outcome; ignored once the record has left the buffer.
    pub(crate) fn io_finished(&self, capability: &'static str, id: u64, outcome: String) {
        self.update(|diagnostics| {
            if let Some(record) = diagnostics.io.iter_mut().rev().find(|record| {
                record.capability == capability && record.id == id && record.outcome.is_none()
            }) {
                record.duration = Some(record.at.elapsed().unwrap_or_default());
                record.outcome = Some(outcome);
            }
        });
    }

    /// Something that has its outcome at once (the agent feed starting or stopping).
    pub(crate) fn io_note(&self, capability: &'static str, what: String, outcome: String) {
        self.io_started(capability, 0, what);
        self.io_finished(capability, 0, outcome);
    }

    pub(crate) fn snapshot(&self) -> Diagnostics {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
