//! Pseudo-terminals (`fyx:ui` §B.2): a display-only `Terminal` whose backend is an extension. The
//! host front ([`crate::host`]) checks, rate-limits and sanitises output on the caller's thread;
//! this module builds the terminal and its tab on the main thread and sends the user's input and
//! the size back, coalesced.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc::{TryRecvError, UnboundedReceiver, UnboundedSender};
use gpui::{
    AnyWindowHandle, App, AppContext as _, BackgroundExecutor, Entity, Global, TaskExt as _,
};
use settings::Settings as _;
use terminal::terminal_settings::TerminalSettings;
use terminal::{Terminal, TerminalBuilder};
use terminal_ext::{PseudoBackend, TerminalExt, TerminalId};
use terminal_view::TerminalView;
use terminal_view::terminal_panel::TerminalPanel;
use ui_ext::terminal::{
    MAX_INPUT_EVENT_BYTES, MAX_INPUT_EVENTS_PER_SECOND, MAX_OUTPUT_BUFFER_BYTES, TerminalEvent,
    TerminalHandle, TerminalSink,
};
use util::paths::PathStyle;
use workspace::dock::Panel as _;
use workspace::{MultiWorkspace, SaveIntent, Workspace};

/// What the terminal sends its backend.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PseudoIo {
    Input(Vec<u8>),
    Resized(u16, u16),
}

/// The upstream hooks call this on the main thread; it only queues.
struct Forwarder(UnboundedSender<PseudoIo>);

impl PseudoBackend for Forwarder {
    fn input(&self, bytes: &[u8]) {
        self.0.unbounded_send(PseudoIo::Input(bytes.to_vec())).ok();
    }

    fn resized(&self, cols: u16, rows: u16) {
        self.0.unbounded_send(PseudoIo::Resized(cols, rows)).ok();
    }
}

/// A pseudo-terminal's `TerminalExt` and the receiving end of its input.
pub(crate) fn new_terminal_ext(extension: &str) -> (TerminalExt, UnboundedReceiver<PseudoIo>) {
    let (input, io) = futures::channel::mpsc::unbounded();
    (
        TerminalExt::pseudo(extension, Arc::new(Forwarder(input))),
        io,
    )
}

/// Everything the main thread needs to show a pseudo-terminal.
pub(crate) struct PseudoOpen {
    pub ext: TerminalExt,
    pub io: UnboundedReceiver<PseudoIo>,
    pub sink: TerminalSink,
    pub handle: TerminalHandle,
    pub title: String,
    pub in_panel: bool,
}

impl std::fmt::Debug for PseudoOpen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PseudoOpen")
            .field("id", &self.ext.id())
            .field("handle", &self.handle)
            .field("in_panel", &self.in_panel)
            .finish_non_exhaustive()
    }
}

/// Extension terminals with no tab: pseudo-terminals shown by an extension panel's `terminal`
/// node, and created terminals when there was no window. The host keeps them alive until the
/// extension closes them.
#[derive(Default)]
struct HeldTerminals(HashMap<TerminalId, Entity<Terminal>>);

impl Global for HeldTerminals {}

pub(crate) fn is_held(id: TerminalId, cx: &App) -> bool {
    cx.try_global::<HeldTerminals>()
        .is_some_and(|held| held.0.contains_key(&id))
}

/// A pseudo-terminal the host keeps for an extension panel, for `ui_ext_host`'s `terminal` node.
pub(crate) fn held_terminal(id: TerminalId, cx: &App) -> Option<Entity<Terminal>> {
    cx.try_global::<HeldTerminals>()?.0.get(&id).cloned()
}

/// Main thread: builds the terminal and, unless it lives in a panel, its tab in the terminal
/// panel of the active window. With no window to show it in, the extension gets `Closed`.
pub(crate) fn open(open: PseudoOpen, cx: &mut App) {
    let PseudoOpen {
        ext,
        io,
        sink,
        handle,
        title,
        in_panel,
    } = open;
    let id = ext.id();
    let target = target_workspace(cx);
    let window_id = target
        .as_ref()
        .map_or(0, |(window, _)| window.window_id().as_u64());
    let settings = TerminalSettings::get_global(cx);
    let (cursor_shape, alternate_scroll, history) = (
        settings.cursor_shape,
        settings.alternate_scroll,
        settings.max_scroll_history_lines,
    );
    let terminal = cx.new(|cx| {
        let mut terminal = TerminalBuilder::new_display_only(
            cursor_shape,
            alternate_scroll,
            history,
            window_id,
            cx.background_executor(),
            PathStyle::local(),
        )
        .subscribe(cx);
        terminal.fyx = ext;
        terminal
    });
    crate::remember_terminal(id, terminal.downgrade(), cx);
    let executor = cx.background_executor().clone();
    cx.background_spawn(forward_input(io, sink, handle, executor))
        .detach();

    if in_panel {
        hold(id, terminal, cx);
        return;
    }
    let shown = target.is_some_and(|(window, workspace)| {
        window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    add_tab(workspace, terminal.clone(), Some(title), window, cx)
                })
            })
            .is_ok()
    });
    if !shown {
        log::warn!("fyx: no window to show a pseudo-terminal in");
        drop(terminal);
        if let Some(front) = crate::front(cx) {
            front.terminal_closed(id);
        }
    }
}

