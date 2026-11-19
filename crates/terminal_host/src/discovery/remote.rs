//! Plan 5.17: discovery files and listeners for tools in an SSH project's terminals.
//!
//! For each SSH host with a project open, one `ssh` process runs while a listener with discovery
//! files is open: `-R <port>:127.0.0.1:<port>` for each such listener (the same port on both
//! ends, so a listener's `Host: 127.0.0.1:<port>` check holds), and a script that writes the
//! files on the host and removes them when its stdin closes (Fyx closes it, quits or dies).
//!
//! - **No secret on a command line.** The script names folders, file names and ports only; the
//!   files' contents (with the listener's token) are its stdin, one JSON line per file, read
//!   with the shell's own `read` and `printf`.
//! - **No file for someone else's port.** `ExitOnForwardFailure=yes`: if another user of the
//!   host holds a port, `ssh` exits before the script runs, so no token goes to that port.
//! - **Files are 0600 in folders the user owns** (`umask 077`); a folder under the temp dir must
//!   not be a symlink; a file is written beside its final name and renamed over it.
//! - **`${pid}` is 0.** Fyx's pid means nothing on the host. Claude Code (2.1.x) removes a lock
//!   file whose pid is not running, but keeps one without a pid while its port answers, and
//!   checks the pid against its parents only inside VS Code-like and JetBrains terminals. Gemini
//!   CLI only prefers a file with its IDE's pid and otherwise matches the workspace.
//! - `${tmp}` and `${uid}` in content are this machine's: such a file is not written remotely.
//!
//! Remote shells get the env of extensions allowed into every new terminal, without tokens,
//! account dirs, package paths or the relay; `${listen.<name>.port}` only while this host's
//! tunnel forwards that port ([`forwarded_port`]).

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use gpui::{App, EntityId, Global, WeakEntity};
use project::Project;
use project::fyx_remote::TunnelCommand;

use super::fill::{self, RemoteBase, Vars};
use super::{GlobalDiscovery, OpenListener};

/// What the script prints once every file is written.
const READY: &str = "fyx-tunnel-ready";
/// A tunnel that ended is started again on a later change at most this often.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long a stopped tunnel gets to remove its files before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);

/// One discovery file on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RemoteFile {
    base: RemoteBase,
    folders: Vec<String>,
    name: String,
    /// The content as one JSON line (the token is in here: stdin only).
    line: String,
}

/// What one host's tunnel forwards and writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    ports: Vec<u16>,
    files: Vec<RemoteFile>,
}

impl Plan {
    /// Everything but the contents, which hold the tokens (they change only with the port).
    fn fingerprint(&self) -> String {
        let files = self
            .files
            .iter()
            .map(|file| {
                format!(
                    "{:?}/{}/{}#{}",
                    file.base,
                    file.folders.join("/"),
                    file.name,
                    file.line.len()
                )
            })
            .collect::<Vec<_>>();
        format!("{:?} {files:?}", self.ports)
    }
}

/// The files of every open listener for a host whose projects have `worktrees` (remote paths).
pub(crate) fn plan(open: &[(String, OpenListener)], worktrees: &[String]) -> Plan {
    let mut plan = Plan::default();
    for (ext, listener) in open {
        let mut any = false;
        for file in &listener.files {
            match remote_file(listener, file, worktrees) {
                Ok(Some(file)) => {
                    plan.files.push(file);
                    any = true;
                }
                Ok(None) => {}
                Err(error) => log::debug!(
                    "fyx: no remote discovery file {:?} of {ext}/{}: {error}",
                    file.name,
                    listener.name
                ),
            }
        }
        if any && !plan.ports.contains(&listener.port) {
            plan.ports.push(listener.port);
        }
    }
    plan
}

