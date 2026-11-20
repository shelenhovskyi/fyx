//! fyx:ui 0.4 diagnostics from extensions (`diagnostics:publish`, T1, design wit-0.4 §E, plan
//! 6.6): the limits and checks of `context.publish-diagnostics` and `clear-diagnostics`, and the
//! host side the runtime hands the checked entries to.
//!
//! The runtime resolves and checks the path (a worktree file, not private) first, then
//! [`PublishedDiagnostics`] checks the rate and the counts and cleans the entries. The host
//! (`ui_ext_host`, W3) maps them onto the project's diagnostics with one `LanguageServerId` per
//! instance and names the extension as their source.

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use crate::{
    context::{Diagnostic, MAX_DIAGNOSTIC_LABEL_BYTES, MAX_DIAGNOSTIC_MESSAGE_BYTES, cut_to},
    process::FileRef,
    v0_2::{HostError, HostErrorCode},
};

pub const CAPABILITY_DIAGNOSTICS_PUBLISH: &str = "diagnostics:publish";
/// §E.1: entries of one file; more is `invalid-argument` and changes nothing.
pub const MAX_PUBLISHED_PER_FILE: usize = 500;
/// §E.1: entries of one extension across its files.
pub const MAX_PUBLISHED_PER_EXTENSION: usize = 5_000;
/// §E.1: `publish-diagnostics` and `clear-diagnostics` calls per second; more is `unavailable`.
pub const MAX_PUBLISHES_PER_SECOND: usize = 20;
const RATE_WINDOW: Duration = Duration::from_secs(1);

/// The editor side of `diagnostics:publish` (implemented in `ui_ext_host`). Each call names a
/// worktree file that passed the runtime's checks; the entries are checked and cut, with
/// `source` `None` (the host sets it to the extension's display name, §E.1).
pub trait DiagnosticsPublisher: Send + Sync + 'static {
    /// Replaces the extension's diagnostics of `path` (its `worktree` always named) with
    /// `diagnostics`; an empty list clears the file. A range past the end of the file is the
    /// host's to clamp. The error is shown to the extension as `unavailable`.
    fn publish(
        &self,
        extension_id: &str,
        path: FileRef,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<(), String>;
    /// Clears every file the extension published: `clear-diagnostics(none)`, and when the
    /// instance stops, crashes, reloads or loses `diagnostics:publish`.
    fn clear_all(&self, extension_id: &str);
    /// Zed's `private_files` (`is_private_path`): such a file gets no diagnostics.
    fn is_private(&self, absolute: &std::path::Path) -> bool;
}

/// One instance's published diagnostics, as counts per file, and its recent calls.
#[derive(Debug, Default)]
pub struct PublishedDiagnostics {
    /// `(worktree, path)` to the number of entries; files without any are not kept.
    files: BTreeMap<(u64, String), usize>,
    calls: VecDeque<Instant>,
}

impl PublishedDiagnostics {
    /// `publish-diagnostics` of `path` (resolved: its `worktree` named): checks the rate, the
    /// ranges and the counts, then cleans `diagnostics` in place (`source` dropped, message and
    /// code cut) and records the new count. Nothing is recorded on an error.
    pub fn publish(
        &mut self,
        path: &FileRef,
        diagnostics: &mut [Diagnostic],
        now: Instant,
    ) -> Result<(), HostError> {
        self.check_rate(now)?;
        if diagnostics.len() > MAX_PUBLISHED_PER_FILE {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("at most {MAX_PUBLISHED_PER_FILE} diagnostics per file"),
            ));
        }
        let key = key(path);
        let others: usize = self
            .files
            .iter()
            .filter(|(file, _)| **file != key)
            .map(|(_, count)| count)
            .sum();
        if others + diagnostics.len() > MAX_PUBLISHED_PER_EXTENSION {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("at most {MAX_PUBLISHED_PER_EXTENSION} diagnostics per extension"),
            ));
        }
        for entry in diagnostics.iter() {
            let start = (entry.range.start.line, entry.range.start.column);
            let end = (entry.range.end.line, entry.range.end.column);
            if end < start {
                return Err(error(
                    HostErrorCode::InvalidArgument,
                    "a diagnostic's range ends before it starts",
                ));
            }
        }
        for entry in diagnostics.iter_mut() {
            entry.source = None;
            cut_to(&mut entry.message, MAX_DIAGNOSTIC_MESSAGE_BYTES);
            if let Some(code) = &mut entry.code {
                cut_to(code, MAX_DIAGNOSTIC_LABEL_BYTES);
            }
        }
        self.calls.push_back(now);
        if diagnostics.is_empty() {
            self.files.remove(&key);
        } else {
            self.files.insert(key, diagnostics.len());
        }
        Ok(())
    }

    /// `clear-diagnostics(none)`: counts as a call; afterwards nothing is published.
    pub fn clear_all(&mut self, now: Instant) -> Result<(), HostError> {
        self.check_rate(now)?;
        self.calls.push_back(now);
        self.files.clear();
        Ok(())
    }

    /// The instance stopped or lost the capability: nothing is published any more.
    pub fn forget(&mut self) {
        self.files.clear();
    }

    /// Whether any file has entries now.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The entries published now, across files.
    pub fn len(&self) -> usize {
        self.files.values().sum()
    }

    fn check_rate(&mut self, now: Instant) -> Result<(), HostError> {
        while let Some(&oldest) = self.calls.front() {
            if now.saturating_duration_since(oldest) < RATE_WINDOW {
                break;
            }
            self.calls.pop_front();
        }
        if self.calls.len() >= MAX_PUBLISHES_PER_SECOND {
            return Err(error(
                HostErrorCode::Unavailable,
                format!(
                    "at most {MAX_PUBLISHES_PER_SECOND} diagnostics calls a second; retry later"
                ),
            ));
        }
        Ok(())
    }
}

