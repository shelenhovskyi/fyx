//! Fyx local history core (plan 6.3, design `docs/fyx/platform/local-history.md`). Synchronous
//! and UI-free: the host calls it off the main thread on every save and for every file an agent
//! turn touched, and shows [`History::file_timeline`] and [`History::changes`] in its timeline.
//!
//! One store per worktree ([`store_dir`]): an append-only `index.jsonl` and content-addressed
//! blobs. A restore is itself a recorded change whose snapshots hold what was on disk before it,
//! so every restore can be undone the same way an agent turn is.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// `schema` in the first line of an index this build understands.
pub const INDEX_SCHEMA: &str = "fyx-history/1";
const INDEX_FILE: &str = "index.jsonl";
const BLOBS_DIR: &str = "blobs";
const TEMPORARY_SUFFIX: &str = ".fyx-tmp";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0:?} is not a relative path inside the worktree")]
    PathOutsideWorktree(PathBuf),
    #[error("there is no snapshot {0}")]
    UnknownSnapshot(u64),
    #[error("there is no agent turn {0}")]
    UnknownTurn(u64),
    #[error("there is no change {0}")]
    UnknownChange(u64),
    #[error("the history index is for schema {0:?}, not {INDEX_SCHEMA}")]
    UnknownSchema(String),
    #[error("the stored content {0} is missing or damaged")]
    DamagedBlob(String),
    #[error("these files changed since, restore them anyway to overwrite: {0:?}")]
    Conflicts(Vec<String>),
    #[error("these files were too large to keep, so they cannot be restored: {0:?}")]
    NotStored(Vec<String>),
    #[error("could not {action} {path:?}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn io_error<'path>(
    action: &'static str,
    path: &'path Path,
) -> impl FnOnce(io::Error) -> Error + 'path {
    move |source| Error::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

/// How much history a store keeps. [`History::prune`] applies the first two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Total size of the distinct contents kept.
    pub max_total_bytes: u64,
    /// Snapshots older than this are dropped.
    pub max_age_ms: u64,
    /// A larger file is recorded by hash only ([`Content::TooLarge`]) and cannot be restored.
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_total_bytes: 256 * 1024 * 1024,
            max_age_ms: 30 * 24 * 60 * 60 * 1000,
            max_file_bytes: 8 * 1024 * 1024,
        }
    }
}