/// Keeps an extension's terminal that has no tab until the extension closes it.
pub(crate) fn hold(id: TerminalId, terminal: Entity<Terminal>, cx: &mut App) {
    cx.default_global::<HeldTerminals>().0.insert(id, terminal);
}

pub(crate) fn target_workspace(cx: &App) -> Option<(AnyWindowHandle, Entity<Workspace>)> {
    let windows = cx.active_window().into_iter().chain(cx.windows());
    windows.into_iter().find_map(|window| {
        let multi_workspace = window.downcast::<MultiWorkspace>()?;
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
            .ok()?;
        Some((window, workspace))
    })
}

/// Opens the tab without focus: the extension, not the user, asked for it.
pub(crate) fn add_tab(
    workspace: &mut Workspace,
    terminal: Entity<Terminal>,
    title: Option<String>,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) {
    let view = cx.new(|cx| {
        let mut view = TerminalView::new(
            terminal,
            workspace.weak_handle(),
            workspace.database_id(),
            workspace.project().downgrade(),
            window,
            cx,
        );
        if title.is_some() {
            view.set_custom_title(title, cx);
        }
        view
    });
    let pane = workspace
        .panel::<TerminalPanel>(cx)
        .and_then(|panel| panel.read(cx).pane());
    match pane {
        Some(pane) => {
            workspace.open_panel::<TerminalPanel>(window, cx);
            pane.update(cx, |pane, cx| {
                pane.add_item(Box::new(view), true, false, None, window, cx)
            });
        }
        None => workspace.add_item_to_active_pane(Box::new(view), None, false, window, cx),
    }
}

/// Main thread: the extension closed the terminal, or its extension went away.
pub(crate) fn close(id: TerminalId, cx: &mut App) {
    let held = cx
        .try_global::<HeldTerminals>()
        .is_some_and(|held| held.0.contains_key(&id));
    if held {
        cx.default_global::<HeldTerminals>().0.remove(&id);
        if let Some(front) = crate::front(cx) {
            front.terminal_closed(id);
        }
        return;
    }
    let Some((view, window)) =
        crate::sessions::global(cx).and_then(|sessions| sessions.read(cx).find_terminal(id, cx))
    else {
        return;
    };
    window
        .update(cx, |_, window, cx| {
            let Some(Some(multi_workspace)) = window.root::<MultiWorkspace>() else {
                return;
            };
            let panes: Vec<_> = multi_workspace
                .read(cx)
                .workspaces()
                .filter_map(|workspace| crate::sessions::pane_of(workspace.read(cx), &view, cx))
                .collect();
            for pane in panes {
                pane.update(cx, |pane, cx| {
                    pane.close_item_by_id(view.entity_id(), SaveIntent::Skip, window, cx)
                        .detach_and_log_err(cx)
                });
            }
        })
        .ok();
}

/// Input to a pseudo-terminal's backend: at most [`MAX_INPUT_EVENTS_PER_SECOND`] events of at
/// most [`MAX_INPUT_EVENT_BYTES`] each, and the last size only.
#[derive(Debug, Default)]
pub(crate) struct InputCoalescer {
    pending: Vec<u8>,
    size: Option<(u16, u16)>,
    sent_size: Option<(u16, u16)>,
    last_sent: Option<Instant>,
    dropped: u64,
}

pub(crate) const MIN_INPUT_INTERVAL: Duration =
    Duration::from_nanos(1_000_000_000 / MAX_INPUT_EVENTS_PER_SECOND as u64);

impl InputCoalescer {
    pub(crate) fn push(&mut self, io: PseudoIo) {
        match io {
            PseudoIo::Input(bytes) => {
                let room = MAX_OUTPUT_BUFFER_BYTES.saturating_sub(self.pending.len());
                let taken = bytes.len().min(room);
                self.pending.extend_from_slice(&bytes[..taken]);
                self.dropped += (bytes.len() - taken) as u64;
            }
            PseudoIo::Resized(cols, rows) => self.size = Some((cols, rows)),
        }
    }

    fn has_pending(&self) -> bool {
        !self.pending.is_empty() || self.size != self.sent_size
    }

    /// How long to wait before [`Self::take`] may send, if there is anything to send.
    pub(crate) fn wait(&self, now: Instant) -> Option<Duration> {
        if !self.has_pending() {
            return None;
        }
        let since = self.last_sent.map_or(MIN_INPUT_INTERVAL, |last| {
            now.saturating_duration_since(last)
        });
        Some(MIN_INPUT_INTERVAL.saturating_sub(since))
    }

