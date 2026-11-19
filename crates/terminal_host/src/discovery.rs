//! Plan 5.12, design wit-0.4 §B.8: the discovery files CLI tools find an IDE by. An extension
//! declares them; the host fills each template for an open listener and writes it, and removes
//! it when the listener closes, on revocation, and on exit. A file Fyx did not write is never
//! removed: what it wrote is recorded in `<data_dir>/extensions/fyx/discovery.json`, and at start
//! the files of runs that are gone are removed.
//!
//! The writer runs on a thread of its own, so the listeners (on the main thread or a tokio worker)
//! never wait on the file system. It is told what is open ([`DiscoveryWriter::opened`],
//! [`DiscoveryWriter::closed`]) and what the window shows ([`Context`]: worktree roots for
//! `${worktrees}`, account dirs for `${account.<tool>}`), and rewrites the files when either
//! changes.

mod fill;
mod safe_fs;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, Global, WeakEntity};
use project::Project;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use settings::SettingsStore;

use fill::Vars;

/// One `[[fyx_ui.discovery_files]]` entry that is granted now.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveryFile {
    /// `${account.<tool>}/…` or `${tmp}/…`.
    pub dir: String,
    /// `~/…`, used when the tool has no account profile.
    pub default_dir: Option<String>,
    pub name: String,
    /// The JSON document, its string values templates.
    pub content: Value,
}

/// An open listener and its granted discovery files.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenListener {
    pub name: String,
    pub port: u16,
    pub token: String,
    pub files: Vec<DiscoveryFile>,
}

/// What the files depend on besides the listener.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// The local worktree roots of the open windows, in order, without repeats.
    pub worktrees: Vec<String>,
    /// Every account profile's dir, by tool.
    pub accounts: BTreeMap<String, Vec<PathBuf>>,
}

enum Job {
    Opened(String, OpenListener),
    Closed(String, String),
    Context(Context),
    /// On exit: remove everything, then answer.
    CloseAll(mpsc::Sender<()>),
    #[cfg(test)]
    Barrier(mpsc::Sender<()>),
}

/// The discovery-file writer. Cheap to call from any thread.
pub struct DiscoveryWriter {
    jobs: Mutex<mpsc::Sender<Job>>,
}

impl DiscoveryWriter {
    /// Starts the writer thread, which first removes the files of runs that are gone.
    pub fn start(registry: PathBuf) -> Arc<Self> {
        Self::start_with(Writer::new(Some(registry)))
    }

    fn start_with(mut writer: Writer) -> Arc<Self> {
        let (jobs, queued) = mpsc::channel();
        std::thread::Builder::new()
            .name("fyx-discovery".into())
            .spawn(move || {
                writer.remove_stale();
                for job in queued {
                    writer.run(job);
                }
            })
            .expect("spawn the discovery-file writer");
        Arc::new(Self {
            jobs: Mutex::new(jobs),
        })
    }

    fn send(&self, job: Job) {
        if let Ok(jobs) = self.jobs.lock() {
            jobs.send(job).ok();
        }
    }

    /// A listener opened, or its granted files changed: write exactly `listener.files`.
    pub fn opened(&self, extension_id: &str, listener: OpenListener) {
        self.send(Job::Opened(extension_id.to_owned(), listener));
    }

    /// A listener closed: remove every file written for it.
    pub fn closed(&self, extension_id: &str, listener: &str) {
        self.send(Job::Closed(extension_id.to_owned(), listener.to_owned()));
    }

    pub fn set_context(&self, context: Context) {
        self.send(Job::Context(context));
    }

    /// Removes every file, waiting at most `timeout`.
    pub fn close_all(&self, timeout: Duration) {
        let (done, wait) = mpsc::channel();
        self.send(Job::CloseAll(done));
        wait.recv_timeout(timeout).ok();
    }

    #[cfg(test)]
    fn flush(&self) {
        let (done, wait) = mpsc::channel();
        self.send(Job::Barrier(done));
        wait.recv().unwrap();
    }
}

type Key = (String, String);

#[derive(Default, Serialize, Deserialize)]
struct Registry {
    files: Vec<Recorded>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Recorded {
    path: PathBuf,
    pid: u32,
}

struct Writer {
    registry: Option<PathBuf>,
    home: Option<PathBuf>,
    tmp: PathBuf,
    pid: u32,
    context: Context,
    open: BTreeMap<Key, OpenListener>,
    written: BTreeMap<Key, Vec<PathBuf>>,
}

impl Writer {
    fn new(registry: Option<PathBuf>) -> Self {
        Self {
            registry,
            home: Some(util::paths::home_dir().clone()),
            tmp: std::env::temp_dir(),
            pid: std::process::id(),
            context: Context::default(),
            open: BTreeMap::new(),
            written: BTreeMap::new(),
        }
    }

