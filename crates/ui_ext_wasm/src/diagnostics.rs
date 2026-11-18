//! What the Extension Inspector shows about an instance (plan 1.7, spec decision 9): its recent
//! calls with their duration against the budget and push count, the guest's `log` output, and the
//! capability checks host imports made. Bounded ring buffers, written on the extension's call
//! queue and read by the host.

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics {
    pub calls: VecDeque<CallRecord>,
    pub logs: VecDeque<LogRecord>,
    pub capability_checks: VecDeque<CapabilityRecord>,
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

    pub(crate) fn snapshot(&self) -> Diagnostics {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
