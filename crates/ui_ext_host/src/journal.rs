//! The usage journal's file store (design wit-0.4 §C.4): `<data_dir>/extensions/fyx/journal/
//! <id>.jsonl`, one [`JournalEntry`] per line, the last [`MAX_JOURNAL_ENTRIES`] or
//! [`MAX_JOURNAL_AGE`], whichever is fewer.
//!
//! [`Journal::record`] never blocks: entries go to one writer thread, which appends them in
//! batches. A write that fails (a full disk) drops its entries and marks the extension's journal
//! incomplete until it is cleared. [`JournalStore::read`] and [`JournalStore::clear`] wait for
//! the writer, so call them on the background executor.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, mpsc},
};

use ui_ext_wasm::journal::{
    Journal, JournalEntry, JournalKind, JournalPage, MAX_JOURNAL_AGE, MAX_JOURNAL_ENTRIES, now_ms,
};

/// Entries a file may hold past [`MAX_JOURNAL_ENTRIES`] before it is rewritten.
const SLACK: usize = MAX_JOURNAL_ENTRIES / 10;

enum Command {
    Record(String, JournalEntry),
    Clear(String, Option<mpsc::Sender<()>>),
    Flush(mpsc::Sender<()>),
}

pub struct JournalStore {
    dir: PathBuf,
    sender: Mutex<mpsc::Sender<Command>>,
    incomplete: Arc<Mutex<HashSet<String>>>,
}

impl JournalStore {
    /// The store in `dir`, with its writer thread (which ends with the store).
    pub fn at(dir: PathBuf) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let incomplete = Arc::new(Mutex::new(HashSet::new()));
        let writer = Writer {
            dir: dir.clone(),
            counts: HashMap::new(),
            incomplete: incomplete.clone(),
        };
        if let Err(error) = std::thread::Builder::new()
            .name("fyx-journal".into())
            .spawn(move || writer.run(receiver))
        {
            log::error!("fyx:ui: no usage journal: {error}");
        }
        Arc::new(Self {
            dir,
            sender: Mutex::new(sender),
            incomplete,
        })
    }

    pub fn default_dir() -> PathBuf {
        paths::data_dir()
            .join("extensions")
            .join("fyx")
            .join("journal")
    }

    fn send(&self, command: Command) -> bool {
        self.sender
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .send(command)
            .is_ok()
    }

    /// The extension's journal, oldest first, after the writer saved what it had. Blocks.
    pub fn read(&self, extension_id: &str) -> JournalPage {
        let (done, wait) = mpsc::channel();
        if self.send(Command::Flush(done)) {
            wait.recv().ok();
        }
        let incomplete = self
            .incomplete
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(extension_id);
        let entries = file_for(&self.dir, extension_id)
            .map(|path| retain(read_entries(&path), now_ms()))
            .unwrap_or_default();
        JournalPage {
            entries,
            incomplete,
        }
    }

    /// Deletes the extension's journal (the page's "Clear", or an uninstall). Blocks.
    pub fn clear(&self, extension_id: &str) {
        let (done, wait) = mpsc::channel();
        if self.send(Command::Clear(extension_id.to_string(), Some(done))) {
            wait.recv().ok();
        }
    }

    /// [`Self::clear`] without waiting, for the main thread.
    pub fn clear_later(&self, extension_id: &str) {
        self.send(Command::Clear(extension_id.to_string(), None));
    }
}

impl Journal for JournalStore {
    fn record(&self, extension_id: &str, kind: JournalKind) {
        self.send(Command::Record(
            extension_id.to_string(),
            JournalEntry {
                at_ms: now_ms(),
                kind,
            },
        ));
    }
}

struct Writer {
    dir: PathBuf,
    /// Lines in each file this writer has touched.
    counts: HashMap<String, usize>,
    incomplete: Arc<Mutex<HashSet<String>>>,
}

