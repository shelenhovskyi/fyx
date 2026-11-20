//! Fyx local history in the app (plan 6.3, `docs/fyx/platform/local-history.md`): records a
//! snapshot on every save and for every file decided in an agent turn, and opens the timeline
//! ("fyx: local history") that restores a file or undoes a whole turn. The store and its rules
//! are `fyx_history`; here are the hooks, the settings and the modal.

mod history_modal;

use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use agent_review::{AgentReview, DecidedFile};
use anyhow::{Result, anyhow};
use fyx_history::{ChangeId, ChangeSummary, History, Limits, RestoreSource, Snapshot};
use gpui::{App, AppContext as _, Entity, Global, Task, TaskExt as _, actions};
use language::{Buffer, BufferEvent, LineEnding};
use project::{Project, buffer_store::BufferStoreEvent};
use settings::{RegisterSetting, Settings, SettingsStore};
use workspace::Workspace;

actions!(
    fyx,
    [
        /// Opens the local history: agent turns and restores of the worktree, and the versions
        /// of the active file, each with a restore.
        LocalHistory
    ]
);

/// A store is pruned after this many records (and once when it opens).
const PRUNE_EVERY: usize = 100;
const MEGABYTE: u64 = 1024 * 1024;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// `fyx.history` in settings (schema in `settings_content::FyxHistorySettingsContent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, RegisterSetting)]
pub struct FyxHistorySettings {
    pub enabled: bool,
    pub limits: Limits,
}

impl Settings for FyxHistorySettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let history = content
            .fyx
            .as_ref()
            .and_then(|fyx| fyx.history.as_ref());
        let defaults = Limits::default();
        let field = |value: Option<u64>, unit: u64, default: u64| {
            value.map_or(default, |value| value.saturating_mul(unit))
        };
        Self {
            enabled: history
                .and_then(|history| history.enabled)
                .unwrap_or(true),
            limits: Limits {
                max_total_bytes: field(
                    history.and_then(|history| history.max_total_mb),
                    MEGABYTE,
                    defaults.max_total_bytes,
                ),
                max_age_ms: field(
                    history.and_then(|history| history.max_age_days),
                    DAY_MS,
                    defaults.max_age_ms,
                ),
                max_file_bytes: field(
                    history.and_then(|history| history.max_file_mb),
                    MEGABYTE,
                    defaults.max_file_bytes,
                ),
            },
        }
    }
}

fn settings(cx: &App) -> FyxHistorySettings {
    if cx.has_global::<SettingsStore>() {
        *FyxHistorySettings::get_global(cx)
    } else {
        FyxHistorySettings {
            enabled: false,
            limits: Limits::default(),
        }
    }
}

struct Store {
    history: History,
    records_since_prune: usize,
    /// `agent_review` turn (stable for the app's run) → this store's change.
    turns: HashMap<u64, ChangeId>,
}

/// The open stores, one per worktree, used from background tasks.
#[derive(Clone)]
pub(crate) struct Stores {
    history_root: PathBuf,
    stores: Arc<Mutex<HashMap<PathBuf, Store>>>,
}

impl Stores {
    fn new(history_root: PathBuf) -> Self {
        Self {
            history_root,
            stores: Arc::default(),
        }
    }

    /// Runs `action` on the store of `worktree`, opening (and pruning) it first. Blocking.
    fn with_store<R>(
        &self,
        worktree: &Path,
        limits: Limits,
        action: impl FnOnce(&mut Store) -> Result<R>,
    ) -> Result<R> {
        let mut stores = self
            .stores
            .lock()
            .map_err(|_| anyhow!("the local history lock is poisoned"))?;
        let store = match stores.entry(worktree.to_path_buf()) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let directory = fyx_history::store_dir(&self.history_root, worktree);
                let mut history = History::open(&directory, worktree, limits)?;
                history.prune(now_ms())?;
                entry.insert(Store {
                    history,
                    records_since_prune: 0,
                    turns: HashMap::new(),
                })
            }
        };
        store.history.set_limits(limits);
        let result = action(store)?;
        if store.records_since_prune >= PRUNE_EVERY {
            store.records_since_prune = 0;
            store.history.prune(now_ms())?;
        }
        Ok(result)
    }
}

