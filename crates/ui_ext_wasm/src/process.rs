//! `process:spawn` for fyx:ui 0.3, host side (docs/fyx/wit-0.3/design.md §9, plan 4.7, spec
//! decision 24). The manifest rules and the import checks are `ui_ext::process`'s, shared with
//! the Apache harness and re-exported here; this module runs the programs:
//!
//! - **Resolution:** the host finds the program itself and spawns its absolute path, so the OS
//!   never searches. On `PATH`, entries that are empty, relative or inside any worktree are
//!   skipped (spec §4 failure 4: a planted `git.exe` is never run). On Windows only `.exe` and
//!   `.com`; `.bat` and `.cmd` are refused (`cmd.exe` re-parses their arguments). A manifest
//!   `path` program runs only from a trusted worktree and never through a link out of it.
//! - **Environment:** cleared, then `PATH`, `HOME` (on Windows `USERPROFILE`, `SystemRoot`,
//!   `TEMP`, `TMP`), `LANG`, `LC_*`, the declared names from Fyx's environment, then the guest's
//!   pairs for declared names. Names that load code (`LD_*`, `NODE_OPTIONS`, …) never pass.
//! - **The tree dies with it:** on Windows the child is created suspended, put in a job object
//!   with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, then resumed, so even a grandchild started in its
//!   first instant is in the job; on Unix it leads its own process group (and on Linux dies with
//!   Fyx). Kill, revocation, a trap, reload, disable and Fyx quitting kill the whole tree.
//! - **I/O:** pipes, no pty; output in chunks of at most 64 KiB through `handle-io`, with the
//!   queue's backpressure (a guest that does not keep up stalls the program, not memory).

use std::{
    collections::{BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use futures::{StreamExt as _, channel::mpsc};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
pub use ui_ext::process::{
    CAPABILITY_PROCESS, FileRef, KILL_GRACE, MAX_ARG_BYTES, MAX_ARGS, MAX_ENV_PAIRS,
    MAX_OUTPUT_CHUNK_BYTES, MAX_PROCESSES, MAX_STDIN_QUEUED_BYTES, MAX_STDIN_WRITE_BYTES,
    OutputStream, ProcessCounters, ProcessExit, ProcessOutput, ProcessPolicy, ProcessSpec,
    SpawnCheck, SpawnError, SpawnRequest, args_match, check_env_name, check_spawn, is_allowed_env,
    is_forbidden_env,
};
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::{
    network::{IoEvent, Running},
    permissions::{CapabilityCheck, PermissionState},
};

/// A worktree of an open workspace, as the host shows it to extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeRoot {
    /// The opaque id of the WIT `file-ref`, stable for the session.
    pub id: u64,
    pub root: PathBuf,
    /// `process:spawn` runs only in trusted worktrees (decision 26).
    pub trusted: bool,
}

/// The host's open worktrees, for `cwd`, `path` programs and the `PATH` filter. Implemented by
/// the host (`ui_ext_host`); without one there is no worktree and `spawn` is `unavailable`.
pub trait Workspace: Send + Sync + 'static {
    fn worktrees(&self) -> Vec<WorktreeRoot>;
}

/// Fyx's own environment, replaceable in tests.
pub type Environment = Arc<dyn Fn() -> Vec<(OsString, OsString)> + Send + Sync>;

fn invalid(message: impl Into<String>) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------------------------
// Resolution

fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// `name` on `path_var`, skipping empty and relative entries and every entry inside one of the
/// (canonical) `worktree_roots`. On Windows only `.exe` and `.com` run; a name ending in `.bat`
/// or `.cmd` is refused.
pub fn resolve_on_path(
    name: &str,
    path_var: &OsStr,
    worktree_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let candidates = candidates(name)?;
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() || !dir.is_absolute() {
            continue;
        }
        let Some(dir) = canonical(&dir) else {
            continue;
        };
        if worktree_roots.iter().any(|root| dir.starts_with(root)) {
            continue;
        }
        for candidate in &candidates {
            let path = dir.join(candidate);
            if is_executable(&path) {
                return Ok(path);
            }
        }
    }
    Err(format!("{name} is not on PATH outside the worktrees"))
}

