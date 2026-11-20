//! [`TeePty`]: an `EventedPty` that delegates to the real pty and taps its output for OSC payloads.

use std::borrow::Cow;
use std::io::{self, Read};
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoopSender, Msg};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};
use polling::{Event, PollMode, Poller};

use crate::inbox::{NOTIFY_RATE, QUERY_REPLY_RATE, RateLimiter, SHELL_RATE, STATE_RATE};
use crate::osc::{self, OscEvent};
use crate::osc_scan::OscScanner;
use crate::program_status;
use crate::terminal_id::{Shared, Tap, TerminalExt, lock};

/// Called on the IO thread with every OSC payload the pty writes. Keep it cheap.
pub type OscObserver = Box<dyn FnMut(&[u8]) + Send>;

type ReplyFn = Box<dyn Fn(Cow<'static, [u8]>) + Send + Sync>;

/// Wraps the pty of terminal `ext` so its output is scanned for OSC sequences.
pub fn tee<P: EventedPty>(pty: P, ext: &TerminalExt) -> (TeePty<P>, OscReplies) {
    let id = ext.id();
    let (mut pty, replies) = tee_with(
        pty,
        Box::new(move |payload| {
            log::debug!(
                "fyx terminal {id} OSC: {:?}",
                String::from_utf8_lossy(payload)
            );
        }),
    );
    pty.shared = Some(ext.shared.clone());
    (pty, replies)
}

/// Like [`tee`], with a custom observer for OSC payloads.
pub fn tee_with<P: EventedPty>(pty: P, observer: OscObserver) -> (TeePty<P>, OscReplies) {
    let replies = Arc::new(OnceLock::new());
    (
        TeePty {
            inner: pty,
            scanner: OscScanner::default(),
            observer,
            replies: replies.clone(),
            shared: None,
            limits: Limits::default(),
            exit_code: None,
        },
        OscReplies { replies },
    )
}

/// Where the tap sends its replies; connect it to the event loop's channel before the loop starts.
pub struct OscReplies {
    replies: Arc<OnceLock<ReplyFn>>,
}

impl OscReplies {
    /// Replies go through the event loop's own write queue, so they never interleave with input.
    pub fn connect(self, sender: EventLoopSender) {
        self.connect_with(move |bytes| {
            sender.send(Msg::Input(bytes)).ok();
        });
    }

    pub fn connect_with(self, reply: impl Fn(Cow<'static, [u8]>) + Send + Sync + 'static) {
        self.replies.set(Box::new(reply)).ok();
    }
}

pub struct TeePty<P> {
    inner: P,
    scanner: OscScanner,
    observer: OscObserver,
    replies: Arc<OnceLock<ReplyFn>>,
    shared: Option<Arc<Shared>>,
    limits: Limits,
    /// The program's exit code, for a created terminal's tap when the pty closes.
    exit_code: Option<i32>,
}

/// Rate limits, owned by the IO thread.
#[derive(Debug)]
struct Limits {
    notify: RateLimiter,
    shell: RateLimiter,
    state: RateLimiter,
    query_reply: RateLimiter,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            notify: RateLimiter::new(NOTIFY_RATE),
            shell: RateLimiter::new(SHELL_RATE),
            state: RateLimiter::new(STATE_RATE),
            query_reply: RateLimiter::new(QUERY_REPLY_RATE),
        }
    }
}

impl Limits {
    fn allow(&mut self, event: &OscEvent) -> bool {
        match event {
            OscEvent::Notify { .. } => self.notify.allow(),
            OscEvent::Shell { .. } | OscEvent::CommandLine { .. } => self.shell.allow(),
            OscEvent::Cwd { .. }
            | OscEvent::Progress(_)
            | OscEvent::UserVar { .. }
            | OscEvent::ProgramStatus(_) => self.state.allow(),
        }
    }
}

impl<P> TeePty<P> {
    pub fn inner(&self) -> &P {
        &self.inner
    }
}