/// The worktree roots of the open local projects, to place an agent's absolute path.
#[derive(Default)]
struct Roots(BTreeSet<PathBuf>);

impl Roots {
    /// The innermost root holding `abs_path`, and the path relative to it.
    fn split(&self, abs_path: &Path) -> Option<(PathBuf, PathBuf)> {
        self.0
            .iter()
            .filter_map(|root| {
                let relative = abs_path.strip_prefix(root).ok()?;
                Some((root.clone(), relative.to_path_buf()))
            })
            .max_by_key(|(root, _)| root.components().count())
    }
}

struct GlobalHistory {
    stores: Stores,
    roots: Rc<RefCell<Roots>>,
}

impl Global for GlobalHistory {}

pub fn init(cx: &mut App) {
    init_with_root(paths::data_dir().join("history"), cx);
}

fn init_with_root(history_root: PathBuf, cx: &mut App) {
    cx.set_global(GlobalHistory {
        stores: Stores::new(history_root),
        roots: Rc::default(),
    });
    cx.observe_new(|workspace: &mut Workspace, _, cx| {
        workspace.register_action(|workspace, _: &LocalHistory, window, cx| {
            history_modal::open(workspace, window, cx);
        });
        watch_project(workspace.project().clone(), cx);
    })
    .detach();
    match AgentReview::global(cx) {
        Some(review) => review.update(cx, |review, _| {
            review.set_decided_file_observer(Rc::new(record_decided_file));
        }),
        None => log::warn!("fyx: local history: no agent review, agent turns are not recorded"),
    }
}

fn watch_project(project: Entity<Project>, cx: &mut gpui::Context<Workspace>) {
    if !project.read(cx).is_local() {
        return;
    }
    add_roots(&project, cx);
    cx.subscribe(&project, |_, project, event, cx| {
        if matches!(event, project::Event::WorktreeAdded(_)) {
            add_roots(&project, cx);
        }
    })
    .detach();
    let buffer_store = project.read(cx).buffer_store().clone();
    cx.subscribe(&buffer_store, move |_, _, event, cx| {
        let BufferStoreEvent::BufferAdded(buffer) = event else {
            return;
        };
        let project = project.downgrade();
        cx.subscribe(buffer, move |_, buffer, event, cx| {
            if matches!(event, BufferEvent::Saved)
                && let Some(project) = project.upgrade()
            {
                record_save(&buffer, &project, cx);
            }
        })
        .detach();
    })
    .detach();
}

fn add_roots(project: &Entity<Project>, cx: &App) {
    let Some(global) = cx.try_global::<GlobalHistory>() else {
        return;
    };
    let mut roots = global.roots.borrow_mut();
    for worktree in project.read(cx).visible_worktrees(cx) {
        roots.0.insert(worktree.read(cx).abs_path().to_path_buf());
    }
}

fn record_save(buffer: &Entity<Buffer>, project: &Entity<Project>, cx: &App) {
    let settings = settings(cx);
    let Some(global) = cx.try_global::<GlobalHistory>() else {
        return;
    };
    if !settings.enabled {
        return;
    }
    let buffer = buffer.read(cx);
    let Some(file) = buffer.file() else {
        return;
    };
    if file.as_local().is_none() {
        return;
    }
    let Some(worktree) = project.read(cx).worktree_for_id(file.worktree_id(cx), cx) else {
        return;
    };
    let root = worktree.read(cx).abs_path().to_path_buf();
    let relative = PathBuf::from(file.path().as_unix_str());
    let bytes = with_line_ending(&buffer.text(), buffer.line_ending()).into_bytes();
    let stores = global.stores.clone();
    cx.background_spawn(async move {
        stores.with_store(&root, settings.limits, |store| {
            if store
                .history
                .record_save(&relative, &bytes, now_ms())?
                .is_some()
            {
                store.records_since_prune += 1;
            }
            Ok(())
        })
    })
    .detach_and_log_err(cx);
}

