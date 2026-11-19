//! Created terminals (`fyx:ui` §B.3): a declared program in a real pty, as an extension's own
//! terminal. The extension reads the raw output from the first byte: the host makes the terminal's
//! `TerminalExt` with a [`RawTap`] and parks it, and `TerminalBuilder` picks it up before it opens
//! the pty. The output goes to the extension in chunks of at most 64 KiB, at most 30 a second,
//! from a 1 MiB buffer that drops the oldest bytes. The program's process tree dies with the
//! terminal (reload, disable, uninstall, revocation, or the user closing the tab).

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::{App, AppContext as _, BackgroundExecutor, Global};
use project::trusted_worktrees::TrustedWorktrees;
use settings::Settings as _;
use terminal::terminal_settings::TerminalSettings;
use terminal::{TerminalBuilder, TerminalMode};
use terminal_ext::{ParkedTerminal, RawTap, TerminalId};
use ui_ext::terminal::{
    CreateCommand, MAX_OUTPUT_BUFFER_BYTES, MAX_OUTPUT_CHUNK_BYTES, MAX_OUTPUT_EVENTS_PER_SECOND,
    TerminalEvent, TerminalHandle, TerminalSink,
};
use util::paths::PathStyle;
use workspace::MultiWorkspace;

use crate::pseudo;

pub(crate) const MIN_OUTPUT_INTERVAL: Duration =
    Duration::from_nanos(1_000_000_000 / MAX_OUTPUT_EVENTS_PER_SECOND as u64);

/// A created terminal's output on its way to the extension.
#[derive(Debug, Default)]
pub(crate) struct OutputBuffer {
    pending: VecDeque<u8>,
    /// Dropped since the last `Output` event.
    dropped: u64,
    /// The program is gone, with its exit code if it had one.
    finished: Option<Option<i32>>,
    exit_sent: bool,
    last_sent: Option<Instant>,
}

impl OutputBuffer {
    /// Beyond [`MAX_OUTPUT_BUFFER_BYTES`] the oldest output goes.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if self.finished.is_some() {
            return;
        }
        self.pending.extend(bytes);
        let excess = self.pending.len().saturating_sub(MAX_OUTPUT_BUFFER_BYTES);
        if excess > 0 {
            self.pending.drain(..excess);
            self.dropped += excess as u64;
        }
    }

    /// The first call counts; output after it is ignored.
    pub(crate) fn finish(&mut self, code: Option<i32>) {
        self.finished.get_or_insert(code);
    }

    /// Whether `Exited` went out: nothing follows it.
    pub(crate) fn is_done(&self) -> bool {
        self.exit_sent
    }

    /// How long to wait before [`Self::take`] has something, if anything is due.
    pub(crate) fn wait(&self, now: Instant) -> Option<Duration> {
        if !self.pending.is_empty() {
            let since = self.last_sent.map_or(MIN_OUTPUT_INTERVAL, |last| {
                now.saturating_duration_since(last)
            });
            return Some(MIN_OUTPUT_INTERVAL.saturating_sub(since));
        }
        // The exit follows the last output at once.
        (self.finished.is_some() && !self.exit_sent).then_some(Duration::ZERO)
    }

    /// The events due at `now`: one chunk of output, or the exit once all output went out.
    pub(crate) fn take(&mut self, handle: TerminalHandle, now: Instant) -> Option<TerminalEvent> {
        if self.wait(now) != Some(Duration::ZERO) {
            return None;
        }
        if self.pending.is_empty() {
            self.exit_sent = true;
            return Some(TerminalEvent::Exited(handle, self.finished.flatten()));
        }
        let end = self.pending.len().min(MAX_OUTPUT_CHUNK_BYTES);
        let chunk: Vec<u8> = self.pending.drain(..end).collect();
        self.last_sent = Some(now);
        Some(TerminalEvent::Output(
            handle,
            chunk,
            std::mem::take(&mut self.dropped),
        ))
    }
}

/// The [`RawTap`] on a created terminal's pty: queues on the IO thread and wakes the pump.
pub(crate) struct OutputTap {
    buffer: Mutex<OutputBuffer>,
    wake: UnboundedSender<()>,
    /// Set while a wake-up is queued, so a flood of reads queues one.
    queued: AtomicBool,
}

impl OutputTap {
    pub(crate) fn new() -> (Arc<Self>, UnboundedReceiver<()>) {
        let (wake, woken) = futures::channel::mpsc::unbounded();
        let tap = Arc::new(Self {
            buffer: Mutex::default(),
            wake,
            queued: AtomicBool::new(false),
        });
        (tap, woken)
    }