fn remote_file(
    listener: &OpenListener,
    file: &super::DiscoveryFile,
    worktrees: &[String],
) -> Result<Option<RemoteFile>, String> {
    if fill::uses_local_values(&file.content) {
        return Err("it uses ${tmp} or ${uid} of this machine".into());
    }
    let vars = Vars {
        port: listener.port,
        token: listener.token.clone(),
        pid: 0,
        worktrees: worktrees.to_vec(),
        uid: None,
        tmp: String::new(),
        windows: false,
    };
    let content = fill::fill(&file.content, &vars)?;
    let name = fill::file_name(&file.name, &vars)?;
    let Some((base, folders)) = fill::remote_target(&file.dir, file.default_dir.as_deref())? else {
        return Ok(None);
    };
    let line = serde_json::to_string(&content).map_err(|error| error.to_string())?;
    Ok(Some(RemoteFile {
        base,
        folders,
        name,
        line,
    }))
}

/// `text` as one single-quoted POSIX shell word.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn base_word(base: RemoteBase) -> &'static str {
    match base {
        RemoteBase::Home => "\"$HOME\"",
        RemoteBase::Tmp => "\"${TMPDIR:-/tmp}\"",
    }
}

/// The POSIX `sh` script: reads one line per file from stdin and writes it, prints [`READY`],
/// then waits for stdin to close and removes what it wrote. No content is in the script.
pub(crate) fn script(plan: &Plan) -> String {
    let mut out = String::from(concat!(
        "umask 077\n",
        // `mk <h|t> <dir>`: the folder exists (made here or before), is a folder the user owns,
        // and under the temp dir is not a symlink.
        "mk() { mkdir \"$2\" 2>/dev/null; [ -d \"$2\" ] && [ -O \"$2\" ] && ",
        "{ [ \"$1\" = h ] || [ ! -h \"$2\" ]; }; }\n",
        // `put <line> <file>`: written beside the file, never through an existing name, then
        // renamed over it.
        "put() { rm -f \"$2.$$\" && (set -C; printf '%s\\n' \"$1\" > \"$2.$$\") && ",
        "mv -f \"$2.$$\" \"$2\" || { rm -f \"$2.$$\"; false; }; }\n",
    ));
    let mut cleanup = Vec::new();
    for (index, file) in plan.files.iter().enumerate() {
        let mut path = base_word(file.base).to_owned();
        for folder in &file.folders {
            path.push('/');
            path.push_str(&quote(folder));
        }
        out.push_str(&format!(
            "d{index}={path}\nf{index}=\"$d{index}\"/{}\n",
            quote(&file.name)
        ));
        cleanup.push(format!("[ -z \"$w{index}\" ] || rm -f \"$f{index}\""));
    }
    out.push_str(&format!("trap '{}' EXIT\n", cleanup.join("; ")));
    out.push_str("trap 'exit 1' HUP INT TERM PIPE\n");
    for (index, file) in plan.files.iter().enumerate() {
        out.push_str("IFS= read -r l || exit 1\n");
        let kind = match file.base {
            RemoteBase::Home => "h",
            RemoteBase::Tmp => "t",
        };
        let mut steps = Vec::new();
        let mut path = base_word(file.base).to_owned();
        for folder in &file.folders {
            path.push('/');
            path.push_str(&quote(folder));
            steps.push(format!("mk {kind} {path}"));
        }
        steps.push(format!("put \"$l\" \"$f{index}\""));
        out.push_str(&format!("{} && w{index}=1\n", steps.join(" && ")));
    }
    out.push_str(&format!("l=\necho {READY}\ncat >/dev/null\n"));
    out
}

/// The script's stdin: one line per file, in order.
fn stdin_of(plan: &Plan) -> Vec<u8> {
    let mut stdin = Vec::new();
    for file in &plan.files {
        stdin.extend_from_slice(file.line.as_bytes());
        stdin.push(b'\n');
    }
    stdin
}

#[derive(Default)]
struct State {
    ready: AtomicBool,
    ended: AtomicBool,
}

/// A running tunnel. Dropping it closes the script's stdin (its files go) and stops `ssh`.
struct Tunnel {
    fingerprint: String,
    ports: Vec<u16>,
    started: Instant,
    state: Arc<State>,
    /// Dropped to stop the tunnel.
    _stop: mpsc::Sender<()>,
    /// Answers once the process is gone.
    gone: Option<mpsc::Receiver<()>>,
}

