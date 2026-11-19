//! The usage journal of design wit-0.4 §C.4: what an extension did with its T2 and T3 grants,
//! shown on the extension page. The runtime records through a [`Journal`] the host passes in
//! [`crate::LoadOptions::journal`]; the host's store keeps
//! `<data_dir>/extensions/fyx/journal/<id>.jsonl` (`ui_ext_host::journal::JournalStore`).
//!
//! An entry never holds terminal content: a read records its terminal, scope and size only.

use std::time::Duration;

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