    fn buffer(&self) -> MutexGuard<'_, OutputBuffer> {
        self.buffer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn wake(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            self.wake.unbounded_send(()).ok();
        }
    }
}

impl RawTap for OutputTap {
    fn output(&self, bytes: &[u8]) {
        self.buffer().push(bytes);
        self.wake();
    }

    fn finished(&self, code: Option<i32>) {
        self.buffer().finish(code);
        self.wake();
    }
}

/// Sends a created terminal's output to its extension until the exit went out.
async fn pump(
    tap: Arc<OutputTap>,
    mut woken: UnboundedReceiver<()>,
    sink: TerminalSink,
    handle: TerminalHandle,
    executor: BackgroundExecutor,
) {
    loop {
        // Cleared before looking, so output queued after the look wakes us again.
        tap.queued.store(false, Ordering::Release);
        let (event, wait, done) = {
            let mut buffer = tap.buffer();
            let event = buffer.take(handle, executor.now());
            (event, buffer.wait(executor.now()), buffer.is_done())
        };
        if let Some(event) = event {
            sink(event);
        }
        if done {
            return;
        }
        match wait {
            Some(Duration::ZERO) => {}
            Some(wait) => executor.timer(wait).await,
            None => {
                if woken.next().await.is_none() {
                    // Every sender is gone; nothing more can arrive.
                    tap.buffer().finish(None);
                }
            }
        }
    }
}

/// What the main thread needs to start a created terminal.
pub(crate) struct CreatedOpen {
    pub id: TerminalId,
    pub parked: ParkedTerminal,
    pub tap: Arc<OutputTap>,
    pub woken: UnboundedReceiver<()>,
    pub sink: TerminalSink,
    pub handle: TerminalHandle,
    pub command: CreateCommand,
}

impl std::fmt::Debug for CreatedOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedOpen")
            .field("id", &self.id)
            .field("handle", &self.handle)
            .field("program", &self.command.program)
            .finish_non_exhaustive()
    }
}

/// Created terminals whose pty is still being set up: input queued for them, and whether the
/// extension closed them meanwhile.
#[derive(Default)]
struct Starting(HashMap<TerminalId, StartingTerminal>);

#[derive(Default)]
struct StartingTerminal {
    input: Vec<Vec<u8>>,
    cancelled: bool,
}

impl Global for Starting {}

/// Main thread: starts the program in a pty and shows it as a tab in the active window's terminal
/// panel (held without a tab when there is no window). A cwd outside every trusted worktree is
/// refused here too, whatever the import checked.
pub(crate) fn open(open: CreatedOpen, cx: &mut App) {
    let CreatedOpen {
        id,
        parked,
        tap,
        woken,
        sink,
        handle,
        command,
    } = open;
    cx.background_spawn(pump(
        tap.clone(),
        woken,
        sink,
        handle,
        cx.background_executor().clone(),
    ))
    .detach();
    if !in_trusted_worktree(&command.cwd, cx) {
        log::warn!("fyx: refused to start a created terminal outside a trusted worktree");
        refuse(id, &tap, cx);
        return;
    }
    cx.default_global::<Starting>()
        .0
        .insert(id, StartingTerminal::default());

    let window_id =
        pseudo::target_workspace(cx).map_or(0, |(window, _)| window.window_id().as_u64());
    let settings = TerminalSettings::get_global(cx);
    let mut env = settings.env.clone();
    env.extend(command.env);
    let (name, value) = parked.env();
    env.insert(name, value);
    let shell = task::Shell::WithArguments {
        program: command.program.to_string_lossy().into_owned(),
        args: command.args,
        title_override: None,
    };
    let started = unix_seconds(SystemTime::now());
    let builder = TerminalBuilder::new(
        Some(command.cwd),
        TerminalMode::interactive(),
        shell,
        env,
        settings.cursor_shape,
        settings.alternate_scroll,
        settings.max_scroll_history_lines,
        settings.path_hyperlink_regexes.clone(),
        Duration::from_millis(settings.path_hyperlink_timeout_ms),
        false,
        window_id,
        cx,
        Vec::new(),
        PathStyle::local(),
    );
    let title = command.title;
    cx.spawn(async move |cx| {
        let built = builder.await;
        // Taken by the builder unless it failed before; either way it is done with it.
        drop(parked);
        cx.update(|cx| started_terminal(id, built, title, &tap, started, cx));
    })
    .detach();
}