    fn run(&mut self, job: Job) {
        match job {
            Job::Opened(extension, listener) => {
                let key = (extension, listener.name.clone());
                self.open.insert(key.clone(), listener);
                self.write(&key);
            }
            Job::Closed(extension, listener) => {
                let key = (extension, listener);
                self.open.remove(&key);
                self.write(&key);
            }
            Job::Context(context) => {
                if context != self.context {
                    self.context = context;
                    let keys: Vec<_> = self.open.keys().cloned().collect();
                    for key in keys {
                        self.write(&key);
                    }
                }
            }
            Job::CloseAll(done) => {
                self.open.clear();
                let keys: Vec<_> = self.written.keys().cloned().collect();
                for key in keys {
                    self.write(&key);
                }
                done.send(()).ok();
            }
            #[cfg(test)]
            Job::Barrier(done) => {
                done.send(()).ok();
            }
        }
    }

    /// Makes the files on disk for `key` what its open listener says (none when closed).
    fn write(&mut self, key: &Key) {
        let mut now = Vec::new();
        if let Some(listener) = self.open.get(key) {
            for file in &listener.files {
                match self.write_file(listener, file) {
                    Ok(paths) => now.extend(paths),
                    Err(error) => log::warn!(
                        "fyx: discovery file {:?} of {}/{}: {error}",
                        file.name,
                        key.0,
                        key.1
                    ),
                }
            }
        }
        let before = self.written.remove(key).unwrap_or_default();
        for path in before.iter().filter(|path| !now.contains(path)) {
            if let Err(error) = safe_fs::remove_file(path) {
                log::warn!("fyx: removing discovery file {}: {error}", path.display());
            }
        }
        if !now.is_empty() {
            self.written.insert(key.clone(), now);
        }
        if before != self.written.get(key).cloned().unwrap_or_default() {
            self.save_registry();
        }
    }

    fn write_file(
        &self,
        listener: &OpenListener,
        file: &DiscoveryFile,
    ) -> Result<Vec<PathBuf>, String> {
        let vars = Vars {
            port: listener.port,
            token: listener.token.clone(),
            pid: self.pid,
            worktrees: self.context.worktrees.clone(),
            uid: uid(),
            tmp: self.tmp.to_string_lossy().into_owned(),
            windows: cfg!(windows),
        };
        let content = fill::fill(&file.content, &vars)?;
        let name = fill::file_name(&file.name, &vars)?;
        let targets = fill::targets(
            &file.dir,
            file.default_dir.as_deref(),
            |tool| self.context.accounts.get(tool).cloned().unwrap_or_default(),
            self.home.as_deref(),
            &self.tmp,
        )?;
        let mut contents = serde_json::to_vec_pretty(&content).map_err(|e| e.to_string())?;
        contents.push(b'\n');
        let mut paths = Vec::new();
        for target in targets {
            let written = safe_fs::prepare_dir(&target)
                .and_then(|dir| safe_fs::write_file(&dir, &name, &contents));
            match written {
                Ok(path) => paths.push(path),
                // One account's folder failing does not stop the others.
                Err(error) => log::warn!(
                    "fyx: discovery file {name:?} in {}: {error}",
                    target.base.display()
                ),
            }
        }
        Ok(paths)
    }

    fn load_registry(&self) -> Registry {
        self.registry
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Records this run's files beside those of other runs that are still going.
    fn save_registry(&self) {
        let Some(path) = self.registry.as_deref() else {
            return;
        };
        let mut registry = self.load_registry();
        registry.files.retain(|file| file.pid != self.pid);
        registry
            .files
            .extend(self.written.values().flatten().map(|path| Recorded {
                path: path.clone(),
                pid: self.pid,
            }));
        registry.files.sort();
        registry.files.dedup();
        if let Err(error) = save(path, &registry) {
            log::warn!("fyx: saving {}: {error}", path.display());
        }
    }

    /// At start: removes the files of runs whose process is gone, and forgets them.
    fn remove_stale(&self) {
        let Some(path) = self.registry.as_deref() else {
            return;
        };
        let mut registry = self.load_registry();
        if registry.files.is_empty() {
            return;
        }
        let pids: BTreeSet<u32> = registry.files.iter().map(|file| file.pid).collect();
        let running = running(&pids);
        registry.files.retain(|file| {
            if file.pid != self.pid && running.contains(&file.pid) {
                return true;
            }
            if let Err(error) = safe_fs::remove_file(&file.path) {
                log::warn!(
                    "fyx: removing stale discovery file {}: {error}",
                    file.path.display()
                );
            }
            false
        });
        if let Err(error) = save(path, &registry) {
            log::warn!("fyx: saving {}: {error}", path.display());
        }
    }
}

fn save(path: &Path, registry: &Registry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(registry)?)?;
    std::fs::rename(&temporary, path)
}

fn running(pids: &BTreeSet<u32>) -> BTreeSet<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let wanted: Vec<Pid> = pids.iter().map(|pid| Pid::from_u32(*pid)).collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&wanted),
        true,
        ProcessRefreshKind::nothing(),
    );
    pids.iter()
        .copied()
        .filter(|pid| system.process(Pid::from_u32(*pid)).is_some())
        .collect()
}

