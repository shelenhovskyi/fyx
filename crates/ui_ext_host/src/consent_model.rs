//! The 0.4 consent model (docs/fyx/wit-0.4/design.md §C.1–§C.5) as the consent UI uses it:
//! W2c's types from `ui_ext_wasm::permissions` and `ui_ext_wasm::journal`, and how the host
//! words a diff and a journal entry. The broker is `crate::consent::ConsentStore` (`install`,
//! `set_item`, `item_state`, `diff`); the journal file is `crate::journal::JournalStore`.

pub use ui_ext_wasm::journal::{JournalEntry, JournalKind, JournalPage};
pub use ui_ext_wasm::permissions::{
    ConsentDiff, ConsentItem, ItemLine, Tier as ConsentTier, Widened, consent_diff_lines,
    consent_items,
};

/// What the dialog and the page draw per line.
pub trait ItemLineExt {
    /// Whether the user ticks it (T2); T0, T1 and T3 lines have no checkbox.
    fn has_checkbox(&self) -> bool;
}

impl ItemLineExt for ItemLine {
    fn has_checkbox(&self) -> bool {
        self.tier == ConsentTier::T2
    }
}

/// The lines an update adds or widens, in that order (§C.5).
pub fn new_lines(diff: &ConsentDiff) -> Vec<ItemLine> {
    diff.added
        .iter()
        .cloned()
        .chain(diff.widened.iter().map(|widened| widened.line.clone()))
        .collect()
}

/// One sentence for a held update's card: "New in this version: Runs claude; Connects to the
/// internet (evil.example.com). Removed: Shows approval cards."
pub fn diff_text(diff: &ConsentDiff) -> String {
    let describe = |line: &ItemLine, details: &[String]| {
        if details.is_empty() {
            line.title.clone()
        } else {
            format!("{} ({})", line.title, details.join(", "))
        }
    };
    let mut parts = Vec::new();
    let new: Vec<String> = diff
        .added
        .iter()
        .map(|line| describe(line, &line.details))
        .chain(
            diff.widened
                .iter()
                .map(|widened| describe(&widened.line, &widened.new_details)),
        )
        .collect();
    if !new.is_empty() {
        parts.push(format!("New in this version: {}.", new.join("; ")));
    }
    if !diff.removed.is_empty() {
        let removed: Vec<String> = diff.removed.iter().map(|line| line.title.clone()).collect();
        parts.push(format!("Removed: {}.", removed.join("; ")));
    }
    parts.join(" ")
}

/// When an entry happened. The journal is a file on disk: a time past what `SystemTime` holds (a
/// damaged or hand-edited line) reads as the epoch rather than panicking while the page renders.
pub fn entry_time(entry: &JournalEntry) -> std::time::SystemTime {
    std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_millis(entry.at_ms))
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// How the extension page words an entry.
pub fn journal_line(kind: &JournalKind) -> String {
    match kind {
        JournalKind::ProgramStarted {
            program, args, cwd, ..
        } => {
            let mut line = format!("Ran {program}");
            for arg in args {
                line.push(' ');
                line.push_str(arg);
            }
            if let Some(cwd) = cwd {
                line.push_str(&format!(" in {cwd}"));
            }
            line
        }
        JournalKind::ProgramExited { program, code } => match code {
            Some(code) => format!("{program} exited with {code}"),
            None => format!("{program} ended"),
        },
        JournalKind::TerminalRead {
            terminal,
            scope,
            bytes,
        } => format!("Read {scope} of the terminal \"{terminal}\" ({bytes} bytes)"),
        JournalKind::TerminalWrite {
            terminal,
            text,
            submit,
        } => format!(
            "Typed into the terminal \"{terminal}\": {text}{}",
            if *submit { " (and pressed Enter)" } else { "" }
        ),
        JournalKind::ListenerOpened { listener, port } => {
            format!("Listened on port {port} ({listener})")
        }
        JournalKind::Connections { listener, count } => {
            format!("{count} connections to {listener} in an hour")
        }
        JournalKind::DiscoveryFile { path, written } => format!(
            "{} the discovery file {path}",
            if *written { "Wrote" } else { "Removed" }
        ),
        JournalKind::Relay { channel, count } => {
            format!("{count} hook requests on {channel} in an hour")
        }
        JournalKind::ProposedEdit { path, outcome } => {
            format!("Proposed an edit to {path}: {outcome}")
        }
        JournalKind::ConsentChanged { item_key, allowed } => format!(
            "{item_key} {}",
            if *allowed { "turned on" } else { "turned off" }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_at(at_ms: u64) -> JournalEntry {
        JournalEntry {
            at_ms,
            kind: JournalKind::TerminalRead {
                terminal: "t".into(),
                scope: "lines:20".into(),
                bytes: 10,
            },
        }
    }

    /// `u64::MAX` ms overflows Windows' `SystemTime` (100 ns ticks in a `u64`); Unix's holds it.
    #[test]
    fn an_entry_time_past_what_system_time_holds_does_not_panic() {
        let far = entry_time(&entry_at(u64::MAX));
        if cfg!(windows) {
            assert_eq!(far, std::time::UNIX_EPOCH);
        }
        assert_eq!(
            entry_time(&entry_at(1_000)),
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1)
        );
    }
}
