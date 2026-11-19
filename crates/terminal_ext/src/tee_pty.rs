//! [`TeePty`]: an `EventedPty` that delegates to the real pty and taps its output for OSC payloads.

use std::borrow::Cow;
use std::io::{self, Read};
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoopSender, Msg};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};
use polling::{Event, PollMode, Poller};

use crate::osc_scan::OscScanner;
use crate::program_status;

/// Called on the IO thread with every OSC payload the pty writes. Keep it cheap.
pub type OscObserver = Box<dyn FnMut(&[u8]) + Send>;

type ReplyFn = Box<dyn Fn(Cow<'static, [u8]>) + Send + Sync>;

/// Wraps `pty` so its output is scanned for OSC sequences; OSC payloads are logged at debug level.
pub fn tee<P: EventedPty>(pty: P) -> (TeePty<P>, OscReplies) {
    tee_with(
        pty,
        Box::new(|payload| {
            log::debug!("fyx terminal OSC: {:?}", String::from_utf8_lossy(payload));
        }),
    )
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
}

impl<P> TeePty<P> {
    pub fn inner(&self) -> &P {
        &self.inner
    }
}

impl<P: EventedPty> Read for TeePty<P> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.reader().read(buf)?;
        let Self {
            scanner,
            observer,
            replies,
            ..
        } = self;
        scanner.feed(&buf[..read], |payload| {
            if program_status::is_query(payload) {
                match replies.get() {
                    Some(reply) => reply(Cow::Borrowed(program_status::REPLY)),
                    None => log::warn!("fyx terminal: OSC 7501 query before the tap was connected"),
                }
            }
            observer(payload);
        });
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
        self.inner.next_child_event()
    }
}

impl<P: OnResize> OnResize for TeePty<P> {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size)
    }
}