impl<P: EventedPty> Read for TeePty<P> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.reader().read(buf)?;
        // A created terminal's extension reads the raw output, from the first byte (§B.3).
        if let Some(Tap(tap)) = self
            .shared
            .as_ref()
            .and_then(|shared| shared.raw_tap.as_ref())
        {
            tap.output(&buf[..read]);
        }
        let Self {
            scanner,
            observer,
            replies,
            shared,
            limits,
            ..
        } = self;
        let bytes = &buf[..read];
        // Output up to here goes to the running command block, if any (plan 5.17).
        let mut block_start = 0;
        scanner.feed_at(bytes, |payload, end| {
            observer(payload);
            if program_status::is_query(payload) {
                if !limits.query_reply.allow() {
                    return;
                }
                match replies.get() {
                    Some(reply) => reply(Cow::Borrowed(program_status::REPLY)),
                    None => log::warn!("fyx terminal: OSC 7501 query before the tap was connected"),
                }
                return;
            }
            let Some(shared) = shared else {
                return;
            };
            let Some(event) = osc::parse(payload) else {
                return;
            };
            if matches!(event, OscEvent::Shell { .. } | OscEvent::CommandLine { .. }) {
                let mut blocks = lock(&shared.blocks);
                if blocks.recording() {
                    blocks.output(&bytes[block_start..end]);
                }
                block_start = crate::blocks::after_terminator(bytes, end);
                blocks.mark(&event, &shared.shell_nonce);
            }
            if !limits.allow(&event) {
                lock(&shared.inbox).note_dropped();
                return;
            }
            lock(&shared.status).apply(&event, std::time::Instant::now());
            lock(&shared.inbox).push(event);
        });
        if let Some(shared) = shared {
            let mut blocks = lock(&shared.blocks);
            if blocks.recording() {
                blocks.output(&bytes[block_start..]);
            }
        }
        Ok(read)
    }
}

impl<P: EventedPty> EventedReadWrite for TeePty<P> {
    type Reader = Self;
    type Writer = P::Writer;

    unsafe fn register(
        &mut self,
        poller: &Arc<Poller>,
        event: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        // SAFETY: the inner pty's sources live as long as `self`, which the caller keeps registered.
        unsafe { self.inner.register(poller, event, mode) }
    }

    fn reregister(&mut self, poller: &Arc<Poller>, event: Event, mode: PollMode) -> io::Result<()> {
        self.inner.reregister(poller, event, mode)
    }

    fn deregister(&mut self, poller: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poller)
    }

    fn reader(&mut self) -> &mut Self {
        self
    }

    fn writer(&mut self) -> &mut P::Writer {
        self.inner.writer()
    }
}

impl<P: EventedPty> EventedPty for TeePty<P> {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        let event = self.inner.next_child_event();
        if let (Some(ChildEvent::Exited(status)), Some(shared)) = (&event, &self.shared) {
            self.exit_code = status.and_then(|status| status.code());
            lock(&shared.status).drop_live_records();
        }
        event
    }
}

impl<P> Drop for TeePty<P> {
    fn drop(&mut self) {
        // The event loop drains the output after the exit, then drops the pty: nothing follows.
        if let Some(Tap(tap)) = self
            .shared
            .as_ref()
            .and_then(|shared| shared.raw_tap.as_ref())
        {
            tap.finished(self.exit_code);
        }
    }
}

impl<P: OnResize> OnResize for TeePty<P> {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Mutex;

    use super::*;
    use crate::osc::{Progress, Protocol};

    /// A pty whose output is a fixed byte string.
    struct FakePty {
        output: Cursor<Vec<u8>>,
        input: Vec<u8>,
    }

    impl EventedReadWrite for FakePty {
        type Reader = Cursor<Vec<u8>>;
        type Writer = Vec<u8>;

        unsafe fn register(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
            Ok(())
        }

        fn reregister(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
            Ok(())
        }

        fn deregister(&mut self, _: &Arc<Poller>) -> io::Result<()> {
            Ok(())
        }

        fn reader(&mut self) -> &mut Self::Reader {
            &mut self.output
        }

        fn writer(&mut self) -> &mut Self::Writer {
            &mut self.input
        }
    }

    impl EventedPty for FakePty {
        fn next_child_event(&mut self) -> Option<ChildEvent> {
            None
        }
    }

    /// A [`FakePty`] whose program has exited with code 3.
    struct ExitedPty(FakePty);

    impl EventedReadWrite for ExitedPty {
        type Reader = Cursor<Vec<u8>>;
        type Writer = Vec<u8>;

        unsafe fn register(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
            Ok(())
        }

        fn reregister(&mut self, _: &Arc<Poller>, _: Event, _: PollMode) -> io::Result<()> {
            Ok(())
        }

        fn deregister(&mut self, _: &Arc<Poller>) -> io::Result<()> {
            Ok(())
        }

        fn reader(&mut self) -> &mut Self::Reader {
            self.0.reader()
        }

        fn writer(&mut self) -> &mut Self::Writer {
            self.0.writer()
        }
    }

    impl EventedPty for ExitedPty {
        fn next_child_event(&mut self) -> Option<ChildEvent> {
            #[cfg(unix)]
            let status = std::os::unix::process::ExitStatusExt::from_raw(3 << 8);
            #[cfg(windows)]
            let status = std::os::windows::process::ExitStatusExt::from_raw(3);
            Some(ChildEvent::Exited(Some(status)))
        }
    }