impl Tunnel {
    /// Starts `command` once `previous` (the host's tunnel before) is gone, so its forwards are
    /// free again.
    fn start(command: TunnelCommand, plan: &Plan, previous: Option<Tunnel>) -> Self {
        let state = Arc::new(State::default());
        let (stop, stopped) = mpsc::channel();
        let (gone_tx, gone) = mpsc::channel();
        let previous_gone = previous.and_then(|mut previous| previous.gone.take());
        let stdin = stdin_of(plan);
        let thread_state = state.clone();
        let spawned = std::thread::Builder::new()
            .name("fyx-ssh-tunnel".into())
            .spawn(move || {
                if let Some(previous_gone) = previous_gone {
                    previous_gone.recv_timeout(STOP_GRACE * 2).ok();
                }
                if let Err(error) = run(command, &stdin, &thread_state, &stopped) {
                    log::warn!("fyx: SSH tunnel: {error}");
                }
                thread_state.ended.store(true, Ordering::SeqCst);
                gone_tx.send(()).ok();
            });
        if let Err(error) = spawned {
            log::warn!("fyx: SSH tunnel thread: {error}");
            state.ended.store(true, Ordering::SeqCst);
        }
        Self {
            fingerprint: plan.fingerprint(),
            ports: plan.ports.clone(),
            started: Instant::now(),
            state,
            _stop: stop,
            gone: Some(gone),
        }
    }

    fn ready(&self) -> bool {
        self.state.ready.load(Ordering::SeqCst) && !self.state.ended.load(Ordering::SeqCst)
    }

    fn ended(&self) -> bool {
        self.state.ended.load(Ordering::SeqCst)
    }
}

/// Runs the tunnel until `stopped` (its sender dropped) or `ssh` exits.
#[allow(
    clippy::disallowed_methods,
    reason = "on the tunnel's own thread, which may block"
)]
fn run(
    command: TunnelCommand,
    stdin: &[u8],
    state: &Arc<State>,
    stopped: &mpsc::Receiver<()>,
) -> Result<(), String> {
    let mut child = util::command::new_std_command(&command.program)
        .args(&command.args)
        .envs(command.env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("starting {}: {error}", command.program))?;
    let mut input = child.stdin.take();
    if let Some(input) = input.as_mut()
        && let Err(error) = input.write_all(stdin).and_then(|()| input.flush())
    {
        log::warn!("fyx: SSH tunnel stdin: {error}");
    }
    if let Some(stdout) = child.stdout.take() {
        let state = state.clone();
        std::thread::Builder::new()
            .name("fyx-ssh-tunnel-out".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    match line {
                        Ok(line) if line.trim() == READY => {
                            state.ready.store(true, Ordering::SeqCst)
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                state.ended.store(true, Ordering::SeqCst);
            })
            .ok();
    }
    if let Some(stderr) = child.stderr.take() {
        std::thread::Builder::new()
            .name("fyx-ssh-tunnel-err".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    log::info!("fyx: SSH tunnel: {line}");
                }
            })
            .ok();
    }
    // Wait for the stop, checking now and then whether `ssh` ended on its own.
    while let Err(mpsc::RecvTimeoutError::Timeout) =
        stopped.recv_timeout(Duration::from_millis(500))
    {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("ssh exited with {status}"));
        }
    }
    drop(input);
    stop(&mut child);
    Ok(())
}

/// Gives the script [`STOP_GRACE`] to remove its files after its stdin closed, then kills `ssh`.
fn stop(child: &mut Child) {
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill().ok();
    child.wait().ok();
}

/// The tunnels, by host.
#[derive(Default)]
struct Tunnels {
    by_host: HashMap<String, Tunnel>,
    /// Each remote client's host, for [`forwarded_port`].
    hosts: HashMap<EntityId, String>,
}

impl Global for Tunnels {}

