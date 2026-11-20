//! The usage journal of design wit-0.4 §C.4: what an extension did with its T2 and T3 grants,
//! shown on the extension page. The runtime records through a [`Journal`] the host passes in
//! [`crate::LoadOptions::journal`]; the host's store keeps
//! `<data_dir>/extensions/fyx/journal/<id>.jsonl` (`ui_ext_host::journal::JournalStore`).
//!
//! An entry never holds terminal content: a read records its terminal, scope and size only.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// The store keeps at most this many entries per extension, and none older than
/// [`MAX_JOURNAL_AGE`].
pub const MAX_JOURNAL_ENTRIES: usize = 2000;
pub const MAX_JOURNAL_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What happened. Counts per hour (connections, relay requests) are summed by the recorder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum JournalKind {
    /// `process:spawn` or `terminal:create` started a program.
    ProgramStarted {
        capability: String,
        program: String,
        args: Vec<String>,
        cwd: Option<String>,
    },
    ProgramExited {
        program: String,
        code: Option<i32>,
    },
    /// `terminal:read`: which terminal, the scope and the size; never the text.
    TerminalRead {
        terminal: String,
        scope: String,
        bytes: u64,
    },
    /// `terminal:write`: the exact text the user confirmed.
    TerminalWrite {
        terminal: String,
        text: String,
        submit: bool,
    },
    ListenerOpened {
        listener: String,
        port: u16,
    },
    /// Connections a listener accepted in the past hour.
    Connections {
        listener: String,
        count: u32,
    },
    DiscoveryFile {
        path: String,
        written: bool,
    },
    /// Relay requests on a channel in the past hour.
    Relay {
        channel: String,
        count: u32,
    },
    ProposedEdit {
        path: String,
        outcome: String,
    },
    /// `network:fetch` requests to `host`, summed over at most [`USAGE_FLUSH`] (plan 7.6).
    Fetched {
        host: String,
        count: u32,
    },
    /// `workspace:edit` changes applied to a worktree file, summed like [`JournalKind::Fetched`].
    FileEdited {
        path: String,
        count: u32,
    },
    /// A T2 or T3 consent change, by [`crate::permissions::ConsentItem::key`]
    /// (`terminal:read#always` for "Always allow").
    ConsentChanged {
        item_key: String,
        allowed: bool,
    },
}

/// One line of the journal file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    #[serde(flatten)]
    pub kind: JournalKind,
}

/// What the extension page reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JournalPage {
    /// Oldest first.
    pub entries: Vec<JournalEntry>,
    /// Some entries were dropped (a full disk): the page says "Journal incomplete".
    pub incomplete: bool,
}

/// Where the runtime records. It must not block: the store batches writes off the caller's
/// thread.
pub trait Journal: Send + Sync + 'static {
    fn record(&self, extension_id: &str, kind: JournalKind);
}

/// The current time for an entry.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// How long frequent uses are summed before they are recorded (plan 7.6).
pub const USAGE_FLUSH: Duration = Duration::from_secs(60);

#[derive(Default)]
struct PendingUses {
    since: Option<Instant>,
    fetches: BTreeMap<String, u32>,
    edits: BTreeMap<String, u32>,
    /// Hosts and files used before: the first use of each is recorded at once, so the privacy
    /// report shows it without waiting for the minute.
    seen: BTreeSet<(bool, String)>,
}

/// One instance's frequent uses (fetches per host, edits per file). The first use of each host or
/// file is recorded at once; later ones are summed and recorded at most once per [`USAGE_FLUSH`]
/// and when the instance ends, so a busy extension does not push older entries out of the
/// journal.
pub(crate) struct UsageJournal {
    extension_id: Arc<str>,
    journal: Arc<dyn Journal>,
    pending: Mutex<PendingUses>,
}

impl UsageJournal {
    pub(crate) fn new(extension_id: Arc<str>, journal: Arc<dyn Journal>) -> Self {
        Self {
            extension_id,
            journal,
            pending: Mutex::default(),
        }
    }

    pub(crate) fn fetched(&self, host: &str) {
        self.add(true, host);
    }

    pub(crate) fn edited(&self, path: &str) {
        self.add(false, path);
    }

    fn add(&self, fetch: bool, key: &str) {
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.seen.insert((fetch, key.to_string())) {
            drop(pending);
            let key = key.to_string();
            self.journal.record(
                &self.extension_id,
                if fetch {
                    JournalKind::Fetched {
                        host: key,
                        count: 1,
                    }
                } else {
                    JournalKind::FileEdited {
                        path: key,
                        count: 1,
                    }
                },
            );
            return;
        }
        let since = *pending.since.get_or_insert(now);
        let uses = if fetch {
            &mut pending.fetches
        } else {
            &mut pending.edits
        };
        let count = uses.entry(key.to_string()).or_default();
        *count = count.saturating_add(1);
        if now.duration_since(since) >= USAGE_FLUSH {
            let fetches = std::mem::take(&mut pending.fetches);
            let edits = std::mem::take(&mut pending.edits);
            pending.since = None;
            drop(pending);
            self.record(fetches, edits);
        }
    }

    fn record(&self, fetches: BTreeMap<String, u32>, edits: BTreeMap<String, u32>) {
        for (host, count) in fetches {
            self.journal
                .record(&self.extension_id, JournalKind::Fetched { host, count });
        }
        for (path, count) in edits {
            self.journal
                .record(&self.extension_id, JournalKind::FileEdited { path, count });
        }
    }
}

impl Drop for UsageJournal {
    fn drop(&mut self) {
        let pending = std::mem::take(
            self.pending
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner),
        );
        self.record(pending.fetches, pending.edits);
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;

    #[derive(Default)]
    struct Recorded(Mutex<Vec<JournalKind>>);

    impl Journal for Recorded {
        fn record(&self, _: &str, kind: JournalKind) {
            self.0.lock().unwrap().push(kind);
        }
    }

    #[test]
    fn test_usage_is_summed_until_the_end() {
        let recorded = Arc::new(Recorded::default());
        let usage = UsageJournal::new("demo".into(), recorded.clone());
        let fetched = |count| JournalKind::Fetched {
            host: "api.example.com".into(),
            count,
        };
        usage.fetched("api.example.com");
        usage.edited("src/a.rs");
        let first = vec![
            fetched(1),
            JournalKind::FileEdited {
                path: "src/a.rs".into(),
                count: 1,
            },
        ];
        assert_eq!(*recorded.0.lock().unwrap(), first, "the first uses at once");
        usage.fetched("api.example.com");
        usage.fetched("api.example.com");
        assert_eq!(
            *recorded.0.lock().unwrap(),
            first,
            "later ones within the minute"
        );
        drop(usage);
        let mut all = first;
        all.push(fetched(2));
        assert_eq!(*recorded.0.lock().unwrap(), all);
    }
}