fn candidates(name: &str) -> Result<Vec<String>, String> {
    if cfg!(windows) {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            return Err(format!(
                "{name} is a batch script, which Fyx does not run (cmd.exe re-parses its arguments)"
            ));
        }
        if lower.ends_with(".exe") || lower.ends_with(".com") {
            return Ok(vec![name.to_string()]);
        }
        Ok(vec![format!("{name}.exe"), format!("{name}.com")])
    } else {
        Ok(vec![name.to_string()])
    }
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}

/// A manifest `path` program: inside `root` after resolving links, an executable file, and on
/// Windows not a batch script.
pub fn resolve_in_worktree(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = canonical(root).ok_or_else(|| "the worktree is gone".to_string())?;
    let path = canonical(&root.join(relative))
        .ok_or_else(|| format!("{relative} does not exist in the worktree"))?;
    if !path.starts_with(&root) {
        return Err(format!("{relative} leaves the worktree through a link"));
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if cfg!(windows) && (name.ends_with(".bat") || name.ends_with(".cmd")) {
        return Err(format!(
            "{relative} is a batch script, which Fyx does not run"
        ));
    }
    if !is_executable(&path) {
        return Err(format!("{relative} is not an executable file"));
    }
    Ok(path)
}

/// The directory a process starts in: `cwd` inside its worktree (by id, or the one worktree that
/// has the path), the first worktree's root without one. Refused when it leaves the worktree
/// through a link. Returns the directory and its worktree.
pub fn resolve_cwd(
    cwd: Option<&FileRef>,
    worktrees: &[WorktreeRoot],
) -> Result<(PathBuf, WorktreeRoot), SpawnError> {
    let Some(first) = worktrees.first() else {
        return Err(SpawnError::Denied);
    };
    let (worktree, relative) = match cwd {
        None => (first.clone(), ""),
        Some(file) => {
            let worktree = match file.worktree {
                Some(id) => worktrees.iter().find(|worktree| worktree.id == id).cloned(),
                None => {
                    let having: Vec<_> = worktrees
                        .iter()
                        .filter(|worktree| worktree.root.join(&file.path).is_dir())
                        .collect();
                    (having.len() == 1).then(|| having[0].clone())
                }
            };
            let Some(worktree) = worktree else {
                return Err(SpawnError::Io(format!("no worktree has {}", file.path)));
            };
            (worktree, file.path.as_str())
        }
    };
    let root =
        canonical(&worktree.root).ok_or_else(|| SpawnError::Io("the worktree is gone".into()))?;
    let dir = canonical(&root.join(relative))
        .ok_or_else(|| SpawnError::Io(format!("{relative} does not exist")))?;
    if !dir.starts_with(&root) || !dir.is_dir() {
        return Err(SpawnError::Io(format!(
            "{relative} is not a directory inside the worktree"
        )));
    }
    Ok((dir, worktree))
}

// ---------------------------------------------------------------------------------------------
// Environment

fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// `PATH` without the entries the host never resolves programs from: empty, relative, or inside a
/// worktree (integration-5 S6), so the program does not find planted tools there either.
pub fn filtered_path(path: &OsStr, worktree_roots: &[PathBuf]) -> OsString {
    let kept: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| {
            !dir.as_os_str().is_empty()
                && dir.is_absolute()
                && !canonical(dir)
                    .is_some_and(|dir| worktree_roots.iter().any(|root| dir.starts_with(root)))
                && !worktree_roots.iter().any(|root| dir.starts_with(root))
        })
        .collect();
    std::env::join_paths(kept).unwrap_or_default()
}