fn uid() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions.
        Some(unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        None
    }
}

struct GlobalDiscovery {
    writer: Arc<DiscoveryWriter>,
    projects: Vec<WeakEntity<Project>>,
    last: Option<Context>,
}

impl Global for GlobalDiscovery {}

/// The writer [`init_discovery`] started, for the listeners.
pub fn discovery_writer(cx: &App) -> Option<Arc<DiscoveryWriter>> {
    cx.try_global::<GlobalDiscovery>()
        .map(|discovery| discovery.writer.clone())
}

/// Starts the writer with its record in `data_dir`, keeps its [`Context`] current, and removes
/// every file on quit. Called once by `main.rs`.
pub fn init_discovery(data_dir: &Path, cx: &mut App) {
    let writer = DiscoveryWriter::start(data_dir.join("extensions/fyx/discovery.json"));
    cx.set_global(GlobalDiscovery {
        writer: writer.clone(),
        projects: Vec::new(),
        last: None,
    });
    cx.observe_new(|_: &mut Project, _, cx| {
        let project = cx.weak_entity();
        let discovery = cx.global_mut::<GlobalDiscovery>();
        discovery.projects.retain(|project| project.is_upgradable());
        discovery.projects.push(project);
        cx.subscribe_self(|_, event: &project::Event, cx| {
            if matches!(
                event,
                project::Event::WorktreeAdded(_)
                    | project::Event::WorktreeRemoved(_)
                    | project::Event::WorktreeOrderChanged
            ) {
                cx.defer(refresh);
            }
        })
        .detach();
        cx.on_release(|_, cx| cx.defer(refresh)).detach();
        cx.defer(refresh);
    })
    .detach();
    cx.observe_global::<SettingsStore>(refresh).detach();
    cx.on_app_quit(move |_| {
        let writer = writer.clone();
        async move { writer.close_all(Duration::from_secs(2)) }
    })
    .detach();
    refresh(cx);
}