fn key(path: &FileRef) -> (u64, String) {
    (path.worktree.unwrap_or_default(), path.path.clone())
}

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DiagnosticSeverity;
    use crate::edit::{Position, TextRange};

    fn file(path: &str) -> FileRef {
        FileRef {
            worktree: Some(1),
            path: path.into(),
        }
    }

    fn entry(line: u32) -> Diagnostic {
        Diagnostic {
            range: TextRange {
                start: Position { line, column: 0 },
                end: Position { line, column: 4 },
            },
            severity: DiagnosticSeverity::Warning,
            message: "TODO".into(),
            source: Some("rust-analyzer".into()),
            code: Some("x".repeat(MAX_DIAGNOSTIC_LABEL_BYTES + 1)),
        }
    }

    /// §E.1, §E.4: `source` is dropped, the per-file and per-extension caps leave the old
    /// entries, an empty list clears, and the rate is per second.
    #[test]
    fn publishes_are_checked_and_counted() {
        let mut published = PublishedDiagnostics::default();
        let start = Instant::now();
        let mut entries = vec![entry(0), entry(3)];
        published
            .publish(&file("src/a.rs"), &mut entries, start)
            .unwrap();
        assert_eq!(entries[0].source, None);
        assert_eq!(
            entries[0].code.as_deref().map(str::len),
            Some(MAX_DIAGNOSTIC_LABEL_BYTES)
        );
        assert_eq!(published.len(), 2);

        let mut too_many = vec![entry(0); MAX_PUBLISHED_PER_FILE + 1];
        let refused = published
            .publish(&file("src/a.rs"), &mut too_many, start)
            .unwrap_err();
        assert_eq!(refused.code, HostErrorCode::InvalidArgument);
        assert_eq!(published.len(), 2);

        let mut backwards = vec![entry(2)];
        backwards[0].range.end = Position { line: 1, column: 0 };
        assert_eq!(
            published
                .publish(&file("src/b.rs"), &mut backwards, start)
                .unwrap_err()
                .code,
            HostErrorCode::InvalidArgument
        );

        // Replacing a file's entries does not count them twice.
        let mut full = vec![entry(0); MAX_PUBLISHED_PER_FILE];
        published
            .publish(&file("src/a.rs"), &mut full, start)
            .unwrap();
        assert_eq!(published.len(), MAX_PUBLISHED_PER_FILE);

        published
            .publish(&file("src/a.rs"), &mut [], start)
            .unwrap();
        assert!(published.is_empty());

        // 20 calls a second: the 21st within the second is `unavailable`, a second later fine.
        let mut published = PublishedDiagnostics::default();
        for _ in 0..MAX_PUBLISHES_PER_SECOND {
            published.clear_all(start).unwrap();
        }
        assert_eq!(
            published.clear_all(start).unwrap_err().code,
            HostErrorCode::Unavailable
        );
        published
            .clear_all(start + Duration::from_millis(1001))
            .unwrap();
    }

    /// The per-extension cap counts every other file.
    #[test]
    fn the_extension_cap_spans_files() {
        let mut published = PublishedDiagnostics::default();
        let mut now = Instant::now();
        for index in 0..(MAX_PUBLISHED_PER_EXTENSION / MAX_PUBLISHED_PER_FILE) {
            let mut full = vec![entry(0); MAX_PUBLISHED_PER_FILE];
            published
                .publish(&file(&format!("{index}.rs")), &mut full, now)
                .unwrap();
            now += Duration::from_secs(1);
        }
        assert_eq!(
            published
                .publish(&file("one-more.rs"), &mut [entry(0)], now)
                .unwrap_err()
                .code,
            HostErrorCode::InvalidArgument
        );
        published.forget();
        assert!(published.is_empty());
    }
}