/// The environment of a spawned program (design.md §9, "Environment"): a few of Fyx's own
/// variables (with `PATH` as [`filtered_path`] makes it), the declared ones, and the guest's
/// values for declared names. Only [`is_allowed_env`] names are declared or set.
pub fn program_environment(
    fyx: &[(OsString, OsString)],
    declared: &BTreeSet<String>,
    guest: &[(String, String)],
) -> Vec<(OsString, OsString)> {
    let base: &[&str] = if cfg!(windows) {
        &[
            "PATH",
            "HOME",
            "USERPROFILE",
            "SystemRoot",
            "TEMP",
            "TMP",
            "LANG",
        ]
    } else {
        &["PATH", "HOME", "LANG"]
    };
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    for (name, value) in fyx {
        let Some(text) = name.to_str() else {
            continue;
        };
        let base = base.iter().any(|base| same_name(base, text)) || text.starts_with("LC_");
        let declared =
            is_allowed_env(text) && declared.iter().any(|declared| same_name(declared, text));
        if base || declared {
            env.push((name.clone(), value.clone()));
        }
    }
    for (name, value) in guest {
        if declared.contains(name) && is_allowed_env(name) {
            env.retain(|(existing, _)| {
                !existing
                    .to_str()
                    .is_some_and(|existing| same_name(existing, name))
            });
            env.push((name.into(), value.into()));
        }
    }
    env
}