/// Makes the tunnels what the open SSH projects and listeners need: one per host with a
/// discovery file to write, restarted when what it writes changes.
pub(crate) fn refresh(cx: &mut App) {
    let Some(discovery) = cx.try_global::<GlobalDiscovery>() else {
        return;
    };
    let open = discovery.writer.open_listeners();
    let projects = discovery
        .projects
        .iter()
        .filter_map(WeakEntity::upgrade)
        .collect::<Vec<_>>();
    // Each SSH host's projects (any one builds the command) and worktrees.
    let mut hosts = BTreeMap::<String, (Vec<gpui::Entity<Project>>, Vec<String>)>::new();
    let mut by_client = HashMap::new();
    for project in projects {
        let Some((client, host)) = project::fyx_remote::ssh_host(project.read(cx), cx) else {
            continue;
        };
        by_client.insert(client, host.clone());
        let (host_projects, worktrees) = hosts.entry(host).or_default();
        for worktree in project.read(cx).visible_worktrees(cx) {
            let root = worktree.read(cx).abs_path().to_string_lossy().into_owned();
            if !worktrees.contains(&root) {
                worktrees.push(root);
            }
        }
        host_projects.push(project);
    }

    let mut tunnels = Tunnels::default();
    let mut previous = if cx.has_global::<Tunnels>() {
        std::mem::take(&mut cx.global_mut::<Tunnels>().by_host)
    } else {
        HashMap::new()
    };
    for (host, (projects, worktrees)) in hosts {
        let plan = plan(&open, &worktrees);
        let before = previous.remove(&host);
        if plan.files.is_empty() || worktrees.is_empty() {
            continue;
        }
        if let Some(before) = before {
            let fresh = before.fingerprint == plan.fingerprint()
                && (!before.ended() || before.started.elapsed() < RETRY_AFTER);
            if fresh {
                tunnels.by_host.insert(host, before);
                continue;
            }
            if let Some(tunnel) = start(&projects, &plan, Some(before), cx) {
                tunnels.by_host.insert(host, tunnel);
            }
        } else if let Some(tunnel) = start(&projects, &plan, None, cx) {
            tunnels.by_host.insert(host, tunnel);
        }
    }
    // Hosts no longer open: their tunnels drop here, which removes their files.
    drop(previous);
    tunnels.hosts = by_client;
    cx.set_global(tunnels);
}

fn start(
    projects: &[gpui::Entity<Project>],
    plan: &Plan,
    previous: Option<Tunnel>,
    cx: &App,
) -> Option<Tunnel> {
    let project = projects.first()?.read(cx);
    match project::fyx_remote::ssh_tunnel_command(project, &script(plan), &plan.ports, cx)? {
        Ok(command) => Some(Tunnel::start(command, plan, previous)),
        Err(error) => {
            log::warn!("fyx: SSH tunnel command: {error}");
            None
        }
    }
}

/// `ext`'s listener `name`'s port, if the tunnel of `remote`'s host forwards it now.
pub(crate) fn forwarded_port(remote: EntityId, ext: &str, name: &str, cx: &App) -> Option<u16> {
    let tunnels = cx.try_global::<Tunnels>()?;
    let tunnel = tunnels.by_host.get(tunnels.hosts.get(&remote)?)?;
    let (port, _) = super::discovery_writer(cx)?.listener(ext, name)?;
    (tunnel.ready() && tunnel.ports.contains(&port)).then_some(port)
}

/// What [`close_all`] stopped, to wait for.
pub(crate) struct Closing(Vec<mpsc::Receiver<()>>);

impl Closing {
    /// Waits at most `timeout` in all for the tunnels to remove their files.
    pub(crate) fn wait(self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        for gone in self.0 {
            gone.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .ok();
        }
    }
}