fn started_terminal<E: std::fmt::Debug>(
    id: TerminalId,
    built: Result<TerminalBuilder, E>,
    title: Option<String>,
    tap: &Arc<OutputTap>,
    started: u64,
    cx: &mut App,
) {
    let starting = cx
        .default_global::<Starting>()
        .0
        .remove(&id)
        .unwrap_or_default();
    let builder = match built {
        Ok(builder) => builder,
        Err(error) => {
            log::warn!("fyx: a created terminal did not start: {error:?}");
            refuse(id, tap, cx);
            return;
        }
    };
    let terminal = cx.new(|cx| builder.subscribe(cx));
    if let Some(pid) = terminal
        .read(cx)
        .pid_getter()
        .map(|getter| getter.fallback_pid())
    {
        cx.observe_release(&terminal, move |_, cx| {
            cx.background_spawn(async move { kill_tree(pid, started) })
                .detach();
        })
        .detach();
    }
    if terminal.read(cx).fyx.id() != id {
        log::error!("fyx: a created terminal's builder did not take its parked state");
        drop(terminal);
        refuse(id, tap, cx);
        return;
    }
    if starting.cancelled {
        drop(terminal);
        closed(id, cx);
        return;
    }
    crate::remember_terminal(id, terminal.downgrade(), cx);
    for bytes in starting.input {
        terminal.update(cx, |terminal, _| terminal.input(bytes));
    }
    let shown = pseudo::target_workspace(cx).is_some_and(|(window, workspace)| {
        window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    pseudo::add_tab(workspace, terminal.clone(), title, window, cx)
                })
            })
            .is_ok()
    });
    if !shown {
        pseudo::hold(id, terminal, cx);
    }
}

/// The program never runs (or no longer has a terminal): the extension gets `Exited(None)`.
fn refuse(id: TerminalId, tap: &OutputTap, cx: &mut App) {
    tap.finished(None);
    closed(id, cx);
}

fn closed(id: TerminalId, cx: &mut App) {
    if let Some(front) = crate::front(cx) {
        front.terminal_closed(id);
    }
}

/// Main thread: the extension's input. Before the pty is up it waits.
pub(crate) fn input(id: TerminalId, bytes: Vec<u8>, cx: &mut App) {
    if let Some(terminal) = crate::terminal_by_id(id, cx) {
        terminal.update(cx, |terminal, _| terminal.input(bytes));
    } else if let Some(starting) = cx.default_global::<Starting>().0.get_mut(&id) {
        starting.input.push(bytes);
    }
}

/// Main thread: closing a terminal that is still starting. Returns whether it was.
pub(crate) fn cancel(id: TerminalId, cx: &mut App) -> bool {
    match cx.default_global::<Starting>().0.get_mut(&id) {
        Some(starting) => {
            starting.cancelled = true;
            true
        }
        None => false,
    }
}

/// Whether `cwd` is inside a visible worktree of an open project that the trust store trusts.
/// Without a trust store (tests, a headless app) nothing is.
fn in_trusted_worktree(cwd: &Path, cx: &mut App) -> bool {
    let Some(trust) = TrustedWorktrees::try_get_global(cx) else {
        return false;
    };
    let projects: Vec<_> = cx
        .windows()
        .into_iter()
        .filter_map(|window| window.downcast::<MultiWorkspace>())
        .filter_map(|window| {
            window
                .read_with(cx, |multi_workspace, cx| {
                    multi_workspace
                        .workspaces()
                        .map(|workspace| workspace.read(cx).project().clone())
                        .collect::<Vec<_>>()
                })
                .ok()
        })
        .flatten()
        .collect();
    for project in projects {
        let (store, worktrees): (_, Vec<_>) = {
            let project = project.read(cx);
            (
                project.worktree_store(),
                project
                    .visible_worktrees(cx)
                    .filter(|worktree| contains(&worktree.read(cx).abs_path(), cwd))
                    .map(|worktree| worktree.read(cx).id())
                    .collect(),
            )
        };
        for worktree in worktrees {
            if trust.update(cx, |trust, cx| trust.can_trust(&store, worktree, cx)) {
                return true;
            }
        }
    }
    false
}