fn path_value(env: &[(OsString, OsString)]) -> OsString {
    env.iter()
        .find(|(name, _)| name.to_str().is_some_and(|name| same_name(name, "PATH")))
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Killing the tree

/// Kills one process and everything it started.
pub(crate) struct Killer {
    #[cfg(windows)]
    job: job::JobObject,
    #[cfg(unix)]
    group: i32,
}

impl Killer {
    /// The polite kill: on Unix `SIGTERM` to the group, then `SIGKILL` after [`KILL_GRACE`]; on
    /// Windows the job is terminated at once.
    fn kill(&self) {
        #[cfg(windows)]
        self.job.terminate();
        #[cfg(unix)]
        {
            let group = self.group;
            // SAFETY: killpg only sends a signal to a process group this host created.
            unsafe {
                libc::killpg(group, libc::SIGTERM);
            }
            std::thread::spawn(move || {
                std::thread::sleep(KILL_GRACE);
                // SAFETY: as above.
                unsafe {
                    libc::killpg(group, libc::SIGKILL);
                }
            });
        }
    }
}

impl Drop for Killer {
    fn drop(&mut self) {
        // Windows: closing the job handle (in `JobObject`'s drop) kills the tree.
        #[cfg(unix)]
        // SAFETY: as in `kill`.
        unsafe {
            libc::killpg(self.group, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
mod job {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };

    pub(crate) const CREATE_SUSPENDED: u32 = 0x0000_0004;
    pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// A job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: closing it kills every process
    /// in it, which the OS also does when Fyx exits for any reason.
    pub(crate) struct JobObject(HANDLE);

    // SAFETY: job object handles may be used from any thread.
    unsafe impl Send for JobObject {}
    unsafe impl Sync for JobObject {}

    impl JobObject {
        pub(crate) fn new() -> windows::core::Result<Self> {
            // SAFETY: plain Win32 calls on handles this function owns.
            unsafe {
                let job = Self(CreateJobObjectW(None, None)?);
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;
                Ok(job)
            }
        }

        /// Assigns the (suspended) process `handle` to the job.
        pub(crate) fn assign(
            &self,
            handle: std::os::windows::io::RawHandle,
        ) -> windows::core::Result<()> {
            // SAFETY: `handle` is the live process handle of a child this host spawned.
            unsafe { AssignProcessToJobObject(self.0, HANDLE(handle)) }
        }

        pub(crate) fn terminate(&self) {
            // SAFETY: self owns a valid job handle.
            unsafe {
                TerminateJobObject(self.0, 1).ok();
            }
        }
    }

    impl Drop for JobObject {
        fn drop(&mut self) {
            // SAFETY: self owns the handle.
            unsafe {
                CloseHandle(self.0).ok();
            }
        }
    }

    /// Resumes the threads of a process created with `CREATE_SUSPENDED`, once it is in its job.
    pub(crate) fn resume(pid: u32) -> windows::core::Result<()> {
        // SAFETY: a thread snapshot read in the documented way; every opened handle is closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)?;
            let mut entry = THREADENTRY32 {
                dwSize: size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut found = Thread32First(snapshot, &mut entry).is_ok();
            while found {
                if entry.th32OwnerProcessID == pid
                    && let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                {
                    ResumeThread(thread);
                    CloseHandle(thread).ok();
                }
                found = Thread32Next(snapshot, &mut entry).is_ok();
            }
            CloseHandle(snapshot).ok();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// The processes of one instance

struct Entry {
    stdin: Option<mpsc::UnboundedSender<Vec<u8>>>,
    queued: Arc<AtomicUsize>,
    killer: Option<Arc<Killer>>,
    killed: Arc<AtomicBool>,
}

/// The live processes of one instance, by id, from the import until they exit.
#[derive(Default)]
pub(crate) struct ProcessSet {
    entries: Mutex<HashMap<u64, Entry>>,
}

impl ProcessSet {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Kills `id`'s tree; its `process-exited` says `killed`.
    pub(crate) fn kill(&self, id: u64) {
        if let Some(entry) = self.lock().get(&id) {
            entry.killed.store(true, Ordering::SeqCst);
            if let Some(killer) = &entry.killer {
                killer.kill();
            }
        }
    }

    /// Revocation, a trap, reload, disable: every tree dies.
    pub(crate) fn kill_all(&self) {
        for entry in self.lock().values() {
            entry.killed.store(true, Ordering::SeqCst);
            if let Some(killer) = &entry.killer {
                killer.kill();
            }
        }
    }

    fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }

    fn write(&self, id: u64, data: Vec<u8>) -> Result<(), HostError> {
        let entries = self.lock();
        let Some(entry) = entries.get(&id) else {
            return Err(invalid(format!("process {id} has exited")));
        };
        let Some(stdin) = &entry.stdin else {
            return Err(invalid(format!(
                "process {id} has no stdin open (spawn it with stdin, and do not close it)"
            )));
        };
        let queued = entry.queued.load(Ordering::SeqCst);
        if queued + data.len() > MAX_STDIN_QUEUED_BYTES {
            return Err(HostError {
                code: HostErrorCode::LimitExceeded,
                message: format!("at most {MAX_STDIN_QUEUED_BYTES} bytes queued for stdin"),
            });
        }
        entry.queued.fetch_add(data.len(), Ordering::SeqCst);
        stdin
            .unbounded_send(data)
            .map_err(|_| invalid(format!("process {id}'s stdin is closed")))
    }

    fn close_stdin(&self, id: u64) -> Result<(), HostError> {
        match self.lock().get_mut(&id) {
            Some(entry) => {
                entry.stdin = None;
                Ok(())
            }
            None => Err(invalid(format!("process {id} has exited"))),
        }
    }
}

impl Drop for ProcessSet {
    fn drop(&mut self) {
        // Dropping the killers kills the trees (job handles close; SIGKILL to the groups).
        self.entries
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

/// A spawn the import accepted, started once the call returns successfully.
pub(crate) struct PendingSpawn {
    id: u64,
    request: SpawnRequest,
    spec: Option<ProcessSpec>,
    stdin: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
}

/// The `process` state of one instance, in its store.
pub(crate) struct ProcessState {
    policy: Arc<ProcessPolicy>,
    counters: ProcessCounters,
    pending: Vec<PendingSpawn>,
    set: Arc<ProcessSet>,
    workspace: Option<Arc<dyn Workspace>>,
}

impl ProcessState {
    pub(crate) fn new(
        policy: Arc<ProcessPolicy>,
        set: Arc<ProcessSet>,
        workspace: Option<Arc<dyn Workspace>>,
    ) -> Self {
        Self {
            policy,
            counters: ProcessCounters::default(),
            pending: Vec::new(),
            set,
            workspace,
        }
    }

    /// `process.spawn`: the shared checks, the worktree rules and the cap. The caller has
    /// checked the capability with the broker.
    pub(crate) fn spawn(&mut self, request: SpawnRequest) -> Result<u64, HostError> {
        if self.policy.programs.is_empty() {
            return Err(HostError {
                code: HostErrorCode::NotPermitted,
                message: format!(
                    "spawn needs {CAPABILITY_PROCESS:?} and [[fyx_ui.processes]] in the manifest"
                ),
            });
        }
        let spec = match check_spawn(&request, &self.policy)? {
            SpawnCheck::Spawn(spec) => Some(spec.clone()),
            SpawnCheck::NotDeclared => None,
        };
        let worktrees = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.worktrees())
            .unwrap_or_default();
        if worktrees.is_empty() {
            return Err(HostError {
                code: HostErrorCode::NoWorktree,
                message: "no worktree is open".into(),
            });
        }
        if let Ok((_, worktree)) = resolve_cwd(request.cwd.as_ref(), &worktrees)
            && !worktree.trusted
        {
            return Err(HostError {
                code: HostErrorCode::UntrustedWorktree,
                message: "the worktree is restricted: it is not trusted".into(),
            });
        }
        let id = self.counters.start()?;
        let (sender, receiver) = if request.stdin {
            let (sender, receiver) = mpsc::unbounded();
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        self.set.lock().insert(
            id,
            Entry {
                stdin: sender,
                queued: Arc::new(AtomicUsize::new(0)),
                killer: None,
                killed: Arc::new(AtomicBool::new(false)),
            },
        );
        self.pending.push(PendingSpawn {
            id,
            request,
            spec,
            stdin: receiver,
        });
        Ok(id)
    }

    pub(crate) fn write_stdin(&mut self, id: u64, data: Vec<u8>) -> Result<(), HostError> {
        if data.len() > MAX_STDIN_WRITE_BYTES {
            return Err(invalid(format!(
                "at most {MAX_STDIN_WRITE_BYTES} bytes per write-stdin"
            )));
        }
        self.set.write(id, data)
    }

    pub(crate) fn close_stdin(&mut self, id: u64) -> Result<(), HostError> {
        self.set.close_stdin(id)
    }

    /// `process.kill`: an id that exited (or never was) is `invalid-argument`.
    pub(crate) fn kill(&mut self, id: u64) -> Result<(), HostError> {
        if !self.counters.is_live(id) {
            return Err(invalid(format!("process {id} has exited")));
        }
        self.set.kill(id);
        Ok(())
    }

    /// After a call: the spawns to start if it succeeded; a failed call drops them.
    pub(crate) fn finish_call(&mut self, succeeded: bool) -> Vec<PendingSpawn> {
        let pending = std::mem::take(&mut self.pending);
        if succeeded {
            return pending;
        }
        for spawn in &pending {
            self.counters.finished(spawn.id);
            self.set.remove(spawn.id);
        }
        Vec::new()
    }

    /// Before `handle-io` delivers `process-exited` or `spawn-failed`.
    pub(crate) fn delivered(&mut self, id: u64) {
        self.counters.finished(id);
    }
}

/// Starts accepted spawns, once per instance. Dropping it kills every tree.
pub(crate) struct ProcessHandle {
    extension_id: Arc<str>,
    set: Arc<ProcessSet>,
    broker: Option<Arc<dyn CapabilityCheck>>,
    workspace: Option<Arc<dyn Workspace>>,
    environment: Environment,
    declared_env: Arc<BTreeSet<String>>,
    running: Arc<Running>,
    journal: Option<Arc<dyn crate::journal::Journal>>,
}

impl ProcessHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        set: Arc<ProcessSet>,
        broker: Option<Arc<dyn CapabilityCheck>>,
        workspace: Option<Arc<dyn Workspace>>,
        environment: Option<Environment>,
        declared_env: Arc<BTreeSet<String>>,
        running: Arc<Running>,
        journal: Option<Arc<dyn crate::journal::Journal>>,
    ) -> Self {
        Self {
            journal,
            extension_id,
            set,
            broker,
            workspace,
            environment: environment.unwrap_or_else(|| Arc::new(|| std::env::vars_os().collect())),
            declared_env,
            running,
        }
    }

    pub(crate) fn kill_all(&self) {
        self.set.kill_all();
    }

    /// Starts `pending` on the current tokio runtime, in call order.
    pub(crate) fn start(&self, pending: Vec<PendingSpawn>) {
        for spawn in pending {
            self.running.diagnostics.io_started(
                CAPABILITY_PROCESS,
                spawn.id,
                format!(
                    "{} ({} arguments)",
                    spawn.request.program,
                    spawn.request.args.len()
                ),
            );
            let context = SpawnContext {
                extension_id: self.extension_id.clone(),
                set: self.set.clone(),
                broker: self.broker.clone(),
                workspace: self.workspace.clone(),
                environment: self.environment.clone(),
                declared_env: self.declared_env.clone(),
                running: self.running.clone(),
                journal: self.journal.clone(),
            };
            tokio::spawn(run(spawn, context));
        }
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        self.set.kill_all();
    }
}

struct SpawnContext {
    extension_id: Arc<str>,
    set: Arc<ProcessSet>,
    broker: Option<Arc<dyn CapabilityCheck>>,
    workspace: Option<Arc<dyn Workspace>>,
    environment: Environment,
    declared_env: Arc<BTreeSet<String>>,
    running: Arc<Running>,
    journal: Option<Arc<dyn crate::journal::Journal>>,
}

/// The broker when the spawn is applied: granted runs, `ask` waits for the prompt.
async fn consent(broker: Option<&Arc<dyn CapabilityCheck>>, extension_id: &str) -> bool {
    let Some(broker) = broker else {
        return false;
    };
    match broker.state(extension_id, CAPABILITY_PROCESS) {
        PermissionState::Granted => true,
        PermissionState::Ask => {
            broker.request(extension_id, CAPABILITY_PROCESS).await == PermissionState::Granted
        }
        PermissionState::Denied | PermissionState::Unavailable => false,
    }
}

async fn run(spawn: PendingSpawn, context: SpawnContext) {
    let id = spawn.id;
    let failed = |error: SpawnError| IoEvent::SpawnFailed(id, error);
    let killed = context
        .set
        .lock()
        .get(&id)
        .map(|entry| entry.killed.clone())
        .unwrap_or_default();
    if killed.load(Ordering::SeqCst) {
        context.set.remove(id);
        context
            .running
            .deliver(IoEvent::ProcessExited(ProcessExit {
                process: id,
                code: None,
                killed: true,
            }))
            .await;
        return;
    }
    let outcome = start(spawn, &context, killed.clone()).await;
    if let Err(error) = outcome {
        context.set.remove(id);
        context.running.deliver(failed(error)).await;
    }
}

/// Resolves and starts the program, then pumps its output and reports its exit.
async fn start(
    spawn: PendingSpawn,
    context: &SpawnContext,
    killed: Arc<AtomicBool>,
) -> Result<(), SpawnError> {
    let id = spawn.id;
    let Some(spec) = spawn.spec else {
        return Err(SpawnError::NotFound(format!(
            "{} is not in the manifest's [[fyx_ui.processes]]",
            spawn.request.program
        )));
    };
    if !consent(context.broker.as_ref(), &context.extension_id).await {
        return Err(SpawnError::Denied);
    }
    // 0.4 (§C.2): the program's own checkbox.
    let item = crate::permissions::ConsentItem::Program {
        capability: CAPABILITY_PROCESS.into(),
        name: spec.name.clone(),
    };
    if context.broker.as_ref().is_some_and(|broker| {
        matches!(
            broker.item_state(&context.extension_id, &item),
            PermissionState::Denied | PermissionState::Unavailable
        )
    }) {
        return Err(SpawnError::Denied);
    }
    let worktrees = context
        .workspace
        .as_ref()
        .map(|workspace| workspace.worktrees())
        .unwrap_or_default();
    let (cwd, worktree) = resolve_cwd(spawn.request.cwd.as_ref(), &worktrees)?;
    if !worktree.trusted {
        return Err(SpawnError::Denied);
    }
    let fyx_environment = (context.environment)();
    let program = match &spec.path {
        Some(relative) => resolve_in_worktree(&worktree.root, relative),
        None => {
            let roots: Vec<PathBuf> = worktrees
                .iter()
                .filter_map(|worktree| canonical(&worktree.root))
                .collect();
            resolve_on_path(&spec.name, &path_value(&fyx_environment), &roots)
        }
    }
    .map_err(SpawnError::NotFound)?;
    let roots: Vec<PathBuf> = worktrees
        .iter()
        .filter_map(|worktree| canonical(&worktree.root))
        .collect();
    let environment =
        program_environment(&fyx_environment, &context.declared_env, &spawn.request.env)
            .into_iter()
            .map(|(name, value)| {
                if name.to_str().is_some_and(|name| same_name(name, "PATH")) {
                    let value = filtered_path(&value, &roots);
                    (name, value)
                } else {
                    (name, value)
                }
            })
            .collect::<Vec<_>>();

    let mut command = tokio::process::Command::new(&program);
    command
        .args(&spawn.request.args)
        .current_dir(&cwd)
        .env_clear()
        .envs(environment)
        .stdin(if spawn.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(job::CREATE_SUSPENDED | job::CREATE_NO_WINDOW);
    #[cfg(unix)]
    {
        command.process_group(0);
        #[cfg(target_os = "linux")]
        // SAFETY: prctl is async-signal-safe; it only sets this child's parent-death signal.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    let mut child = command
        .spawn()
        .map_err(|error| SpawnError::Io(format!("{}: {error}", program.display())))?;
    if let Some(journal) = &context.journal {
        journal.record(
            &context.extension_id,
            crate::journal::JournalKind::ProgramStarted {
                capability: CAPABILITY_PROCESS.into(),
                program: spec.name.clone(),
                args: spawn.request.args.clone(),
                cwd: Some(cwd.display().to_string()),
            },
        );
    }
    let killer = Arc::new(make_killer(&child)?);
    {
        let mut entries = context.set.lock();
        match entries.get_mut(&id) {
            Some(entry) => entry.killer = Some(killer.clone()),
            // The instance went away while it started: the killer's drop ends it.
            None => return Ok(()),
        }
    }
    if killed.load(Ordering::SeqCst) {
        killer.kill();
    }

    if let (Some(mut stdin_rx), Some(mut stdin)) = (spawn.stdin, child.stdin.take()) {
        let queued = context
            .set
            .lock()
            .get(&id)
            .map(|entry| entry.queued.clone())
            .unwrap_or_default();
        tokio::spawn(async move {
            while let Some(data) = stdin_rx.next().await {
                let length = data.len();
                let written = stdin.write_all(&data).await;
                queued.fetch_sub(length, Ordering::SeqCst);
                if written.is_err() {
                    break;
                }
            }
        });
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let pump = |reader: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>,
                stream: OutputStream| {
        let running = context.running.clone();
        async move {
            let Some(mut reader) = reader else {
                return;
            };
            let mut buffer = vec![0u8; MAX_OUTPUT_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) | Err(_) => return,
                    // Delivering waits while the `handle-io` queue is full: backpressure.
                    Ok(read) => {
                        running
                            .deliver(IoEvent::ProcessOutput(ProcessOutput {
                                process: id,
                                stream,
                                data: buffer[..read].to_vec(),
                            }))
                            .await
                    }
                }
            }
        }
    };
    futures::join!(
        pump(stdout.map(|out| Box::new(out) as _), OutputStream::Stdout),
        pump(stderr.map(|err| Box::new(err) as _), OutputStream::Stderr),
    );
    let status = child.wait().await;
    let was_killed = killed.load(Ordering::SeqCst);
    context.set.remove(id);
    context
        .running
        .deliver(IoEvent::ProcessExited(ProcessExit {
            process: id,
            code: if was_killed {
                None
            } else {
                status.ok().and_then(|status| status.code())
            },
            killed: was_killed,
        }))
        .await;
    Ok(())
}

#[cfg(windows)]
fn make_killer(child: &tokio::process::Child) -> Result<Killer, SpawnError> {
    let io = |error: windows::core::Error| SpawnError::Io(format!("job object: {error}"));
    let job = job::JobObject::new().map_err(io)?;
    let handle = child
        .raw_handle()
        .ok_or_else(|| SpawnError::Io("the process exited at once".into()))?;
    // Assigned while still suspended, so nothing it starts escapes the job; then resumed.
    job.assign(handle).map_err(io)?;
    let pid = child
        .id()
        .ok_or_else(|| SpawnError::Io("the process exited at once".into()))?;
    job::resume(pid).map_err(io)?;
    Ok(Killer { job })
}

#[cfg(unix)]
fn make_killer(child: &tokio::process::Child) -> Result<Killer, SpawnError> {
    let pid = child
        .id()
        .ok_or_else(|| SpawnError::Io("the process exited at once".into()))?;
    Ok(Killer { group: pid as i32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_planted_program_in_the_worktree_is_never_found() {
        // Spec §4 failure 4: the worktree has its own `hostname.exe`, and both the worktree and
        // a relative `.` come first on PATH. The system one is found.
        let worktree = tempfile::tempdir().expect("temp dir");
        let system =
            PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot")).join("System32");
        std::fs::copy(
            system.join("whoami.exe"),
            worktree.path().join("hostname.exe"),
        )
        .expect("plants a program");
        let path = std::env::join_paths([
            worktree.path().to_path_buf(),
            PathBuf::from("."),
            PathBuf::new(),
            system.clone(),
        ])
        .expect("PATH");
        let roots = vec![canonical(worktree.path()).expect("canonical")];
        let found = resolve_on_path("hostname", &path, &roots).expect("found");
        let expected = canonical(&system.join("hostname.exe")).expect("canonical");
        assert!(
            found
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.to_string_lossy()),
            "{found:?}"
        );
        // Without the worktree filter the planted one would win.
        assert!(
            resolve_on_path("hostname", &path, &[])
                .expect("found")
                .starts_with(canonical(worktree.path()).expect("canonical"))
        );
        // Batch scripts never run.
        std::fs::write(worktree.path().join("tool.cmd"), "@echo off").expect("writes");
        assert!(resolve_on_path("tool.cmd", &path, &[]).is_err());
        assert!(resolve_in_worktree(worktree.path(), "tool.cmd").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_program_in_the_worktree_is_never_found() {
        use std::os::unix::fs::PermissionsExt as _;
        let worktree = tempfile::tempdir().expect("temp dir");
        let planted = worktree.path().join("sh");
        std::fs::write(&planted, "#!/bin/sh\necho planted\n").expect("plants");
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let path = std::env::join_paths([
            worktree.path().to_path_buf(),
            PathBuf::from("."),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/bin"),
        ])
        .expect("PATH");
        let roots = vec![canonical(worktree.path()).expect("canonical")];
        let found = resolve_on_path("sh", &path, &roots).expect("found");
        assert!(!found.starts_with(&roots[0]), "{found:?}");
    }

    #[test]
    fn the_environment_is_filtered() {
        let fyx: Vec<(OsString, OsString)> = [
            ("PATH", "/bin"),
            ("HOME", "/home/me"),
            ("LANG", "C"),
            ("LC_ALL", "C"),
            ("SECRET_TOKEN", "x"),
            ("NO_COLOR", "1"),
            ("LD_PRELOAD", "/evil.so"),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
        let declared: BTreeSet<String> =
            ["NO_COLOR".into(), "LD_PRELOAD".into(), "GIT_DIR".into()].into();
        let env = program_environment(
            &fyx,
            &declared,
            &[
                ("GIT_DIR".into(), ".git".into()),
                ("LD_PRELOAD".into(), "/x".into()),
            ],
        );
        let names: BTreeSet<String> = env
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        // `GIT_DIR` is declared but forbidden (integration-5 S6), so neither Fyx's value nor the
        // guest's reaches the program.
        assert_eq!(
            names,
            ["HOME", "LANG", "LC_ALL", "NO_COLOR", "PATH"]
                .map(String::from)
                .into()
        );
    }

    /// integration-5 S6: the program's PATH has no empty, relative or worktree entries.
    #[test]
    fn the_programs_path_leaves_out_the_worktrees() {
        let worktree = tempfile::tempdir().expect("a worktree");
        let outside = tempfile::tempdir().expect("a directory");
        let root = canonical(worktree.path()).expect("canonical");
        let bin = worktree.path().join("node_modules").join(".bin");
        std::fs::create_dir_all(&bin).expect("bin");
        let path = std::env::join_paths([
            bin,
            PathBuf::from("relative"),
            outside.path().to_path_buf(),
            PathBuf::new(),
        ])
        .expect("joins");
        let filtered = filtered_path(&path, &[root]);
        let kept: Vec<PathBuf> = std::env::split_paths(&filtered).collect();
        assert_eq!(kept, [outside.path().to_path_buf()]);
    }
}