fn refresh(cx: &mut App) {
    let Some(discovery) = cx.try_global::<GlobalDiscovery>() else {
        return;
    };
    let mut worktrees = Vec::new();
    for project in discovery.projects.iter().filter_map(WeakEntity::upgrade) {
        let project = project.read(cx);
        if !project.is_local() {
            continue;
        }
        for worktree in project.visible_worktrees(cx) {
            let root = worktree.read(cx).abs_path().to_string_lossy().into_owned();
            if !worktrees.contains(&root) {
                worktrees.push(root);
            }
        }
    }
    let mut accounts = BTreeMap::<String, Vec<PathBuf>>::new();
    for account in crate::accounts::settings(cx).accounts() {
        let dirs = accounts.entry(account.tool).or_default();
        if !dirs.contains(&account.dir) {
            dirs.push(account.dir);
        }
    }
    let context = Context {
        worktrees,
        accounts,
    };
    if discovery.last.as_ref() == Some(&context) {
        return;
    }
    discovery.writer.set_context(context.clone());
    cx.global_mut::<GlobalDiscovery>().last = Some(context);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn writer(home: &Path, tmp: &Path, registry: &Path) -> Writer {
        Writer {
            registry: Some(registry.to_path_buf()),
            home: Some(home.to_path_buf()),
            tmp: tmp.to_path_buf(),
            pid: std::process::id(),
            context: Context::default(),
            open: BTreeMap::new(),
            written: BTreeMap::new(),
        }
    }

    fn claude_lock() -> DiscoveryFile {
        DiscoveryFile {
            dir: "${account.claude}/ide".into(),
            default_dir: Some("~/.claude/ide".into()),
            name: "${port}.lock".into(),
            content: json!({ "pid": "${pid}", "workspaceFolders": "${worktrees}",
                "ideName": "Fyx", "transport": "ws", "authToken": "${token}" }),
        }
    }

    fn listener(port: u16, files: Vec<DiscoveryFile>) -> OpenListener {
        OpenListener {
            name: "ide".into(),
            port,
            token: "secret".into(),
            files,
        }
    }

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn files_follow_the_listener_the_accounts_and_the_worktrees() {
        let root = tempfile::tempdir().unwrap();
        let (home, tmp) = (root.path().join("home"), root.path().join("tmp"));
        let (work, me) = (
            root.path().join("claude-work"),
            root.path().join("claude-me"),
        );
        for dir in [&home, &tmp, &work, &me] {
            std::fs::create_dir(dir).unwrap();
        }
        let registry = root.path().join("data/extensions/fyx/discovery.json");
        let discovery = DiscoveryWriter::start_with(writer(&home, &tmp, &registry));

        // No account profile: the default dir under home.
        discovery.set_context(Context {
            worktrees: vec!["/p".into()],
            accounts: BTreeMap::new(),
        });
        discovery.opened("fyx-claude-code", listener(4100, vec![claude_lock()]));
        discovery.flush();
        let default = home.join(".claude/ide/4100.lock");
        assert_eq!(
            read(&default),
            json!({ "pid": std::process::id(), "workspaceFolders": ["/p"], "ideName": "Fyx",
                "transport": "ws", "authToken": "secret" })
        );

        // Two account profiles: one file in each, and the default one goes.
        discovery.set_context(Context {
            worktrees: vec!["/p".into(), "/q".into()],
            accounts: BTreeMap::from([("claude".into(), vec![work.clone(), me.clone()])]),
        });
        discovery.flush();
        assert!(!default.exists());
        for dir in [&work, &me] {
            assert_eq!(
                read(&dir.join("ide/4100.lock"))["workspaceFolders"],
                json!(["/p", "/q"])
            );
        }
        let recorded: Registry =
            serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
        assert_eq!(recorded.files.len(), 2);

        // The grant goes: `opened` with no files removes them; a file nobody wrote stays.
        std::fs::write(work.join("ide/9999.lock"), b"{}").unwrap();
        discovery.opened("fyx-claude-code", listener(4100, Vec::new()));
        discovery.flush();
        assert!(!work.join("ide/4100.lock").exists() && !me.join("ide/4100.lock").exists());
        assert!(work.join("ide/9999.lock").exists());

        // Reopened, then closed; then quit removes what is left of another listener.
        discovery.opened("fyx-claude-code", listener(4200, vec![claude_lock()]));
        discovery.opened(
            "fyx-gemini-cli",
            OpenListener {
                name: "ide".into(),
                port: 4300,
                token: "g".into(),
                files: vec![DiscoveryFile {
                    dir: "${tmp}/gemini/ide".into(),
                    default_dir: None,
                    name: "gemini-ide-server-${pid}-${port}.json".into(),
                    content: json!({ "port": "${port}", "workspacePath": "${worktrees.joined}",
                        "authToken": "${token}",
                        "ideInfo": { "name": "fyx", "displayName": "Fyx" } }),
                }],
            },
        );
        discovery.closed("fyx-claude-code", "ide");
        discovery.flush();
        assert!(!work.join("ide/4200.lock").exists());
        let gemini = tmp.join(format!(
            "gemini/ide/gemini-ide-server-{}-4300.json",
            std::process::id()
        ));
        let separator = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(
            read(&gemini)["workspacePath"],
            json!(format!("/p{separator}/q"))
        );
        assert_eq!(read(&gemini)["port"], json!(4300));
        discovery.close_all(Duration::from_secs(5));
        assert!(!gemini.exists());
        let recorded: Registry =
            serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
        assert!(recorded.files.is_empty());
    }

    #[test]
    fn at_start_the_files_of_runs_that_are_gone_are_removed() {
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("discovery.json");
        let (gone, alive, moved) = (
            root.path().join("1.lock"),
            root.path().join("2.lock"),
            root.path().join("3.lock"),
        );
        std::fs::write(&gone, b"{}").unwrap();
        std::fs::write(&alive, b"{}").unwrap();
        // A folder where a recorded file was is not removed.
        std::fs::create_dir(&moved).unwrap();
        // A pid that cannot be running (above any real pid on these systems), and this test's.
        let dead = u32::MAX - 1;
        save(
            &registry,
            &Registry {
                files: vec![
                    Recorded {
                        path: gone.clone(),
                        pid: dead,
                    },
                    Recorded {
                        path: moved.clone(),
                        pid: dead,
                    },
                    Recorded {
                        path: alive.clone(),
                        pid: std::process::id(),
                    },
                ],
            },
        )
        .unwrap();
        let mut writer = writer(root.path(), root.path(), &registry);
        // Another run's file, as if from a process still going.
        writer.pid = dead - 1;
        writer.remove_stale();
        assert!(!gone.exists());
        assert!(moved.exists());
        assert!(alive.exists());
        let recorded: Registry =
            serde_json::from_slice(&std::fs::read(&registry).unwrap()).unwrap();
        assert_eq!(recorded.files.len(), 1);
        assert_eq!(recorded.files[0].path, alive);
    }
}