/// `path` is `root` or below it, comparing canonical paths when both exist (the import passes a
/// canonical cwd; a worktree's root may not be).
fn contains(root: &Path, path: &Path) -> bool {
    if path.starts_with(root) {
        return true;
    }
    match (std::fs::canonicalize(root), std::fs::canonicalize(path)) {
        (Ok(root), Ok(path)) => path.starts_with(root),
        _ => false,
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Kills `root` and every process below it that started after the terminal did (so a reused pid
/// is left alone). The parent ids are those of one snapshot: on Windows a child keeps its parent
/// id after the parent died; on Unix the pty's process group also gets the terminal's signals.
fn kill_tree(root: sysinfo::Pid, started: u64) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let parents: Vec<_> = system
        .processes()
        .iter()
        .filter(|(_, process)| process.start_time() + 1 >= started)
        .map(|(pid, process)| (*pid, process.parent()))
        .collect();
    for pid in descendants(root, &parents) {
        if let Some(process) = system.process(pid) {
            process.kill();
        }
    }
}

/// `root` (if it is among `parents`) and every process below it, parents before their children.
fn descendants(
    root: sysinfo::Pid,
    parents: &[(sysinfo::Pid, Option<sysinfo::Pid>)],
) -> Vec<sysinfo::Pid> {
    let mut tree = Vec::new();
    if parents.iter().any(|(pid, _)| *pid == root) {
        tree.push(root);
    }
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        let children: Vec<_> = parents
            .iter()
            .filter(|(pid, of)| *of == Some(parent) && *pid != root && !tree.contains(pid))
            .map(|(pid, _)| *pid)
            .collect();
        tree.extend(&children);
        frontier.extend(children);
    }
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANDLE: TerminalHandle = TerminalHandle(3);

    fn drain(buffer: &mut OutputBuffer, mut now: Instant) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        while let Some(wait) = buffer.wait(now) {
            now += wait;
            events.extend(buffer.take(HANDLE, now));
        }
        events
    }

    #[test]
    fn output_goes_out_at_most_30_times_a_second_then_the_exit() {
        let start = Instant::now();
        let mut buffer = OutputBuffer::default();
        assert_eq!(buffer.wait(start), None);
        buffer.push(b"hel");
        buffer.push(b"lo");
        assert_eq!(
            buffer.take(HANDLE, start),
            Some(TerminalEvent::Output(HANDLE, b"hello".to_vec(), 0))
        );
        buffer.push(b"!");
        let soon = start + Duration::from_millis(5);
        assert_eq!(buffer.take(HANDLE, soon), None);
        assert_eq!(
            buffer.wait(soon),
            Some(MIN_OUTPUT_INTERVAL - Duration::from_millis(5))
        );
        // The exit waits for the output before it, and nothing follows it.
        buffer.finish(Some(2));
        buffer.finish(None);
        buffer.push(b"late");
        assert_eq!(
            drain(&mut buffer, soon),
            [
                TerminalEvent::Output(HANDLE, b"!".to_vec(), 0),
                TerminalEvent::Exited(HANDLE, Some(2)),
            ]
        );
        assert!(buffer.is_done());
        assert_eq!(buffer.wait(soon + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_flood_goes_out_in_64_kib_chunks_and_the_oldest_output_is_dropped() {
        let mut buffer = OutputBuffer::default();
        let mut flood = vec![b'a'; MAX_OUTPUT_BUFFER_BYTES];
        flood.extend_from_slice(b"tail");
        buffer.push(&flood);
        buffer.finish(Some(0));
        let events = drain(&mut buffer, Instant::now());
        let mut total = 0;
        let mut dropped = 0;
        let mut last = Vec::new();
        for event in &events[..events.len() - 1] {
            let TerminalEvent::Output(_, bytes, gone) = event else {
                panic!("unexpected {event:?}");
            };
            assert!(bytes.len() <= MAX_OUTPUT_CHUNK_BYTES);
            total += bytes.len();
            dropped += gone;
            last = bytes.clone();
        }
        assert_eq!(total, MAX_OUTPUT_BUFFER_BYTES);
        assert_eq!(dropped, 4, "the first event says how much went");
        assert!(last.ends_with(b"tail"));
        assert_eq!(events.last(), Some(&TerminalEvent::Exited(HANDLE, Some(0))));
    }

    #[test]
    fn the_tree_is_the_root_and_everything_below_it() {
        let pid = sysinfo::Pid::from_u32;
        let parents = [
            (pid(1), None),
            (pid(10), Some(pid(1))),
            (pid(11), Some(pid(10))),
            (pid(12), Some(pid(11))),
            (pid(20), Some(pid(1))),
            (pid(13), Some(pid(10))),
        ];
        assert_eq!(
            descendants(pid(10), &parents),
            [pid(10), pid(11), pid(13), pid(12)]
        );
        // A root that already exited: its children still name it on Windows.
        assert_eq!(descendants(pid(30), &parents), []);
        assert_eq!(descendants(pid(11), &parents[2..]), [pid(11), pid(12)]);
    }

    /// Through the real `TerminalBuilder` and pty: the builder takes the parked state, so the tap
    /// sees the program's output and its exit code.
    #[gpui::test]
    async fn a_real_program_reaches_the_tap(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(|cx| {
            let store = settings::SettingsStore::test(cx);
            cx.set_global(store);
        });
        let (tap, _woken) = OutputTap::new();
        let ext = terminal_ext::TerminalExt::created("acme", tap.clone());
        let id = ext.id();
        let parked = ext.park();
        #[cfg(windows)]
        let (program, args) = (
            "cmd.exe".to_owned(),
            vec!["/C".to_owned(), "echo fyx-created& exit 3".to_owned()],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "printf fyx-created; exit 3".to_owned()],
        );
        let env = HashMap::from([parked.env()]);
        let builder = cx
            .update(|cx| {
                TerminalBuilder::new(
                    None,
                    TerminalMode::interactive(),
                    task::Shell::WithArguments {
                        program,
                        args,
                        title_override: None,
                    },
                    env.into_iter().collect(),
                    Default::default(),
                    terminal::terminal_settings::AlternateScroll::On,
                    None,
                    Vec::new(),
                    Duration::ZERO,
                    false,
                    0,
                    cx,
                    Vec::new(),
                    PathStyle::local(),
                )
            })
            .await
            .unwrap();
        drop(parked);
        let terminal = cx.new(|cx| builder.subscribe(cx));
        assert_eq!(terminal.read_with(cx, |terminal, _| terminal.fyx.id()), id);

        // The pty's IO thread is real: wait for it, not for the test scheduler.
        for _ in 0..400 {
            if tap.buffer().finished.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let events = drain(&mut tap.buffer(), Instant::now());
        let output: Vec<u8> = events
            .iter()
            .filter_map(|event| match event {
                TerminalEvent::Output(_, bytes, _) => Some(bytes.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(
            String::from_utf8_lossy(&output).contains("fyx-created"),
            "output: {:?}",
            String::from_utf8_lossy(&output)
        );
        assert_eq!(events.last(), Some(&TerminalEvent::Exited(HANDLE, Some(3))));
        drop(terminal);
    }

    /// Reload kills the tree: the program and what it started.
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test may block while it spawns"
    )]
    fn kill_tree_kills_the_program_and_its_children() {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

        let started = unix_seconds(SystemTime::now());
        #[cfg(windows)]
        let mut root = std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut root = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 60 & wait"])
            .spawn()
            .unwrap();
        let root_pid = sysinfo::Pid::from_u32(root.id());
        let mut system = System::new();
        let mut child = None;
        for _ in 0..200 {
            system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing(),
            );
            child = system
                .processes()
                .iter()
                .find(|(_, process)| process.parent() == Some(root_pid))
                .map(|(pid, _)| *pid);
            if child.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let child = child.expect("the program's child started");

        kill_tree(root_pid, started);
        root.wait().unwrap();
        let mut gone = false;
        for _ in 0..200 {
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[child]),
                true,
                ProcessRefreshKind::nothing(),
            );
            gone = system
                .process(child)
                .is_none_or(|process| process.status() == sysinfo::ProcessStatus::Zombie);
            if gone {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(gone, "the child outlived the kill");
    }

    /// The tap queues one wake-up for many reads; the pump sends everything, then the exit.
    #[gpui::test]
    async fn the_pump_delivers_tapped_output_and_the_exit(cx: &mut gpui::TestAppContext) {
        let (tap, woken) = OutputTap::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink: TerminalSink = Arc::new({
            let events = events.clone();
            move |event| events.lock().unwrap().push(event)
        });
        tap.output(b"first ");
        tap.output(b"byte");
        assert!(tap.queued.load(Ordering::Acquire));
        let executor = cx.executor();
        executor
            .spawn(pump(tap.clone(), woken, sink, HANDLE, executor.clone()))
            .detach();
        cx.run_until_parked();
        tap.output(b"!");
        tap.finished(Some(1));
        executor.advance_clock(MIN_OUTPUT_INTERVAL);
        cx.run_until_parked();
        assert_eq!(
            *events.lock().unwrap(),
            [
                TerminalEvent::Output(HANDLE, b"first byte".to_vec(), 0),
                TerminalEvent::Output(HANDLE, b"!".to_vec(), 0),
                TerminalEvent::Exited(HANDLE, Some(1)),
            ]
        );
    }
}