impl Writer {
    fn run(mut self, receiver: mpsc::Receiver<Command>) {
        while let Ok(first) = receiver.recv() {
            let mut pending: Vec<(String, Vec<JournalEntry>)> = Vec::new();
            let mut next = Some(first);
            while let Some(command) = next.take() {
                match command {
                    Command::Record(extension_id, entry) => {
                        match pending.iter_mut().find(|(id, _)| *id == extension_id) {
                            Some((_, entries)) => entries.push(entry),
                            None => pending.push((extension_id, vec![entry])),
                        }
                    }
                    Command::Clear(extension_id, done) => {
                        pending.retain(|(id, _)| *id != extension_id);
                        self.clear(&extension_id);
                        if let Some(done) = done {
                            done.send(()).ok();
                        }
                    }
                    Command::Flush(done) => {
                        self.write_all(std::mem::take(&mut pending));
                        done.send(()).ok();
                    }
                }
                next = receiver.try_recv().ok();
            }
            self.write_all(pending);
        }
    }

    fn write_all(&mut self, pending: Vec<(String, Vec<JournalEntry>)>) {
        for (extension_id, entries) in pending {
            self.write(&extension_id, entries);
        }
    }

    fn write(&mut self, extension_id: &str, entries: Vec<JournalEntry>) {
        let Some(path) = file_for(&self.dir, extension_id) else {
            log::error!("fyx:ui: no journal file for the extension id {extension_id:?}");
            return;
        };
        let count = entries.len();
        let mut text = String::new();
        for entry in &entries {
            match serde_json::to_string(entry) {
                Ok(line) => {
                    text.push_str(&line);
                    text.push('\n');
                }
                Err(error) => log::error!("fyx:ui: cannot journal an entry: {error}"),
            }
        }
        let written = fs::create_dir_all(&self.dir).and_then(|()| {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .and_then(|mut file| file.write_all(text.as_bytes()))
        });
        if let Err(error) = written {
            log::error!("fyx:ui: journal of {extension_id} incomplete: {error}");
            self.incomplete
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(extension_id.to_string());
            return;
        }
        let first_touch = !self.counts.contains_key(extension_id);
        let lines = self
            .counts
            .entry(extension_id.to_string())
            .or_insert_with(|| count_lines(&path).saturating_sub(count));
        *lines += count;
        // The first write of a session also drops entries past their age.
        if first_touch || *lines > MAX_JOURNAL_ENTRIES + SLACK {
            *lines = compact(&path);
        }
    }

    fn clear(&mut self, extension_id: &str) {
        self.counts.remove(extension_id);
        self.incomplete
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(extension_id);
        if let Some(path) = file_for(&self.dir, extension_id)
            && let Err(error) = fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("fyx:ui: cannot clear the journal of {extension_id}: {error}");
        }
    }
}

/// `<dir>/<id>.jsonl`, for an id that is a plain file name.
fn file_for(dir: &Path, extension_id: &str) -> Option<PathBuf> {
    let plain = !extension_id.is_empty()
        && !extension_id.starts_with('.')
        && extension_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    plain.then(|| dir.join(format!("{extension_id}.jsonl")))
}