/// Where the store for `worktree` lives under the history root (`<data dir>/history`): one folder
/// per worktree, named by a hash of its path so the path itself never appears in a file name.
pub fn store_dir(history_root: &Path, worktree: &Path) -> PathBuf {
    let digest = sha256_hex(worktree.to_string_lossy().as_bytes());
    history_root.join(&digest[..16])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(u64);

impl SnapshotId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// An agent turn or a restore: the files it changed are undone together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChangeId(u64);

impl ChangeId {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// A file's content at one point in time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Content {
    /// The file did not exist.
    Missing,
    Stored {
        sha256: String,
        len: u64,
    },
    /// Above [`Limits::max_file_bytes`]: the hash still shows whether the file changed since.
    TooLarge {
        sha256: String,
        len: u64,
    },
}

impl Content {
    fn sha256(&self) -> Option<&str> {
        match self {
            Content::Missing => None,
            Content::Stored { sha256, .. } | Content::TooLarge { sha256, .. } => Some(sha256),
        }
    }

    fn matches(&self, bytes: Option<&[u8]>) -> bool {
        match bytes {
            None => *self == Content::Missing,
            Some(bytes) => self.sha256() == Some(sha256_hex(bytes).as_str()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ChangeKind {
    /// One agent turn. `session` is the agent's session id, `label` what the timeline shows.
    AgentTurn {
        session: String,
        label: Option<String>,
    },
    Restore {
        of: RestoreSource,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreSource {
    /// Undo a whole change: every file it touched goes back to what it was before.
    Change(ChangeId),
    /// Put one file back to the content of a snapshot.
    Snapshot(SnapshotId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub id: ChangeId,
    pub time_ms: u64,
    #[serde(flatten)]
    pub kind: ChangeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "kebab-case")]
pub enum Origin {
    Save,
    Change { change: ChangeId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub time_ms: u64,
    /// Relative to the worktree, `/`-separated.
    pub path: String,
    #[serde(flatten)]
    pub origin: Origin,
    /// For a change: the content before it. A save has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Content>,
    pub content: Content,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "kebab-case")]
enum Entry {
    Header { schema: String },
    Change(Change),
    Snapshot(Snapshot),
}

/// One file of a change: what it was before the change's first edit and after its last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub before: Content,
    pub after: Content,
}

/// A change with its files, for the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSummary {
    pub change: Change,
    pub files: Vec<ChangedFile>,
}

/// What a restore would do, for the timeline to show before the user confirms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePlan {
    pub source: RestoreSource,
    /// The content each file gets.
    pub writes: Vec<(String, Content)>,
    /// Files whose content on disk is no longer what the change left: restoring overwrites
    /// someone's later edit, so [`History::restore`] needs `overwrite_conflicts`.
    pub conflicts: Vec<String>,
    /// Files whose earlier content was too large to keep: the restore is refused.
    pub not_stored: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub removed_snapshots: usize,
    pub removed_blobs: usize,
}

pub struct History {
    store: PathBuf,
    worktree: PathBuf,
    limits: Limits,
    changes: Vec<Change>,
    snapshots: Vec<Snapshot>,
    next_id: u64,
    skipped_lines: usize,
}

impl History {
    /// Opens or creates the store. Lines that do not parse (a crash during an append) are
    /// skipped and counted in [`History::skipped_lines`]; the next prune drops them.
    pub fn open(store: &Path, worktree: &Path, limits: Limits) -> Result<Self, Error> {
        fs::create_dir_all(store.join(BLOBS_DIR)).map_err(io_error("create", store))?;
        let mut history = Self {
            store: store.to_path_buf(),
            worktree: worktree.to_path_buf(),
            limits,
            changes: Vec::new(),
            snapshots: Vec::new(),
            next_id: 1,
            skipped_lines: 0,
        };
        let index = history.index_path();
        let text = match fs::read_to_string(&index) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(io_error("read", &index)(error)),
        };
        if !text.is_empty() && !text.ends_with('\n') {
            // A torn last line: end it, so the next entry starts on a line of its own.
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(&index)
                .map_err(io_error("open", &index))?;
            file.write_all(b"\n")
                .map_err(io_error("append to", &index))?;
        }
        let mut lines = text.lines().filter(|line| !line.trim().is_empty());
        match lines.next() {
            None => history.append(&Entry::Header {
                schema: INDEX_SCHEMA.to_string(),
            })?,
            Some(first) => match serde_json::from_str::<Entry>(first) {
                Ok(Entry::Header { schema }) if schema == INDEX_SCHEMA => {}
                Ok(Entry::Header { schema }) => return Err(Error::UnknownSchema(schema)),
                _ => return Err(Error::UnknownSchema(String::new())),
            },
        }
        for line in lines {
            match serde_json::from_str::<Entry>(line) {
                Ok(Entry::Change(change)) => {
                    history.next_id = history.next_id.max(change.id.0 + 1);
                    history.changes.push(change);
                }
                Ok(Entry::Snapshot(snapshot)) => {
                    history.next_id = history.next_id.max(snapshot.id.0 + 1);
                    history.snapshots.push(snapshot);
                }
                Ok(Entry::Header { .. }) | Err(_) => history.skipped_lines += 1,
            }
        }
        Ok(history)
    }

    pub fn skipped_lines(&self) -> usize {
        self.skipped_lines
    }

    /// Records a saved file. Returns `None` when the content is the file's latest snapshot.
    pub fn record_save(
        &mut self,
        path: &Path,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<Option<SnapshotId>, Error> {
        let path = relative_path(path)?;
        let content = self.store_blob(bytes)?;
        let latest = self
            .snapshots
            .iter()
            .rev()
            .find(|snapshot| snapshot.path == path);
        if latest.is_some_and(|snapshot| snapshot.content == content) {
            return Ok(None);
        }
        self.push_snapshot(path, Origin::Save, None, content, now_ms)
            .map(Some)
    }

    /// Starts an agent turn; its files follow with [`History::record_turn_file`].
    pub fn begin_turn(
        &mut self,
        session: &str,
        label: Option<&str>,
        now_ms: u64,
    ) -> Result<ChangeId, Error> {
        self.push_change(
            ChangeKind::AgentTurn {
                session: session.to_string(),
                label: label.map(str::to_string),
            },
            now_ms,
        )
    }

    /// Records one file of an agent turn: `None` means the file did not exist (before) or was
    /// deleted (after). Returns `None` when the turn left the file as it was.
    pub fn record_turn_file(
        &mut self,
        turn: ChangeId,
        path: &Path,
        before: Option<&[u8]>,
        after: Option<&[u8]>,
        now_ms: u64,
    ) -> Result<Option<SnapshotId>, Error> {
        let is_turn = self
            .change(turn)
            .is_some_and(|change| matches!(change.kind, ChangeKind::AgentTurn { .. }));
        if !is_turn {
            return Err(Error::UnknownTurn(turn.0));
        }
        let path = relative_path(path)?;
        let before = self.store_optional(before)?;
        let after = self.store_optional(after)?;
        if before == after {
            return Ok(None);
        }
        self.push_snapshot(
            path,
            Origin::Change { change: turn },
            Some(before),
            after,
            now_ms,
        )
        .map(Some)
    }

    /// The snapshots of one file, newest first.
    pub fn file_timeline(&self, path: &Path) -> Result<Vec<&Snapshot>, Error> {
        let path = relative_path(path)?;
        Ok(self
            .snapshots
            .iter()
            .rev()
            .filter(|snapshot| snapshot.path == path)
            .collect())
    }

    /// Agent turns and restores with their files, newest first.
    pub fn changes(&self) -> Vec<ChangeSummary> {
        self.changes
            .iter()
            .rev()
            .map(|change| ChangeSummary {
                change: change.clone(),
                files: self.changed_files(change.id),
            })
            .collect()
    }

    pub fn snapshot(&self, id: SnapshotId) -> Option<&Snapshot> {
        self.snapshots.iter().find(|snapshot| snapshot.id == id)
    }

    /// The bytes of a stored content; `None` for [`Content::Missing`].
    pub fn read(&self, content: &Content) -> Result<Option<Vec<u8>>, Error> {
        match content {
            Content::Missing => Ok(None),
            Content::TooLarge { sha256, .. } => Err(Error::DamagedBlob(sha256.clone())),
            Content::Stored { sha256, .. } => {
                let path = self.blob_path(sha256);
                let bytes = fs::read(&path).map_err(|_| Error::DamagedBlob(sha256.clone()))?;
                if sha256_hex(&bytes) != *sha256 {
                    return Err(Error::DamagedBlob(sha256.clone()));
                }
                Ok(Some(bytes))
            }
        }
    }

    pub fn plan(&self, source: RestoreSource) -> Result<RestorePlan, Error> {
        let mut plan = RestorePlan {
            source,
            writes: Vec::new(),
            conflicts: Vec::new(),
            not_stored: Vec::new(),
        };
        match source {
            RestoreSource::Change(id) => {
                if self.change(id).is_none() {
                    return Err(Error::UnknownChange(id.0));
                }
                for file in self.changed_files(id) {
                    let current = self.read_worktree_file(&file.path)?;
                    if !file.after.matches(current.as_deref()) {
                        plan.conflicts.push(file.path.clone());
                    }
                    if matches!(file.before, Content::TooLarge { .. }) {
                        plan.not_stored.push(file.path.clone());
                    }
                    plan.writes.push((file.path, file.before));
                }
            }
            RestoreSource::Snapshot(id) => {
                let snapshot = self.snapshot(id).ok_or(Error::UnknownSnapshot(id.0))?;
                if matches!(snapshot.content, Content::TooLarge { .. }) {
                    plan.not_stored.push(snapshot.path.clone());
                }
                plan.writes
                    .push((snapshot.path.clone(), snapshot.content.clone()));
            }
        }
        Ok(plan)
    }

    /// Restores in one step and records the restore as a change, so it can be undone with
    /// `restore(RestoreSource::Change(<returned id>), ..)`. Refused when a file changed since and
    /// `overwrite_conflicts` is false, or when an earlier content was too large to keep. If a
    /// write fails midway, the files written so far stay recorded under the returned change.
    pub fn restore(
        &mut self,
        source: RestoreSource,
        overwrite_conflicts: bool,
        now_ms: u64,
    ) -> Result<ChangeId, Error> {
        let plan = self.plan(source)?;
        if !plan.not_stored.is_empty() {
            return Err(Error::NotStored(plan.not_stored));
        }
        if !plan.conflicts.is_empty() && !overwrite_conflicts {
            return Err(Error::Conflicts(plan.conflicts));
        }
        let mut targets = Vec::with_capacity(plan.writes.len());
        for (path, content) in plan.writes {
            let bytes = self.read(&content)?;
            targets.push((path, content, bytes));
        }
        let restore = self.push_change(ChangeKind::Restore { of: source }, now_ms)?;
        for (path, content, bytes) in targets {
            let current = self.read_worktree_file(&path)?;
            let before = self.store_optional(current.as_deref())?;
            if before == content {
                continue;
            }
            self.write_worktree_file(&path, bytes.as_deref())?;
            self.push_snapshot(
                path,
                Origin::Change { change: restore },
                Some(before),
                content,
                now_ms,
            )?;
        }
        Ok(restore)
    }

    /// Total size of the distinct contents the history refers to.
    pub fn stored_bytes(&self) -> u64 {
        self.referenced_blobs().values().sum()
    }

    /// Drops snapshots older than [`Limits::max_age_ms`], then the oldest until the store fits
    /// [`Limits::max_total_bytes`]. A change is dropped with all its files, never in part, and the
    /// newest save or change is always kept. Rewrites the index and deletes unreferenced blobs.
    pub fn prune(&mut self, now_ms: u64) -> Result<PruneReport, Error> {
        let oldest_kept = now_ms.saturating_sub(self.limits.max_age_ms);
        let mut units = self.units();
        units.sort_by_key(|unit| (unit.time_ms, unit.order));
        let newest = units.last().map(|unit| unit.key);
        let mut dropped: HashSet<UnitKey> = units
            .iter()
            .filter(|unit| unit.time_ms < oldest_kept && Some(unit.key) != newest)
            .map(|unit| unit.key)
            .collect();
        let mut blob_sizes = self.unit_blobs(&dropped);
        for unit in &units {
            if blob_sizes.values().sum::<u64>() <= self.limits.max_total_bytes {
                break;
            }
            if Some(unit.key) == newest || !dropped.insert(unit.key) {
                continue;
            }
            blob_sizes = self.unit_blobs(&dropped);
        }

        let snapshots_before = self.snapshots.len();
        self.snapshots
            .retain(|snapshot| !dropped.contains(&unit_of(snapshot)));
        self.changes
            .retain(|change| !dropped.contains(&UnitKey::Change(change.id)));
        let removed_snapshots = snapshots_before - self.snapshots.len();
        if removed_snapshots > 0 || !dropped.is_empty() || self.skipped_lines > 0 {
            self.rewrite_index()?;
            self.skipped_lines = 0;
        }
        let removed_blobs = self.collect_garbage()?;
        Ok(PruneReport {
            removed_snapshots,
            removed_blobs,
        })
    }

    fn change(&self, id: ChangeId) -> Option<&Change> {
        self.changes.iter().find(|change| change.id == id)
    }

    fn changed_files(&self, id: ChangeId) -> Vec<ChangedFile> {
        let mut files: BTreeMap<&str, ChangedFile> = BTreeMap::new();
        for snapshot in &self.snapshots {
            if snapshot.origin != (Origin::Change { change: id }) {
                continue;
            }
            let before = snapshot.before.clone().unwrap_or(Content::Missing);
            files
                .entry(snapshot.path.as_str())
                .and_modify(|file| file.after = snapshot.content.clone())
                .or_insert_with(|| ChangedFile {
                    path: snapshot.path.clone(),
                    before,
                    after: snapshot.content.clone(),
                });
        }
        files.into_values().collect()
    }

    fn units(&self) -> Vec<Unit> {
        let mut units: HashMap<UnitKey, Unit> = HashMap::new();
        for change in &self.changes {
            let key = UnitKey::Change(change.id);
            units.insert(
                key,
                Unit {
                    key,
                    time_ms: change.time_ms,
                    order: change.id.0,
                },
            );
        }
        for snapshot in &self.snapshots {
            let key = unit_of(snapshot);
            let unit = units.entry(key).or_insert(Unit {
                key,
                time_ms: snapshot.time_ms,
                order: snapshot.id.0,
            });
            unit.time_ms = unit.time_ms.max(snapshot.time_ms);
            unit.order = unit.order.max(snapshot.id.0);
        }
        units.into_values().collect()
    }

    /// The distinct blobs kept when the `dropped` units are gone.
    fn unit_blobs(&self, dropped: &HashSet<UnitKey>) -> HashMap<String, u64> {
        let mut blobs = HashMap::new();
        for snapshot in &self.snapshots {
            if dropped.contains(&unit_of(snapshot)) {
                continue;
            }
            for content in snapshot.before.iter().chain([&snapshot.content]) {
                if let Content::Stored { sha256, len } = content {
                    blobs.insert(sha256.clone(), *len);
                }
            }
        }
        blobs
    }

    fn referenced_blobs(&self) -> HashMap<String, u64> {
        self.unit_blobs(&HashSet::new())
    }

    fn collect_garbage(&self) -> Result<usize, Error> {
        let referenced = self.referenced_blobs();
        let blobs = self.store.join(BLOBS_DIR);
        let mut removed = 0;
        for shard in fs::read_dir(&blobs).map_err(io_error("list", &blobs))? {
            let shard = shard.map_err(io_error("list", &blobs))?.path();
            if !shard.is_dir() {
                continue;
            }
            for blob in fs::read_dir(&shard).map_err(io_error("list", &shard))? {
                let blob = blob.map_err(io_error("list", &shard))?.path();
                let name = blob
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if !referenced.contains_key(&name) {
                    fs::remove_file(&blob).map_err(io_error("delete", &blob))?;
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    fn push_change(&mut self, kind: ChangeKind, now_ms: u64) -> Result<ChangeId, Error> {
        let change = Change {
            id: ChangeId(self.take_id()),
            time_ms: now_ms,
            kind,
        };
        self.append(&Entry::Change(change.clone()))?;
        let id = change.id;
        self.changes.push(change);
        Ok(id)
    }

    fn push_snapshot(
        &mut self,
        path: String,
        origin: Origin,
        before: Option<Content>,
        content: Content,
        now_ms: u64,
    ) -> Result<SnapshotId, Error> {
        let snapshot = Snapshot {
            id: SnapshotId(self.take_id()),
            time_ms: now_ms,
            path,
            origin,
            before,
            content,
        };
        self.append(&Entry::Snapshot(snapshot.clone()))?;
        let id = snapshot.id;
        self.snapshots.push(snapshot);
        Ok(id)
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn index_path(&self) -> PathBuf {
        self.store.join(INDEX_FILE)
    }

    fn append(&self, entry: &Entry) -> Result<(), Error> {
        let index = self.index_path();
        let mut line = serde_json::to_string(entry).map_err(|error| Error::Io {
            action: "encode an entry for",
            path: index.clone(),
            source: io::Error::other(error),
        })?;
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&index)
            .map_err(io_error("open", &index))?;
        file.write_all(line.as_bytes())
            .map_err(io_error("append to", &index))
    }

    fn rewrite_index(&self) -> Result<(), Error> {
        let index = self.index_path();
        let mut text = String::new();
        let header = Entry::Header {
            schema: INDEX_SCHEMA.to_string(),
        };
        let mut entries: Vec<(u64, Entry)> = self
            .changes
            .iter()
            .map(|change| (change.id.0, Entry::Change(change.clone())))
            .chain(
                self.snapshots
                    .iter()
                    .map(|snapshot| (snapshot.id.0, Entry::Snapshot(snapshot.clone()))),
            )
            .collect();
        entries.sort_by_key(|(id, _)| *id);
        for entry in std::iter::once(&header).chain(entries.iter().map(|(_, entry)| entry)) {
            let line = serde_json::to_string(entry).map_err(|error| Error::Io {
                action: "encode an entry for",
                path: index.clone(),
                source: io::Error::other(error),
            })?;
            text.push_str(&line);
            text.push('\n');
        }
        write_atomically(&index, text.as_bytes())
    }

    fn blob_path(&self, sha256: &str) -> PathBuf {
        let shard = sha256.get(..2).unwrap_or("00");
        self.store.join(BLOBS_DIR).join(shard).join(sha256)
    }

    fn store_blob(&self, bytes: &[u8]) -> Result<Content, Error> {
        let sha256 = sha256_hex(bytes);
        let len = bytes.len() as u64;
        if len > self.limits.max_file_bytes {
            return Ok(Content::TooLarge { sha256, len });
        }
        let path = self.blob_path(&sha256);
        if !path.exists() {
            if let Some(shard) = path.parent() {
                fs::create_dir_all(shard).map_err(io_error("create", shard))?;
            }
            write_atomically(&path, bytes)?;
        }
        Ok(Content::Stored { sha256, len })
    }

    fn store_optional(&self, bytes: Option<&[u8]>) -> Result<Content, Error> {
        match bytes {
            None => Ok(Content::Missing),
            Some(bytes) => self.store_blob(bytes),
        }
    }

    fn worktree_path(&self, relative: &str) -> PathBuf {
        let mut path = self.worktree.clone();
        path.extend(relative.split('/'));
        path
    }

    fn read_worktree_file(&self, relative: &str) -> Result<Option<Vec<u8>>, Error> {
        let path = self.worktree_path(relative);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("read", &path)(error)),
        }
    }

    fn write_worktree_file(&self, relative: &str, bytes: Option<&[u8]>) -> Result<(), Error> {
        let path = self.worktree_path(relative);
        match bytes {
            None => match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io_error("delete", &path)(error)),
            },
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(io_error("create", parent))?;
                }
                write_atomically(&path, bytes)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum UnitKey {
    Save(SnapshotId),
    Change(ChangeId),
}

struct Unit {
    key: UnitKey,
    time_ms: u64,
    order: u64,
}

fn unit_of(snapshot: &Snapshot) -> UnitKey {
    match snapshot.origin {
        Origin::Save => UnitKey::Save(snapshot.id),
        Origin::Change { change } => UnitKey::Change(change),
    }
}

/// A worktree-relative path as stored: `/`-separated, no `..`, no root, valid UTF-8. A restore
/// only ever writes below the worktree.
fn relative_path(path: &Path) -> Result<String, Error> {
    let outside = || Error::PathOutsideWorktree(path.to_path_buf());
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().ok_or_else(outside)?),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside());
            }
        }
    }
    if parts.is_empty() {
        return Err(outside());
    }
    Ok(parts.join("/"))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(TEMPORARY_SUFFIX);
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes).map_err(io_error("write", &temporary))?;
    fs::rename(&temporary, path).map_err(io_error("replace", path))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: u64 = 24 * 60 * 60 * 1000;

    struct Fixture {
        _root: tempfile::TempDir,
        store: PathBuf,
        worktree: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().expect("temporary dir");
            let store = root.path().join("history").join("store");
            let worktree = root.path().join("worktree");
            fs::create_dir_all(&worktree).expect("worktree");
            Self {
                _root: root,
                store,
                worktree,
            }
        }

        fn open(&self, limits: Limits) -> History {
            History::open(&self.store, &self.worktree, limits).expect("open")
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.worktree.join(path);
            fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            fs::write(path, text).expect("write");
        }

        fn read(&self, path: &str) -> Option<String> {
            fs::read_to_string(self.worktree.join(path)).ok()
        }
    }

    /// Simulates an accepted agent turn: writes `after` to disk and records each file.
    fn accept_turn(
        fixture: &Fixture,
        history: &mut History,
        files: &[(&str, Option<&str>)],
        now_ms: u64,
    ) -> ChangeId {
        let turn = history
            .begin_turn("session-1", Some("Rename the parser"), now_ms)
            .expect("turn");
        for (path, after) in files {
            let before = fixture.read(path);
            match after {
                Some(text) => fixture.write(path, text),
                None => fs::remove_file(fixture.worktree.join(path)).expect("delete"),
            }
            history
                .record_turn_file(
                    turn,
                    Path::new(path),
                    before.as_deref().map(str::as_bytes),
                    after.map(str::as_bytes),
                    now_ms,
                )
                .expect("record");
        }
        turn
    }

    #[test]
    fn saves_are_deduplicated_and_listed_newest_first() {
        let fixture = Fixture::new();
        let mut history = fixture.open(Limits::default());
        let path = Path::new("src/main.rs");
        let first = history.record_save(path, b"one", 1).expect("save");
        assert!(first.is_some());
        assert_eq!(history.record_save(path, b"one", 2).expect("save"), None);
        let second = history.record_save(path, b"two", 3).expect("save");
        let timeline = history.file_timeline(path).expect("timeline");
        let ids: Vec<_> = timeline.iter().map(|snapshot| Some(snapshot.id)).collect();
        assert_eq!(ids, vec![second, first]);
        assert_eq!(
            history.read(&timeline[1].content).expect("read").as_deref(),
            Some(&b"one"[..])
        );
    }

    #[test]
    fn an_accepted_turn_is_undone_in_one_step_and_the_undo_can_be_undone() {
        let fixture = Fixture::new();
        fixture.write("a.rs", "a before");
        fixture.write("c.rs", "c before");
        let mut history = fixture.open(Limits::default());
        let turn = accept_turn(
            &fixture,
            &mut history,
            &[
                ("a.rs", Some("a middle")),
                ("b/new.rs", Some("created")),
                ("c.rs", None),
                ("a.rs", Some("a after")),
            ],
            10,
        );

        let summary = &history.changes()[0];
        assert_eq!(summary.change.id, turn);
        let paths: Vec<_> = summary
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(paths, vec!["a.rs", "b/new.rs", "c.rs"]);

        let plan = history.plan(RestoreSource::Change(turn)).expect("plan");
        assert!(plan.conflicts.is_empty());
        let undo = history
            .restore(RestoreSource::Change(turn), false, 20)
            .expect("restore");
        assert_eq!(fixture.read("a.rs").as_deref(), Some("a before"));
        assert_eq!(fixture.read("b/new.rs"), None);
        assert_eq!(fixture.read("c.rs").as_deref(), Some("c before"));

        history
            .restore(RestoreSource::Change(undo), false, 30)
            .expect("redo");
        assert_eq!(fixture.read("a.rs").as_deref(), Some("a after"));
        assert_eq!(fixture.read("b/new.rs").as_deref(), Some("created"));
        assert_eq!(fixture.read("c.rs"), None);
    }

    #[test]
    fn a_later_edit_is_a_conflict_and_is_kept_when_overwritten() {
        let fixture = Fixture::new();
        fixture.write("a.rs", "before");
        let mut history = fixture.open(Limits::default());
        let turn = accept_turn(&fixture, &mut history, &[("a.rs", Some("agent"))], 10);
        fixture.write("a.rs", "user edit");

        let plan = history.plan(RestoreSource::Change(turn)).expect("plan");
        assert_eq!(plan.conflicts, vec!["a.rs".to_string()]);
        assert!(matches!(
            history.restore(RestoreSource::Change(turn), false, 20),
            Err(Error::Conflicts(_))
        ));
        assert_eq!(fixture.read("a.rs").as_deref(), Some("user edit"));

        let undo = history
            .restore(RestoreSource::Change(turn), true, 30)
            .expect("overwrite");
        assert_eq!(fixture.read("a.rs").as_deref(), Some("before"));
        history
            .restore(RestoreSource::Change(undo), false, 40)
            .expect("undo the restore");
        assert_eq!(fixture.read("a.rs").as_deref(), Some("user edit"));
    }

    #[test]
    fn a_file_goes_back_to_an_older_save() {
        let fixture = Fixture::new();
        let mut history = fixture.open(Limits::default());
        let path = Path::new("notes.md");
        let first = history
            .record_save(path, b"first", 1)
            .expect("save")
            .expect("new");
        fixture.write("notes.md", "second");
        history.record_save(path, b"second", 2).expect("save");
        history
            .restore(RestoreSource::Snapshot(first), false, 3)
            .expect("restore");
        assert_eq!(fixture.read("notes.md").as_deref(), Some("first"));
        assert_eq!(history.file_timeline(path).expect("timeline").len(), 3);
    }

    #[test]
    fn paths_outside_the_worktree_are_refused() {
        let fixture = Fixture::new();
        let mut history = fixture.open(Limits::default());
        for path in ["../escape.rs", "a/../../escape.rs", "", "."] {
            assert!(
                matches!(
                    history.record_save(Path::new(path), b"x", 1),
                    Err(Error::PathOutsideWorktree(_))
                ),
                "{path:?}"
            );
        }
        let absolute = fixture.worktree.join("a.rs");
        assert!(matches!(
            history.record_save(&absolute, b"x", 1),
            Err(Error::PathOutsideWorktree(_))
        ));
    }

    #[test]
    fn a_turn_over_a_too_large_file_cannot_be_restored() {
        let fixture = Fixture::new();
        fixture.write("big.bin", "0123456789");
        let mut history = fixture.open(Limits {
            max_file_bytes: 4,
            ..Limits::default()
        });
        let turn = accept_turn(&fixture, &mut history, &[("big.bin", Some("ab"))], 10);
        let plan = history.plan(RestoreSource::Change(turn)).expect("plan");
        assert_eq!(plan.not_stored, vec!["big.bin".to_string()]);
        assert!(matches!(
            history.restore(RestoreSource::Change(turn), true, 20),
            Err(Error::NotStored(_))
        ));
        assert_eq!(fixture.read("big.bin").as_deref(), Some("ab"));
    }

    #[test]
    fn prune_drops_old_and_oversized_history_whole_turns_at_a_time() {
        let fixture = Fixture::new();
        fixture.write("a.rs", "aaaa");
        let mut history = fixture.open(Limits {
            max_total_bytes: 12,
            max_age_ms: 10 * DAY_MS,
            max_file_bytes: 1024,
        });
        let path = Path::new("a.rs");
        history.record_save(path, b"old!", 0).expect("save");
        let turn = accept_turn(
            &fixture,
            &mut history,
            &[("a.rs", Some("bbbb")), ("b.rs", Some("cccc"))],
            20 * DAY_MS,
        );
        history
            .record_save(path, b"dddd", 21 * DAY_MS)
            .expect("save");
        history
            .record_save(path, b"eeee", 22 * DAY_MS)
            .expect("save");
        assert_eq!(history.stored_bytes(), 24);

        let report = history.prune(22 * DAY_MS).expect("prune");
        // The age limit drops the first save; the size limit then drops the whole turn (three
        // contents), never one of its two files.
        assert_eq!(report.removed_snapshots, 3);
        assert!(
            history
                .changes()
                .iter()
                .all(|summary| summary.change.id != turn)
        );
        assert!(history.stored_bytes() <= 12);
        assert_eq!(report.removed_blobs, 4);

        let reopened = fixture.open(Limits::default());
        assert_eq!(reopened.file_timeline(path).expect("timeline").len(), 2);
        assert!(reopened.changes().is_empty());
    }

    #[test]
    fn the_index_survives_a_reopen_and_a_torn_last_line() {
        let fixture = Fixture::new();
        fixture.write("a.rs", "before");
        let turn = {
            let mut history = fixture.open(Limits::default());
            accept_turn(&fixture, &mut history, &[("a.rs", Some("after"))], 10)
        };
        let index = fixture.store.join(INDEX_FILE);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&index)
            .expect("index");
        file.write_all(b"{\"entry\":\"snap").expect("torn line");
        drop(file);

        let mut history = fixture.open(Limits::default());
        assert_eq!(history.skipped_lines(), 1);
        let next = history.begin_turn("session-2", None, 20).expect("turn");
        assert!(next > turn);
        history
            .restore(RestoreSource::Change(turn), false, 30)
            .expect("restore");
        assert_eq!(fixture.read("a.rs").as_deref(), Some("before"));
        history.prune(30).expect("prune");
        assert_eq!(fixture.open(Limits::default()).skipped_lines(), 0);
    }

    #[test]
    fn an_index_of_another_schema_is_refused() {
        let fixture = Fixture::new();
        fs::create_dir_all(&fixture.store).expect("store");
        fs::write(
            fixture.store.join(INDEX_FILE),
            "{\"entry\":\"header\",\"schema\":\"fyx-history/9\"}\n",
        )
        .expect("index");
        assert!(matches!(
            History::open(&fixture.store, &fixture.worktree, Limits::default()),
            Err(Error::UnknownSchema(schema)) if schema == "fyx-history/9"
        ));
    }

    #[test]
    fn store_dirs_differ_per_worktree_and_hide_the_path() {
        let root = Path::new("history");
        let one = store_dir(root, Path::new("D:/projects/one"));
        let two = store_dir(root, Path::new("D:/projects/two"));
        assert_ne!(one, two);
        assert!(!one.to_string_lossy().contains("projects"));
    }
}