fn record_decided_file(file: DecidedFile, cx: &mut App) {
    let settings = settings(cx);
    let Some(global) = cx.try_global::<GlobalHistory>() else {
        return;
    };
    if !settings.enabled {
        return;
    }
    let Some((root, relative)) = global.roots.borrow().split(&file.abs_path) else {
        log::info!(
            "fyx: local history: {} is outside the open worktrees",
            file.abs_path.display()
        );
        return;
    };
    let stores = global.stores.clone();
    cx.background_spawn(async move {
        // The decided texts use `\n`; the file on disk tells which line ending lands.
        let disk = match std::fs::read(&file.abs_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let line_ending = match &disk {
            Some(bytes) if bytes.windows(2).any(|pair| pair == b"\r\n") => LineEnding::Windows,
            _ => LineEnding::Unix,
        };
        let before = (disk.is_some() || !file.old_text.is_empty())
            .then(|| with_line_ending(&file.old_text, line_ending));
        let after = with_line_ending(&file.landed_text, line_ending);
        stores.with_store(&root, settings.limits, |store| {
            let turn = file.turn.as_u64();
            let change = match store.turns.get(&turn) {
                Some(change) => *change,
                None => {
                    let change =
                        store
                            .history
                            .begin_turn(&file.session_id.0, Some("Agent turn"), now_ms())?;
                    store.turns.insert(turn, change);
                    change
                }
            };
            if store
                .history
                .record_turn_file(
                    change,
                    &relative,
                    before.as_deref().map(str::as_bytes),
                    Some(after.as_bytes()),
                    now_ms(),
                )?
                .is_some()
            {
                store.records_since_prune += 1;
            }
            Ok(())
        })
    })
    .detach_and_log_err(cx);
}

/// What the modal shows for one worktree.
pub(crate) struct Timeline {
    pub changes: Vec<ChangeSummary>,
    /// The active file's snapshots, newest first.
    pub file: Vec<Snapshot>,
}

pub(crate) enum RestoreOutcome {
    Restored,
    /// Files changed since; restoring needs "Restore Anyway".
    Conflicts(Vec<String>),
}

pub(crate) fn load_timeline(
    worktree: PathBuf,
    file: Option<PathBuf>,
    cx: &App,
) -> Task<Result<Timeline>> {
    let Some(global) = cx.try_global::<GlobalHistory>() else {
        return Task::ready(Err(anyhow!("local history is not running")));
    };
    let stores = global.stores.clone();
    let limits = settings(cx).limits;
    cx.background_spawn(async move {
        stores.with_store(&worktree, limits, |store| {
            let file = match &file {
                Some(file) => store
                    .history
                    .file_timeline(file)?
                    .into_iter()
                    .cloned()
                    .collect(),
                None => Vec::new(),
            };
            Ok(Timeline {
                changes: store.history.changes(),
                file,
            })
        })
    })
}

pub(crate) fn restore(
    worktree: PathBuf,
    source: RestoreSource,
    overwrite_conflicts: bool,
    cx: &App,
) -> Task<Result<RestoreOutcome>> {
    let Some(global) = cx.try_global::<GlobalHistory>() else {
        return Task::ready(Err(anyhow!("local history is not running")));
    };
    let stores = global.stores.clone();
    let limits = settings(cx).limits;
    cx.background_spawn(async move {
        stores.with_store(&worktree, limits, |store| {
            match store
                .history
                .restore(source, overwrite_conflicts, now_ms())
            {
                Ok(_) => {
                    store.records_since_prune += 1;
                    Ok(RestoreOutcome::Restored)
                }
                Err(fyx_history::Error::Conflicts(paths)) => Ok(RestoreOutcome::Conflicts(paths)),
                Err(error) => Err(error.into()),
            }
        })
    })
}

fn with_line_ending(text: &str, line_ending: LineEnding) -> String {
    match line_ending {
        LineEnding::Unix => text.to_string(),
        LineEnding::Windows => text.replace("\r\n", "\n").replace('\n', "\r\n"),
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, UpdateGlobal as _};

    #[test]
    fn line_endings_follow_the_file() {
        assert_eq!(with_line_ending("a\nb\n", LineEnding::Unix), "a\nb\n");
        assert_eq!(with_line_ending("a\nb\n", LineEnding::Windows), "a\r\nb\r\n");
        assert_eq!(with_line_ending("a\r\nb", LineEnding::Windows), "a\r\nb");
    }

    #[test]
    fn a_path_goes_to_its_innermost_worktree() {
        let mut roots = Roots::default();
        roots.0.insert(PathBuf::from("/projects/app"));
        roots.0.insert(PathBuf::from("/projects/app/vendor/lib"));
        assert_eq!(
            roots.split(Path::new("/projects/app/vendor/lib/src/a.rs")),
            Some((
                PathBuf::from("/projects/app/vendor/lib"),
                PathBuf::from("src/a.rs")
            ))
        );
        assert_eq!(
            roots.split(Path::new("/projects/app/main.rs")),
            Some((PathBuf::from("/projects/app"), PathBuf::from("main.rs")))
        );
        assert_eq!(roots.split(Path::new("/elsewhere/a.rs")), None);
    }

    #[gpui::test]
    fn settings_are_read_in_their_units(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
            assert_eq!(
                settings(cx),
                FyxHistorySettings {
                    enabled: true,
                    limits: Limits::default()
                }
            );
            SettingsStore::update_global(cx, |store, cx| {
                store
                    .set_user_settings(
                        r#"{ "fyx": { "history": { "enabled": false, "max_total_mb": 1, "max_age_days": 2, "max_file_mb": 3 } } }"#,
                        cx,
                    )
                    .result()
                    .expect("valid settings");
            });
            assert_eq!(
                settings(cx),
                FyxHistorySettings {
                    enabled: false,
                    limits: Limits {
                        max_total_bytes: MEGABYTE,
                        max_age_ms: 2 * DAY_MS,
                        max_file_bytes: 3 * MEGABYTE,
                    }
                }
            );
        });
    }

    #[test]
    fn a_conflict_comes_back_as_an_outcome_and_restore_anyway_overwrites() {
        let root = tempfile::tempdir().expect("temporary dir");
        let worktree = root.path().join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree");
        std::fs::write(worktree.join("a.rs"), "before").expect("write");
        let stores = Stores::new(root.path().join("history"));
        let limits = Limits::default();
        let change = stores
            .with_store(&worktree, limits, |store| {
                let change = store.history.begin_turn("session", None, now_ms())?;
                std::fs::write(worktree.join("a.rs"), "agent")?;
                store.history.record_turn_file(
                    change,
                    Path::new("a.rs"),
                    Some(b"before"),
                    Some(b"agent"),
                    now_ms(),
                )?;
                Ok(change)
            })
            .expect("turn");
        std::fs::write(worktree.join("a.rs"), "user").expect("user edit");
        let attempt = |overwrite| {
            stores.with_store(&worktree, limits, |store| {
                match store
                    .history
                    .restore(RestoreSource::Change(change), overwrite, now_ms())
                {
                    Err(fyx_history::Error::Conflicts(paths)) => Ok(paths),
                    Ok(_) => Ok(Vec::new()),
                    Err(error) => Err(error.into()),
                }
            })
        };
        assert_eq!(attempt(false).expect("plan"), vec!["a.rs".to_string()]);
        assert_eq!(attempt(true).expect("restore"), Vec::<String>::new());
        assert_eq!(
            std::fs::read_to_string(worktree.join("a.rs")).expect("read"),
            "before"
        );
    }
}
