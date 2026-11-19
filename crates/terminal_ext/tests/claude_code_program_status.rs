//! Runs the real Claude Code in a real pty behind [`TeePty`] and alacritty's own `EventLoop`, the way
//! `crates/terminal` does, and checks the OSC 7501 handshake: Claude Code asks `OSC 7501 ; ?`, the tap
//! answers, and Claude Code then reports its state with `OSC 7501 ; state=...`.
//!
//! Needs Claude Code installed and signed in, so it is ignored by default:
//!
//! ```sh
//! FYX_CLAUDE_EXE=/path/to/claude cargo test -p terminal_ext --test claude_code_program_status -- --ignored --nocapture
//! ```
//!
//! `FYX_CLAUDE_ARGS` passes extra arguments (e.g. `--debug-file out.txt`). `FYX_CLAUDE_PROMPT=...` also types a prompt and waits for `working` and then `idle`/`done`. The test
//! drops every `CLAUDE*` variable from the child's environment, so it also works from inside a Claude
//! Code session (which sets `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`, turning OSC 7501 off).

use std::borrow::Cow;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, Options, Shell};

/// Writes the terminal's own replies (DA1, XTVERSION, ...) back to the pty, as Zed's listener does.
#[derive(Clone)]
struct Listener(Arc<OnceLock<EventLoopSender>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event
            && let Some(sender) = self.0.get()
        {
            sender.send(Msg::Input(text.into_bytes().into())).ok();
        }
    }
}

/// Waits until a payload at index `since` or later matches `found`.
fn wait_for(
    seen: &Mutex<Vec<String>>,
    since: usize,
    timeout: Duration,
    found: impl Fn(&str) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if seen.lock().unwrap()[since..]
            .iter()
            .any(|payload| found(payload))
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[ignore = "needs Claude Code installed and signed in"]
fn claude_code_reports_program_status_through_the_tap() {
    let program = std::env::var("FYX_CLAUDE_EXE").unwrap_or_else(|_| "claude".into());
    let args = std::env::var("FYX_CLAUDE_ARGS")
        .map(|args| args.split_whitespace().map(String::from).collect())
        .unwrap_or_default();
    let claude_env: Vec<String> = std::env::vars()
        .map(|(key, _)| key)
        .filter(|key| key.starts_with("CLAUDE"))
        .collect();
    for key in claude_env {
        // SAFETY: this test binary has a single test, and no other thread reads the environment yet.
        unsafe { std::env::remove_var(key) };
    }

    let options = Options {
        shell: Some(Shell::new(program, args)),
        working_directory: Some(std::env::current_dir().unwrap()),
        ..Default::default()
    };
    let window_size = WindowSize {
        num_lines: 40,
        num_cols: 120,
        cell_width: 8,
        cell_height: 16,
    };
    let pty = tty::new(&options, window_size, 0).expect("spawn Claude Code in a pty");

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let (pty, replies) = terminal_ext::tee_with(pty, {
        let seen = seen.clone();
        Box::new(move |payload| {
            let payload = String::from_utf8_lossy(payload).into_owned();
            eprintln!("OSC {payload:?}");
            seen.lock().unwrap().push(payload);
        })
    });
    let replies_sent = Arc::new(Mutex::new(0usize));

    let sender_slot = Arc::new(OnceLock::new());
    let listener = Listener(sender_slot.clone());
    let term = Term::new(Config::default(), &TermSize::new(120, 40), listener.clone());
    let term = Arc::new(FairMutex::new(term));
    let event_loop = EventLoop::new(term.clone(), listener, pty, false, false).expect("event loop");
    let sender = event_loop.channel();
    sender_slot.set(sender.clone()).ok();
    replies.connect_with({
        let sender = sender.clone();
        let replies_sent = replies_sent.clone();
        move |bytes: Cow<'static, [u8]>| {
            *replies_sent.lock().unwrap() += 1;
            sender.send(Msg::Input(bytes)).ok();
        }
    });
    let io_thread = event_loop.spawn();

    let asked = wait_for(&seen, 0, Duration::from_secs(30), |p| p == "7501;?");
    let reported = asked
        && wait_for(&seen, 0, Duration::from_secs(40), |p| {
            p.starts_with("7501;state=")
        });

    let mut worked = None;
    if reported && let Ok(prompt) = std::env::var("FYX_CLAUDE_PROMPT") {
        std::thread::sleep(Duration::from_secs(2));
        sender
            .send(Msg::Input(Cow::Owned(prompt.into_bytes())))
            .ok();
        std::thread::sleep(Duration::from_millis(500));
        sender.send(Msg::Input(Cow::Borrowed(b"\r"))).ok();
        let before_prompt = seen.lock().unwrap().len();
        let working = wait_for(&seen, before_prompt, Duration::from_secs(60), |p| {
            p.starts_with("7501;state=working")
        });
        let settled = working
            && wait_for(&seen, before_prompt, Duration::from_secs(60), |p| {
                p.starts_with("7501;state=idle") || p.starts_with("7501;state=done")
            });
        worked = Some((working, settled));
    }

    if std::env::var_os("FYX_DUMP_SCREEN").is_some() {
        let term = term.lock();
        let grid = term.grid();
        for line in 0..grid.screen_lines() {
            let row = &grid[alacritty_terminal::index::Line(line as i32)];
            let text: String = (0..grid.columns())
                .map(|column| row[alacritty_terminal::index::Column(column)].c)
                .collect();
            eprintln!("|{}", text.trim_end());
        }
    }
    sender.send(Msg::Shutdown).ok();
    io_thread.join().ok();

    let reports: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.starts_with("7501;"))
        .cloned()
        .collect();
    eprintln!("OSC 7501 payloads: {reports:#?}");
    assert!(asked, "Claude Code never sent the OSC 7501 support query");
    assert_eq!(
        *replies_sent.lock().unwrap(),
        1,
        "the tap should answer the query once"
    );
    assert!(
        reported,
        "Claude Code did not report state after the tap answered"
    );
    if let Some((working, settled)) = worked {
        assert!(working, "no state=working after the prompt");
        assert!(settled, "no state=idle/done after the prompt");
    }
}