/// On quit: stops every tunnel.
pub(crate) fn close_all(cx: &mut App) -> Closing {
    if !cx.has_global::<Tunnels>() {
        return Closing(Vec::new());
    }
    let tunnels = cx.global_mut::<Tunnels>();
    Closing(
        tunnels
            .by_host
            .drain()
            .filter_map(|(_, mut tunnel)| tunnel.gone.take())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::*;
    use crate::discovery::DiscoveryFile;

    const TOKEN: &str = "tok-'secret\"$(x)";

    fn claude_lock() -> DiscoveryFile {
        DiscoveryFile {
            dir: "${account.claude}/ide".into(),
            default_dir: Some("~/.claude/ide".into()),
            name: "${port}.lock".into(),
            content: json!({ "pid": "${pid}", "workspaceFolders": "${worktrees}",
                "ideName": "Fyx", "transport": "ws", "authToken": "${token}" }),
        }
    }

    fn gemini_file() -> DiscoveryFile {
        DiscoveryFile {
            dir: "${tmp}/gemini/ide".into(),
            default_dir: None,
            name: "gemini-ide-server-${pid}-${port}.json".into(),
            content: json!({ "port": "${port}", "workspacePath": "${worktrees.joined}",
                "authToken": "${token}" }),
        }
    }

    fn listener(name: &str, port: u16, files: Vec<DiscoveryFile>) -> OpenListener {
        OpenListener {
            name: name.into(),
            port,
            token: TOKEN.into(),
            files,
        }
    }

    fn open() -> Vec<(String, OpenListener)> {
        let local_only = DiscoveryFile {
            content: json!({ "dir": "${tmp}" }),
            ..gemini_file()
        };
        vec![
            (
                "fyx-claude-code".into(),
                listener("ide", 4100, vec![claude_lock()]),
            ),
            (
                "fyx-gemini-cli".into(),
                listener("ide", 4300, vec![gemini_file()]),
            ),
            ("fyx-other".into(), listener("ide", 4500, vec![local_only])),
            ("fyx-none".into(), listener("ide", 4700, Vec::new())),
        ]
    }

    #[test]
    fn the_plan_has_pid_0_remote_worktrees_and_only_hosts_files() {
        let plan = plan(&open(), &["/srv/app".into(), "/srv/lib".into()]);
        assert_eq!(plan.ports, vec![4100, 4300], "no file, no forward");
        assert_eq!(plan.files.len(), 2);
        let claude = &plan.files[0];
        assert_eq!(
            (claude.base, claude.folders.clone(), claude.name.as_str()),
            (
                RemoteBase::Home,
                vec![".claude".into(), "ide".into()],
                "4100.lock"
            ),
            "no accounts on the host: the default dir"
        );
        let lock: Value = serde_json::from_str(&claude.line).unwrap();
        assert_eq!(lock["pid"], json!(0));
        assert_eq!(lock["workspaceFolders"], json!(["/srv/app", "/srv/lib"]));
        assert_eq!(lock["authToken"], json!(TOKEN));
        let gemini = &plan.files[1];
        assert_eq!(
            (gemini.base, gemini.name.as_str()),
            (RemoteBase::Tmp, "gemini-ide-server-0-4300.json")
        );
        let file: Value = serde_json::from_str(&gemini.line).unwrap();
        assert_eq!(file["workspacePath"], json!("/srv/app:/srv/lib"));
        assert!(!claude.line.contains('\n') && !gemini.line.contains('\n'));
    }

    #[test]
    fn no_token_is_on_the_command_line() {
        let plan = plan(&open(), &["/srv/app".into()]);
        let script = script(&plan);
        assert!(!script.contains("tok-"), "{script}");
        let stdin = String::from_utf8(stdin_of(&plan)).unwrap();
        assert_eq!(stdin.matches("tok-").count(), 2);
        assert!(script.starts_with("umask 077\n"));
        let same = super::plan(&open(), &["/srv/app".into()]);
        let other = super::plan(&open(), &["/srv/b".into()]);
        assert_eq!(plan.fingerprint(), same.fingerprint());
        assert_ne!(plan.fingerprint(), other.fingerprint());
    }

    #[test]
    fn folder_and_file_names_are_quoted() {
        assert_eq!(quote("it's"), r"'it'\''s'");
        let mut file = claude_lock();
        file.default_dir = Some("~/.it's $(here)/ide".into());
        let plan = plan(
            &[("x".into(), listener("ide", 4100, vec![file]))],
            &["/w".into()],
        );
        let script = script(&plan);
        assert!(script.contains(r"'.it'\''s $(here)'"), "{script}");
    }

    #[allow(clippy::disallowed_methods, reason = "a test probe")]
    fn posix_sh() -> Option<PathBuf> {
        let candidates = [
            PathBuf::from("sh"),
            PathBuf::from(r"C:\Program Files\Git\usr\bin\sh.exe"),
            PathBuf::from(r"C:\Program Files\Git\bin\sh.exe"),
        ];
        candidates.into_iter().find(|sh| {
            std::process::Command::new(sh)
                .args(["-c", "exit 0"])
                .stdin(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    fn sh_command(sh: &Path, plan: &Plan, home: &Path, tmp: &Path) -> TunnelCommand {
        TunnelCommand {
            remote: EntityId::from(1u64),
            program: sh.to_string_lossy().into_owned(),
            args: vec!["-c".into(), script(plan)],
            env: vec![
                ("HOME".into(), home.to_string_lossy().into_owned()),
                ("TMPDIR".into(), tmp.to_string_lossy().into_owned()),
            ],
        }
    }

    fn wait_for(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// `sh` stands in for `ssh` after its forwards are set up: the script writes the files from
    /// stdin, says it is ready, and removes them when the tunnel stops.
    #[test]
    fn the_tunnel_writes_the_files_while_it_runs() {
        let Some(sh) = posix_sh() else {
            eprintln!("no POSIX sh: skipped");
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let (home, tmp) = (root.path().join("home"), root.path().join("tmp"));
        std::fs::create_dir_all(home.join(".claude/ide")).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        // Someone else's file in the same folder stays.
        std::fs::write(home.join(".claude/ide/9999.lock"), "{}").unwrap();
        let plan = plan(&open(), &["/srv/app".into()]);
        let lock = home.join(".claude/ide/4100.lock");
        let gemini = tmp.join("gemini/ide/gemini-ide-server-0-4300.json");

        let tunnel = Tunnel::start(sh_command(&sh, &plan, &home, &tmp), &plan, None);
        wait_for("ready", || tunnel.ready());
        assert_eq!(read_json(&lock)["authToken"], json!(TOKEN));
        assert_eq!(read_json(&lock)["pid"], json!(0));
        assert_eq!(read_json(&gemini)["port"], json!(4300));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for path in [&lock, &gemini] {
                let mode = std::fs::metadata(path).unwrap().permissions().mode();
                assert_eq!(mode & 0o077, 0, "{} is {mode:o}", path.display());
            }
        }

        // A new plan (another worktree) replaces the tunnel once the old one is gone.
        let next = super::plan(&open(), &["/srv/other".into()]);
        let replaced = Tunnel::start(sh_command(&sh, &next, &home, &tmp), &next, Some(tunnel));
        wait_for("the new tunnel", || replaced.ready());
        assert_eq!(read_json(&lock)["workspaceFolders"], json!(["/srv/other"]));

        let Tunnel { gone, _stop, .. } = replaced;
        let closing = Closing(gone.into_iter().collect());
        drop(_stop);
        closing.wait(Duration::from_secs(20));
        assert!(!lock.exists() && !gemini.exists(), "removed on stop");
        assert!(home.join(".claude/ide/9999.lock").exists());
    }

    /// Under the temp dir a symlinked folder is refused (someone else may own its target); the
    /// other files are still written.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_temp_folder_gets_no_file() {
        let Some(sh) = posix_sh() else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let (home, tmp) = (root.path().join("home"), root.path().join("tmp"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(root.path().join("elsewhere/ide")).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::os::unix::fs::symlink(root.path().join("elsewhere"), tmp.join("gemini")).unwrap();
        let plan = plan(&open(), &["/srv/app".into()]);
        let tunnel = Tunnel::start(sh_command(&sh, &plan, &home, &tmp), &plan, None);
        wait_for("ready", || tunnel.ready());
        assert!(home.join(".claude/ide/4100.lock").exists());
        let elsewhere = std::fs::read_dir(root.path().join("elsewhere/ide")).unwrap();
        assert_eq!(elsewhere.count(), 0);
    }

    #[test]
    fn a_tunnel_whose_ssh_fails_ends_and_is_not_ready() {
        let plan = plan(&open(), &["/srv/app".into()]);
        let command = TunnelCommand {
            remote: EntityId::from(1u64),
            program: "fyx-no-such-ssh".into(),
            args: Vec::new(),
            env: Vec::new(),
        };
        let tunnel = Tunnel::start(command, &plan, None);
        wait_for("the end", || tunnel.ended());
        assert!(!tunnel.ready());
    }
}