fn count_lines(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

/// The file's entries; a line that does not parse (cut off by a crash) is skipped.
fn read_entries(path: &Path) -> Vec<JournalEntry> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The entries the journal keeps at `now_ms`: none past [`MAX_JOURNAL_AGE`], then the newest
/// [`MAX_JOURNAL_ENTRIES`].
fn retain(mut entries: Vec<JournalEntry>, now_ms: u64) -> Vec<JournalEntry> {
    let oldest = now_ms.saturating_sub(MAX_JOURNAL_AGE.as_millis() as u64);
    entries.retain(|entry| entry.at_ms >= oldest);
    let excess = entries.len().saturating_sub(MAX_JOURNAL_ENTRIES);
    entries.drain(..excess);
    entries
}

/// Rewrites the file with what [`retain`] keeps, and returns how many lines it holds.
fn compact(path: &Path) -> usize {
    let entries = read_entries(path);
    let total = count_lines(path);
    let kept = retain(entries, now_ms());
    if kept.len() == total {
        return total;
    }
    let mut text = String::new();
    for entry in &kept {
        if let Ok(line) = serde_json::to_string(entry) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    let temporary = path.with_extension("jsonl.tmp");
    let rewritten = fs::write(&temporary, text).and_then(|()| fs::rename(&temporary, path));
    if let Err(error) = rewritten {
        log::error!("fyx:ui: cannot trim {}: {error}", path.display());
        return total;
    }
    kept.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_at(kind: &str, at_ms: u64) -> JournalEntry {
        JournalEntry {
            at_ms,
            kind: JournalKind::TerminalRead {
                terminal: kind.into(),
                scope: "lines:20".into(),
                bytes: 10,
            },
        }
    }

    /// §C.4: entries survive in the file in order, a read sees what was recorded just before it,
    /// clear deletes the file, and ids that are not plain file names get no file.
    #[test]
    fn test_record_read_clear() {
        let dir = tempfile::tempdir().unwrap();
        let store = JournalStore::at(dir.path().to_path_buf());
        for index in 0..5 {
            store.record(
                "ext",
                JournalKind::ProgramStarted {
                    capability: "process:spawn".into(),
                    program: format!("p{index}"),
                    args: vec![],
                    cwd: None,
                },
            );
        }
        store.record(
            "other",
            JournalKind::ConsentChanged {
                item_key: "network:fetch".into(),
                allowed: true,
            },
        );
        let page = store.read("ext");
        assert!(!page.incomplete);
        let programs: Vec<String> = page
            .entries
            .iter()
            .map(|entry| match &entry.kind {
                JournalKind::ProgramStarted { program, .. } => program.clone(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(programs, ["p0", "p1", "p2", "p3", "p4"]);
        assert_eq!(store.read("other").entries.len(), 1);
        assert!(dir.path().join("ext.jsonl").exists());

        store.clear("ext");
        assert!(!dir.path().join("ext.jsonl").exists());
        assert!(store.read("ext").entries.is_empty());
        assert_eq!(store.read("other").entries.len(), 1, "others stay");

        store.record(
            "../escape",
            JournalKind::ConsentChanged {
                item_key: "x".into(),
                allowed: false,
            },
        );
        assert!(store.read("../escape").entries.is_empty());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    /// The journal keeps the newest 2000 entries and none older than 30 days, in the file too.
    #[test]
    fn test_limits() {
        let now = now_ms();
        let day = 24 * 60 * 60 * 1000;
        let mut entries = vec![read_at("old", now - 31 * day)];
        entries.extend((0..MAX_JOURNAL_ENTRIES + 5).map(|index| read_at(&index.to_string(), now)));
        let kept = retain(entries, now);
        assert_eq!(kept.len(), MAX_JOURNAL_ENTRIES);
        assert!(matches!(
            &kept[0].kind,
            JournalKind::TerminalRead { terminal, .. } if terminal == "5"
        ));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ext.jsonl");
        let mut text = serde_json::to_string(&read_at("old", now - 31 * day)).unwrap() + "\n";
        text.push_str(&(serde_json::to_string(&read_at("new", now)).unwrap() + "\n"));
        fs::write(&path, text).unwrap();
        let store = JournalStore::at(dir.path().to_path_buf());
        for _ in 0..MAX_JOURNAL_ENTRIES + SLACK + 1 {
            store.record(
                "ext",
                JournalKind::Relay {
                    channel: "hooks".into(),
                    count: 1,
                },
            );
        }
        let page = store.read("ext");
        assert_eq!(page.entries.len(), MAX_JOURNAL_ENTRIES);
        assert!(
            count_lines(&path) <= MAX_JOURNAL_ENTRIES + SLACK,
            "trimmed on disk"
        );
        assert!(
            !fs::read_to_string(&path).unwrap().contains("\"old\""),
            "the old entry left on the first write"
        );
    }

    /// §C.4: a terminal read never holds the terminal's content, only which terminal, the scope
    /// and the size.
    #[test]
    fn test_reads_hold_no_content() {
        let entry = read_at("Terminal 1", 1);
        let value = serde_json::to_value(&entry).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(keys, ["at_ms", "bytes", "kind", "scope", "terminal"]);
    }

    /// A write that fails drops its entries and marks the journal incomplete until cleared.
    #[test]
    fn test_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the directory should be: every write fails.
        let blocked = dir.path().join("journal");
        fs::write(&blocked, "").unwrap();
        let store = JournalStore::at(blocked);
        store.record(
            "ext",
            JournalKind::ConsentChanged {
                item_key: "x".into(),
                allowed: true,
            },
        );
        let page = store.read("ext");
        assert!(page.incomplete);
        assert!(page.entries.is_empty());
        store.clear("ext");
        assert!(!store.read("ext").incomplete);
    }
}