    #[derive(Default)]
    struct Recorder {
        output: Mutex<Vec<u8>>,
        finished: Mutex<Vec<Option<i32>>>,
    }

    impl crate::RawTap for Recorder {
        fn output(&self, bytes: &[u8]) {
            self.output.lock().unwrap().extend_from_slice(bytes);
        }

        fn finished(&self, code: Option<i32>) {
            self.finished.lock().unwrap().push(code);
        }
    }

    /// §B.3: the extension sees every byte, OSC included, and the exit code when the pty closes.
    #[test]
    fn a_created_terminals_tap_sees_raw_output_and_the_exit_code() {
        let output = b"\x1b]52;c;aGk=\x07hi\x1b]9;4;1;30\x07";
        let tap = Arc::new(Recorder::default());
        let ext = TerminalExt::created("acme", tap.clone());
        let (mut pty, _replies) = tee(
            ExitedPty(FakePty {
                output: Cursor::new(output.to_vec()),
                input: Vec::new(),
            }),
            &ext,
        );
        let mut read = Vec::new();
        pty.read_to_end(&mut read).unwrap();
        assert_eq!(read, output);
        assert_eq!(*tap.output.lock().unwrap(), output);
        assert!(matches!(
            pty.next_child_event(),
            Some(ChildEvent::Exited(Some(_)))
        ));
        assert!(tap.finished.lock().unwrap().is_empty());
        drop(pty);
        assert_eq!(*tap.finished.lock().unwrap(), [Some(3)]);
    }

    #[test]
    fn output_passes_through_and_osc_events_reach_the_terminal() {
        let output = b"hi\x1b]9;4;1;30\x07\x1b]9;4;1;60\x07\x1b]7501;?\x07\x1b]7501;?\x07\x1b]7501;?\x07\x1b]7501;?\x07\x1b]7501;?\x07\x1b]9;done\x1b\\!";
        let ext = TerminalExt::new();
        let (mut pty, replies) = tee(
            FakePty {
                output: Cursor::new(output.to_vec()),
                input: Vec::new(),
            },
            &ext,
        );
        let sent = Arc::new(Mutex::new(Vec::new()));
        replies.connect_with({
            let sent = sent.clone();
            move |bytes| sent.lock().unwrap().push(bytes.into_owned())
        });

        let mut read = Vec::new();
        pty.read_to_end(&mut read).unwrap();
        assert_eq!(read, output, "the tap must not change the output");

        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 4, "query replies are rate limited to the burst");
        assert!(sent.iter().all(|reply| reply == program_status::REPLY));

        let (events, dropped) = ext.take_osc_events();
        assert_eq!(
            events,
            [
                OscEvent::Progress(Progress::Normal(60)),
                OscEvent::Notify {
                    source: Protocol::Osc9,
                    title: None,
                    body: "done".into()
                }
            ]
        );
        assert_eq!(dropped, 0);
        assert_eq!(ext.take_osc_events(), (Vec::new(), 0));
    }

    #[test]
    fn the_last_command_block_is_recorded_with_its_nonce() {
        let _reader = crate::CommandBlockReader::acquire();
        let ext = TerminalExt::new();
        let run = |output: Vec<u8>| {
            let (mut pty, _) = tee(
                FakePty {
                    output: Cursor::new(output.clone()),
                    input: Vec::new(),
                },
                &ext,
            );
            let mut read = Vec::new();
            pty.read_to_end(&mut read).unwrap();
            assert_eq!(read, output, "the tap must not change the output");
            ext.last_command_block().unwrap()
        };
        let block = run(format!(
            "$ \x1b]633;E;cargo test;{}\x07\x1b]633;C\x1b\\\r\n\x1b[32mok\x1b[0m\r\n\x1b]633;D;0\x07\x1b]633;A\x07$ ",
            ext.shell_nonce()
        )
        .into_bytes());
        assert_eq!(
            (
                block.command_line.as_deref(),
                block.trusted,
                block.exit_code
            ),
            (Some("cargo test"), true, Some(0))
        );
        assert_eq!(block.output, "ok");
        // A program can print marks too; without the nonce its command line is not trusted.
        let forged =
            run(b"\x1b]633;E;sudo rm -rf /;guess\x07\x1b]133;C\x07fake\x1b]133;D;0\x07".to_vec());
        assert_eq!(
            (
                forged.command_line.as_deref(),
                forged.trusted,
                forged.output.as_str()
            ),
            (Some("sudo rm -rf /"), false, "fake")
        );
        assert_eq!(ext.shell_nonce().len(), 64);
    }
}