    /// The events due at `now`: the new size, then one chunk of input.
    pub(crate) fn take(&mut self, handle: TerminalHandle, now: Instant) -> Vec<TerminalEvent> {
        if self.wait(now) != Some(Duration::ZERO) {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some((cols, rows)) = self.size
            && self.sent_size != self.size
        {
            self.sent_size = self.size;
            events.push(TerminalEvent::Resized(handle, cols, rows));
        }
        if !self.pending.is_empty() {
            let end = self.pending.len().min(MAX_INPUT_EVENT_BYTES);
            let chunk: Vec<u8> = self.pending.drain(..end).collect();
            events.push(TerminalEvent::Input(handle, chunk));
        }
        if self.dropped > 0 {
            log::warn!(
                "fyx: dropped {} bytes of pseudo-terminal input its extension did not keep up with",
                self.dropped
            );
            self.dropped = 0;
        }
        self.last_sent = Some(now);
        events
    }
}

async fn forward_input(
    mut io: UnboundedReceiver<PseudoIo>,
    sink: TerminalSink,
    handle: TerminalHandle,
    executor: BackgroundExecutor,
) {
    let mut coalescer = InputCoalescer::default();
    loop {
        if !coalescer.has_pending() {
            match io.next().await {
                Some(item) => coalescer.push(item),
                None => return,
            }
        }
        loop {
            match io.try_recv() {
                Ok(item) => coalescer.push(item),
                // The terminal is gone.
                Err(TryRecvError::Closed) => return,
                Err(TryRecvError::Empty) => break,
            }
        }
        match coalescer.wait(executor.now()) {
            Some(Duration::ZERO) => {
                for event in coalescer.take(handle, executor.now()) {
                    sink(event);
                }
            }
            Some(wait) => executor.timer(wait).await,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use terminal::terminal_settings::{AlternateScroll, CursorShape};

    use super::*;

    /// The upstream hooks: typing reaches the backend, the terminal's own answers do not.
    #[gpui::test]
    async fn a_pseudo_terminal_forwards_input_but_never_answers(cx: &mut TestAppContext) {
        let (ext, mut io) = new_terminal_ext("acme");
        let terminal = cx.new(|cx| {
            let mut terminal = TerminalBuilder::new_display_only(
                CursorShape::default(),
                AlternateScroll::On,
                None,
                0,
                cx.background_executor(),
                PathStyle::local(),
            )
            .subscribe(cx);
            terminal.fyx = ext;
            terminal
        });
        terminal.update(cx, |terminal, cx| {
            // Unsanitised on purpose: DA, CPR and an OSC 52 read.
            terminal.write_raw_output(b"\x1b[c\x1b[6n\x1b]52;c;?\x07", cx);
            terminal.input(b"ls\r".to_vec());
        });
        cx.run_until_parked();
        let mut received = Vec::new();
        while let Ok(item) = io.try_recv() {
            received.push(item);
        }
        assert_eq!(received, [PseudoIo::Input(b"ls\r".to_vec())]);
    }

    const HANDLE: TerminalHandle = TerminalHandle(7);

    #[test]
    fn input_is_coalesced_and_only_the_last_size_is_sent() {
        let start = Instant::now();
        let mut coalescer = InputCoalescer::default();
        assert_eq!(coalescer.wait(start), None);

        coalescer.push(PseudoIo::Resized(80, 24));
        coalescer.push(PseudoIo::Input(b"l".to_vec()));
        coalescer.push(PseudoIo::Resized(100, 30));
        coalescer.push(PseudoIo::Input(b"s\r".to_vec()));
        assert_eq!(coalescer.wait(start), Some(Duration::ZERO));
        assert_eq!(
            coalescer.take(HANDLE, start),
            [
                TerminalEvent::Resized(HANDLE, 100, 30),
                TerminalEvent::Input(HANDLE, b"ls\r".to_vec())
            ]
        );

        // Not again within 1/30 s.
        coalescer.push(PseudoIo::Input(b"x".to_vec()));
        let soon = start + Duration::from_millis(10);
        assert!(coalescer.take(HANDLE, soon).is_empty());
        assert_eq!(
            coalescer.wait(soon),
            Some(MIN_INPUT_INTERVAL - Duration::from_millis(10))
        );
        // The same size again is not a resize.
        coalescer.push(PseudoIo::Resized(100, 30));
        let later = start + MIN_INPUT_INTERVAL;
        assert_eq!(
            coalescer.take(HANDLE, later),
            [TerminalEvent::Input(HANDLE, b"x".to_vec())]
        );
        assert_eq!(coalescer.wait(later), None);
    }

    #[test]
    fn large_input_goes_out_in_64_kib_events_and_the_buffer_is_capped() {
        let mut now = Instant::now();
        let mut coalescer = InputCoalescer::default();
        coalescer.push(PseudoIo::Input(vec![b'a'; MAX_OUTPUT_BUFFER_BYTES + 10]));
        let mut total = 0;
        while let Some(wait) = coalescer.wait(now) {
            now += wait;
            for event in coalescer.take(HANDLE, now) {
                let TerminalEvent::Input(_, bytes) = event else {
                    panic!("unexpected {event:?}");
                };
                assert!(bytes.len() <= MAX_INPUT_EVENT_BYTES);
                total += bytes.len();
            }
        }
        assert_eq!(total, MAX_OUTPUT_BUFFER_BYTES);
    }
}
