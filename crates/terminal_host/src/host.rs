//! `ui_ext::terminal::TerminalHost` for `fyx:ui@0.4` extensions (design `docs/fyx/wit-0.4/design.md`
//! §B). Extensions call it from the runtime's threads, so this is a `Send + Sync` front: it checks,
//! allocates handles and rate-limits on the calling thread, and queues work that needs GPUI to the
//! main thread ([`Command`]). The main thread keeps it told about every terminal ([`HostFront::
//! terminal_opened`] and friends), so `observe` answers without a round trip.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use futures::channel::mpsc::UnboundedSender;
use terminal_ext::inbox::{Rate, RateLimiter};
use terminal_ext::{ExtensionStatus, OutputSanitizer, StatusHandle, StatusSource, TerminalId, osc};
use ui_ext::terminal::{
    CreateCommand, MAX_CREATED_TERMINALS, MAX_PASTE_BYTES, MAX_PSEUDO_OUTPUT_BYTES_PER_SECOND,
    MAX_READ_LINES, MAX_SEND_INPUT_BYTES, MAX_STATUS_UPDATES_PER_SECOND, MAX_WRITE_OUTPUT_BYTES,
    PseudoRequest, ReadScope, StatusState, T3Target, TerminalError, TerminalEvent, TerminalHandle,
    TerminalHost, TerminalInfo, TerminalOrigin, TerminalSink, TerminalStatus,
};

use crate::created::{CreatedOpen, OutputTap};
use crate::pseudo::{self, PseudoOpen};
use crate::t3::T3Reply;

/// The title another extension sees for a terminal that is not its own (W2c, 282e99d69a).
pub const REDACTED: &str = "•••";

/// Work for the main thread.
#[derive(Debug)]
pub(crate) enum Command {
    /// A status changed off the main thread: send the terminal a `Wakeup` so its tab repaints.
    Repaint(TerminalId),
    /// Build a pseudo-terminal and its tab.
    OpenPseudo(PseudoOpen),
    /// Sanitised output for a pseudo-terminal, at most [`MAX_WRITE_OUTPUT_BYTES`].
    Output(TerminalId, Vec<u8>),
    /// Start a created terminal's program and show its tab.
    OpenCreated(CreatedOpen),
    /// An extension's input for its created terminal, at most [`MAX_SEND_INPUT_BYTES`].
    Input(TerminalId, Vec<u8>),
    /// A bracketed paste ([`paste_bytes`]); dropped if the program turned bracketed paste off.
    Paste(TerminalId, Vec<u8>),
    /// Close an extension's terminal (its tab, or the one the host holds for a panel).
    Close(TerminalId),
    /// An extension's profiles and env; an empty one removes them.
    Integration(String, crate::TerminalIntegration),
    /// §C.3, after the card: the terminal's text for a T3 read.
    Read {
        id: TerminalId,
        scope: ReadScope,
        lines: u16,
        reply: T3Reply,
    },
    /// §C.3, after the card: type the confirmed text for `extension`.
    Write {
        id: TerminalId,
        extension: String,
        text: String,
        submit: bool,
        reply: T3Reply,
    },
}

/// Told the host's id ([`TerminalId::as_u64`]) of every terminal that closes, from any thread.
pub type TerminalClosedHook = Arc<dyn Fn(u64) + Send + Sync>;

/// Whether a terminal's program has bracketed paste on, read live from any thread.
pub(crate) type PasteProbe = Arc<dyn Fn() -> bool + Send + Sync>;

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// §B.11: the text without ESC or any other control character except tab and newline, at most
/// [`MAX_PASTE_BYTES`] of it, between the bracketed-paste markers. `None` when nothing is left.
pub(crate) fn paste_bytes(text: &str) -> Option<Vec<u8>> {
    let mut clean = String::with_capacity(text.len().min(MAX_PASTE_BYTES));
    for char in text
        .chars()
        .filter(|char| !char.is_control() || matches!(char, '\t' | '\n'))
    {
        if clean.len() + char.len_utf8() > MAX_PASTE_BYTES {
            break;
        }
        clean.push(char);
    }
    if clean.is_empty() {
        return None;
    }
    let mut bytes = Vec::with_capacity(PASTE_START.len() + clean.len() + PASTE_END.len());
    bytes.extend_from_slice(PASTE_START);
    bytes.extend_from_slice(clean.as_bytes());
    bytes.extend_from_slice(PASTE_END);
    Some(bytes)
}

/// The output side of a pseudo-terminal, owned by its extension.
struct PseudoOutput {
    extension: String,
    handle: TerminalHandle,
    sink: TerminalSink,
    /// Held while sanitising and queueing, so concurrent writes keep their order.
    writer: Mutex<(OutputSanitizer, RateLimiter)>,
}

fn output_rate() -> Rate {
    let per_second = MAX_PSEUDO_OUTPUT_BYTES_PER_SECOND as f64;
    Rate::new(per_second, per_second)
}

/// Which extension a terminal belongs to, if any (pseudo, created and profile terminals).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pub extension: String,
    pub origin: TerminalOrigin,
    /// A profile terminal's title for observers: its profile name.
    pub profile_name: Option<String>,
}

struct Known {
    status: StatusHandle,
    owner: Option<Owner>,
    title: String,
    cwd: Option<String>,
    /// The status last reported to observers.
    reported: Option<TerminalStatus>,
    pseudo: Option<Arc<PseudoOutput>>,
    /// A created terminal's extension. It hears of the end through `Exited`, from the pty.
    created: Option<String>,
    /// Set by the main thread once the terminal has a pty.
    paste_probe: Option<PasteProbe>,
}

impl Known {
    /// The extension whose own terminal this is: a pseudo or created one.
    fn owning_extension(&self) -> Option<&str> {
        self.pseudo
            .as_ref()
            .map(|pseudo| pseudo.extension.as_str())
            .or(self.created.as_deref())
    }
}

#[derive(Default)]
struct Extension {
    next_handle: u64,
    handles: HashMap<TerminalHandle, TerminalId>,
    ids: HashMap<TerminalId, TerminalHandle>,
    status_limits: HashMap<TerminalHandle, RateLimiter>,
    observer: Option<TerminalSink>,
}

impl Extension {
    fn handle_for(&mut self, id: TerminalId) -> TerminalHandle {
        if let Some(handle) = self.ids.get(&id) {
            return *handle;
        }
        self.next_handle += 1;
        let handle = TerminalHandle(self.next_handle);
        self.handles.insert(handle, id);
        self.ids.insert(id, handle);
        handle
    }
}

#[derive(Default)]
struct State {
    terminals: HashMap<TerminalId, Known>,
    extensions: HashMap<String, Extension>,
    /// The terminal whose view has keyboard focus, if any (§B.11 focus rule).
    focused: Option<TerminalId>,
}

pub struct HostFront {
    state: Mutex<State>,
    commands: UnboundedSender<Command>,
    closed_hook: Mutex<Option<TerminalClosedHook>>,
}

fn status_rate() -> Rate {
    let per_second = f64::from(MAX_STATUS_UPDATES_PER_SECOND);
    Rate::new(per_second, per_second)
}

fn to_ext_state(state: StatusState) -> osc::StatusState {
    match state {
        StatusState::Idle => osc::StatusState::Idle,
        StatusState::Working => osc::StatusState::Working,
        StatusState::Blocked => osc::StatusState::Blocked,
        StatusState::Done => osc::StatusState::Done,
        StatusState::Error => osc::StatusState::Error,
    }
}

/// The status as `fyx:ui` reports it to observers. Its source is drawn by the host, so it is
/// not redacted.
fn to_wit_status(status: &terminal_ext::TerminalStatus) -> TerminalStatus {
    let state = match status.state {
        osc::StatusState::Idle | osc::StatusState::Clear => StatusState::Idle,
        osc::StatusState::Working => StatusState::Working,
        osc::StatusState::Blocked => StatusState::Blocked,
        osc::StatusState::Done => StatusState::Done,
        osc::StatusState::Error => StatusState::Error,
    };
    let label = status
        .label
        .clone()
        .unwrap_or_else(|| match &status.source {
            StatusSource::Program { app: Some(app), .. } => {
                terminal_ext::status::app_display_name(app).to_owned()
            }
            source => source.describe(),
        });
    TerminalStatus {
        state,
        label: truncate(label, ui_ext::terminal::MAX_STATUS_LABEL_BYTES),
        detail: status
            .detail
            .clone()
            .map(|detail| truncate(detail, ui_ext::terminal::MAX_STATUS_DETAIL_BYTES)),
        progress: status.progress,
        attention: status.attention,
    }
}

fn truncate(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

impl Known {
    fn info(&self, extension: &str, handle: TerminalHandle) -> TerminalInfo {
        let (origin, title) = match &self.owner {
            Some(owner) if owner.extension == extension => (owner.origin, self.title.clone()),
            Some(Owner {
                origin: TerminalOrigin::Profile,
                profile_name: Some(name),
                ..
            }) => (TerminalOrigin::User, name.clone()),
            _ => (TerminalOrigin::User, REDACTED.to_owned()),
        };
        TerminalInfo {
            handle,
            origin,
            title,
            cwd: self.cwd.clone(),
            status: self.status.status().as_ref().map(to_wit_status),
            account: None,
        }
    }
}

impl HostFront {
    pub(crate) fn new(commands: UnboundedSender<Command>) -> Self {
        Self {
            state: Mutex::default(),
            commands,
            closed_hook: Mutex::default(),
        }
    }

    /// A terminal view gained (`true`) or lost focus. A loss clears only that terminal, so a
    /// later gain elsewhere is never undone by the old view's focus-out.
    pub(crate) fn terminal_focus_changed(&self, id: TerminalId, focused: bool) {
        let mut state = self.lock();
        if focused {
            state.focused = Some(id);
        } else if state.focused == Some(id) {
            state.focused = None;
        }
    }

    /// §B.11: whether the focused terminal is one of `ext`'s: its pseudo or created terminal, or
    /// a terminal of one of its profiles.
    pub fn extension_terminal_focused(&self, ext: &str) -> bool {
        let state = self.lock();
        state
            .focused
            .and_then(|id| state.terminals.get(&id))
            .and_then(|known| known.owner.as_ref())
            .is_some_and(|owner| owner.extension == ext)
    }

    /// Plan 5.12: the handle `ext` already holds for `id`, for a relay request from that
    /// terminal. Never allocates one: a hook does not tell an extension of a terminal it has not
    /// seen (through `observe`, or as its own).
    pub(crate) fn existing_handle(&self, ext: &str, id: TerminalId) -> Option<TerminalHandle> {
        self.lock()
            .extensions
            .get(ext)
            .and_then(|extension| extension.ids.get(&id).copied())
    }

    /// §C.3: the consent broker forgets a closed terminal's shares and ends its open cards.
    pub(crate) fn set_closed_hook(&self, hook: TerminalClosedHook) {
        *self
            .closed_hook
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(hook);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Main thread: a terminal view appeared.
    pub(crate) fn terminal_opened(
        &self,
        status: StatusHandle,
        owner: Option<Owner>,
        title: String,
        cwd: Option<String>,
    ) {
        let id = status.id();
        let events = {
            let mut state = self.lock();
            // The host registered its own terminals (pseudo, created) when it made them.
            if let Some(known) = state.terminals.get_mut(&id) {
                known.cwd = cwd;
                return;
            }
            let reported = status.status().as_ref().map(to_wit_status);
            let known = Known {
                status,
                owner,
                title,
                cwd,
                reported,
                pseudo: None,
                created: None,
                paste_probe: None,
            };
            let State {
                terminals,
                extensions,
                ..
            } = &mut *state;
            let events: Vec<_> = extensions
                .iter_mut()
                .filter_map(|(name, extension)| {
                    let sink = extension.observer.clone()?;
                    let handle = extension.handle_for(id);
                    Some((sink, TerminalEvent::Created(known.info(name, handle))))
                })
                .collect();
            terminals.insert(id, known);
            events
        };
        deliver(events);
    }

    /// Main thread, on the terminal's `Wakeup`: reports a changed status to observers.
    pub(crate) fn terminal_changed(&self, id: TerminalId, title: String, cwd: Option<String>) {
        let events = {
            let mut state = self.lock();
            let State {
                terminals,
                extensions,
                ..
            } = &mut *state;
            let Some(known) = terminals.get_mut(&id) else {
                return;
            };
            // A pseudo-terminal keeps the title its extension gave it.
            if known.pseudo.is_none() {
                known.title = title;
            }
            known.cwd = cwd;
            let status = known.status.status().as_ref().map(to_wit_status);
            if status == known.reported {
                return;
            }
            known.reported = status;
            extensions
                .iter_mut()
                .filter_map(|(name, extension)| {
                    let sink = extension.observer.clone()?;
                    let handle = extension.handle_for(id);
                    Some((sink, TerminalEvent::StatusChanged(known.info(name, handle))))
                })
                .collect::<Vec<_>>()
        };
        deliver(events);
    }

    /// Main thread: a terminal view went away, or the host closed a terminal it holds. Its
    /// handles are forgotten (and never reused); the owner of a pseudo-terminal hears through the
    /// terminal's own sink, everyone else through `observe`.
    pub(crate) fn terminal_closed(&self, id: TerminalId) {
        let hook = self
            .closed_hook
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        if let Some(hook) = hook {
            hook(id.as_u64());
        }
        let events = {
            let mut state = self.lock();
            if state.focused == Some(id) {
                state.focused = None;
            }
            let Some(known) = state.terminals.remove(&id) else {
                return;
            };
            let owner = known.owning_extension().map(str::to_owned);
            let mut events: Vec<_> = known
                .pseudo
                .iter()
                .map(|pseudo| (pseudo.sink.clone(), TerminalEvent::Closed(pseudo.handle)))
                .collect();
            events.extend(state.extensions.iter_mut().filter_map(|(name, extension)| {
                let handle = extension.ids.remove(&id)?;
                extension.handles.remove(&handle);
                extension.status_limits.remove(&handle);
                if owner.as_deref() == Some(name.as_str()) {
                    return None;
                }
                Some((extension.observer.clone()?, TerminalEvent::Closed(handle)))
            }));
            events
        };
        deliver(events);
    }

    /// Main thread: a created terminal's pty is up; `paste` may now check its mode.
    pub(crate) fn set_paste_probe(&self, id: TerminalId, probe: PasteProbe) {
        if let Some(known) = self.lock().terminals.get_mut(&id) {
            known.paste_probe = Some(probe);
        }
    }

    /// The pseudo or created terminal behind `ext`'s `handle`, if `ext` owns it.
    fn own_terminal(&self, ext: &str, handle: TerminalHandle) -> Result<TerminalId, TerminalError> {
        let state = self.lock();
        let id = *state
            .extensions
            .get(ext)
            .and_then(|extension| extension.handles.get(&handle))
            .ok_or(TerminalError::NotFound)?;
        state
            .terminals
            .get(&id)
            .filter(|known| known.owning_extension() == Some(ext))
            .map(|_| id)
            .ok_or(TerminalError::NotFound)
    }

    /// The created terminal behind `ext`'s `handle`, if `ext` owns it.
    fn own_created(&self, ext: &str, handle: TerminalHandle) -> Result<TerminalId, TerminalError> {
        let id = self.own_terminal(ext, handle)?;
        let created = self
            .lock()
            .terminals
            .get(&id)
            .is_some_and(|known| known.created.is_some());
        if created {
            Ok(id)
        } else {
            Err(TerminalError::NotFound)
        }
    }

    /// The pseudo-terminal behind `ext`'s `handle`, if `ext` owns it.
    fn own_pseudo(
        &self,
        ext: &str,
        handle: TerminalHandle,
    ) -> Result<(TerminalId, Arc<PseudoOutput>), TerminalError> {
        let state = self.lock();
        let id = *state
            .extensions
            .get(ext)
            .and_then(|extension| extension.handles.get(&handle))
            .ok_or(TerminalError::NotFound)?;
        let pseudo = state
            .terminals
            .get(&id)
            .and_then(|known| known.pseudo.clone())
            .filter(|pseudo| pseudo.extension == ext)
            .ok_or(TerminalError::NotFound)?;
        Ok((id, pseudo))
    }

    /// §C.3: any terminal `ext` has a handle for (from `observe`, or its own), except another
    /// extension's pseudo or created terminal: T3 is for the user's terminals.
    fn t3_terminal(&self, ext: &str, handle: TerminalHandle) -> Result<TerminalId, TerminalError> {
        let state = self.lock();
        let id = *state
            .extensions
            .get(ext)
            .and_then(|extension| extension.handles.get(&handle))
            .ok_or(TerminalError::NotFound)?;
        let known = state.terminals.get(&id).ok_or(TerminalError::NotFound)?;
        match known.owning_extension() {
            Some(owner) if owner != ext => Err(TerminalError::Denied),
            _ => Ok(id),
        }
    }

    /// For `ui_ext_host`'s `terminal` node: the terminal behind `ext`'s own `handle`.
    pub(crate) fn owned_terminal(&self, ext: &str, handle: TerminalHandle) -> Option<TerminalId> {
        self.own_terminal(ext, handle).ok()
    }

    fn send(&self, command: Command) -> Result<(), TerminalError> {
        self.commands
            .unbounded_send(command)
            .map_err(|_| TerminalError::Unavailable("the terminal host stopped".into()))
    }
}

/// Sinks are called with no lock held (`TerminalSink`'s contract).
fn deliver(events: Vec<(TerminalSink, TerminalEvent)>) {
    for (sink, event) in events {
        sink(event);
    }
}

impl TerminalHost for HostFront {
    fn open_pseudo(
        &self,
        ext: &str,
        request: PseudoRequest,
        sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError> {
        let (terminal, io) = pseudo::new_terminal_ext(ext);
        let id = terminal.id();
        let (handle, events) = {
            let mut state = self.lock();
            let State {
                terminals,
                extensions,
                ..
            } = &mut *state;
            let handle = extensions.entry(ext.to_owned()).or_default().handle_for(id);
            let known = Known {
                status: terminal.status_handle(),
                owner: Some(Owner {
                    extension: ext.to_owned(),
                    origin: TerminalOrigin::Pseudo,
                    profile_name: None,
                }),
                title: request.title.clone(),
                cwd: None,
                reported: None,
                created: None,
                paste_probe: None,
                pseudo: Some(Arc::new(PseudoOutput {
                    extension: ext.to_owned(),
                    handle,
                    sink: sink.clone(),
                    writer: Mutex::new((OutputSanitizer::new(), RateLimiter::new(output_rate()))),
                })),
            };
            let events: Vec<_> = extensions
                .iter_mut()
                .filter_map(|(name, extension)| {
                    let sink = extension.observer.clone()?;
                    let handle = extension.handle_for(id);
                    Some((sink, TerminalEvent::Created(known.info(name, handle))))
                })
                .collect();
            terminals.insert(id, known);
            (handle, events)
        };
        if let Err(error) = self.send(Command::OpenPseudo(PseudoOpen {
            ext: terminal,
            io,
            sink,
            handle,
            title: request.title,
            in_panel: request.in_panel,
        })) {
            self.terminal_closed(id);
            return Err(error);
        }
        deliver(events);
        Ok(handle)
    }

    fn create(
        &self,
        ext: &str,
        command: CreateCommand,
        sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError> {
        let (tap, woken) = OutputTap::new();
        let terminal = terminal_ext::TerminalExt::created(ext, tap.clone());
        let id = terminal.id();
        let title = command.title.clone().unwrap_or_else(|| {
            command.program.file_stem().map_or_else(
                || command.program.to_string_lossy().into_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            )
        });
        let (handle, events) = {
            let mut state = self.lock();
            let State {
                terminals,
                extensions,
                ..
            } = &mut *state;
            let created = terminals
                .values()
                .filter(|known| known.created.as_deref() == Some(ext))
                .count();
            if created >= MAX_CREATED_TERMINALS {
                return Err(TerminalError::Unavailable(format!(
                    "an extension has at most {MAX_CREATED_TERMINALS} created terminals"
                )));
            }
            let handle = extensions.entry(ext.to_owned()).or_default().handle_for(id);
            let known = Known {
                status: terminal.status_handle(),
                owner: Some(Owner {
                    extension: ext.to_owned(),
                    origin: TerminalOrigin::Created,
                    profile_name: None,
                }),
                title,
                cwd: Some(command.cwd.to_string_lossy().into_owned()),
                reported: None,
                pseudo: None,
                created: Some(ext.to_owned()),
                paste_probe: None,
            };
            let events: Vec<_> = extensions
                .iter_mut()
                .filter_map(|(name, extension)| {
                    let sink = extension.observer.clone()?;
                    let handle = extension.handle_for(id);
                    Some((sink, TerminalEvent::Created(known.info(name, handle))))
                })
                .collect();
            terminals.insert(id, known);
            (handle, events)
        };
        if let Err(error) = self.send(Command::OpenCreated(CreatedOpen {
            id,
            parked: terminal.park(),
            tap,
            woken,
            sink,
            handle,
            command,
        })) {
            self.terminal_closed(id);
            return Err(error);
        }
        deliver(events);
        Ok(handle)
    }

    fn write_output(
        &self,
        ext: &str,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), TerminalError> {
        if bytes.len() > MAX_WRITE_OUTPUT_BYTES {
            return Err(TerminalError::TooLarge(format!(
                "write-output takes at most {MAX_WRITE_OUTPUT_BYTES} bytes"
            )));
        }
        let (id, pseudo) = self.own_pseudo(ext, handle)?;
        let mut writer = pseudo
            .writer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (sanitizer, limit) = &mut *writer;
        if !limit.allow_cost_at(bytes.len() as f64, Instant::now()) {
            return Err(TerminalError::RateLimited);
        }
        let clean = sanitizer.sanitize(&bytes);
        if !clean.is_empty() {
            self.send(Command::Output(id, clean))?;
        }
        Ok(())
    }

    fn send_input(
        &self,
        ext: &str,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), TerminalError> {
        if bytes.len() > MAX_SEND_INPUT_BYTES {
            return Err(TerminalError::TooLarge(format!(
                "send-input takes at most {MAX_SEND_INPUT_BYTES} bytes"
            )));
        }
        let id = self.own_created(ext, handle)?;
        if !bytes.is_empty() {
            self.send(Command::Input(id, bytes))?;
        }
        Ok(())
    }

    /// Into the extension's created terminals (profile terminals join with P2.4). The import
    /// checks the user's gesture; the host checks the mode, because a newline outside bracketed
    /// paste would run a command.
    fn paste(&self, ext: &str, handle: TerminalHandle, text: String) -> Result<(), TerminalError> {
        let id = self.own_created(ext, handle)?;
        let probe = self
            .lock()
            .terminals
            .get(&id)
            .and_then(|known| known.paste_probe.clone())
            .ok_or_else(|| TerminalError::Unavailable("the terminal is still starting".into()))?;
        if !probe() {
            return Err(TerminalError::Unavailable(
                "the program in the terminal has bracketed paste off".into(),
            ));
        }
        match paste_bytes(&text) {
            Some(bytes) => self.send(Command::Paste(id, bytes)),
            None => Ok(()),
        }
    }

    fn set_status(
        &self,
        ext: &str,
        handle: TerminalHandle,
        status: Option<TerminalStatus>,
    ) -> Result<(), TerminalError> {
        let status_handle = {
            let mut state = self.lock();
            let State {
                terminals,
                extensions,
                ..
            } = &mut *state;
            let extension = extensions.get_mut(ext).ok_or(TerminalError::NotFound)?;
            let id = *extension
                .handles
                .get(&handle)
                .ok_or(TerminalError::NotFound)?;
            let known = terminals.get(&id).ok_or(TerminalError::NotFound)?;
            let allowed = extension
                .status_limits
                .entry(handle)
                .or_insert_with(|| RateLimiter::new(status_rate()))
                .allow();
            if !allowed {
                return Err(TerminalError::RateLimited);
            }
            known.status.clone()
        };
        status_handle.set_extension_status(
            ext,
            status.map(|status| ExtensionStatus {
                state: to_ext_state(status.state),
                label: status.label,
                detail: status.detail,
                progress: status.progress,
                attention: status.attention,
            }),
        );
        self.commands
            .unbounded_send(Command::Repaint(status_handle.id()))
            .ok();
        Ok(())
    }

    fn close(&self, ext: &str, handle: TerminalHandle) {
        match self.own_terminal(ext, handle) {
            Ok(id) => {
                self.send(Command::Close(id)).ok();
            }
            // A user terminal stays open: the extension only drops its status on it.
            Err(_) => {
                self.set_status(ext, handle, None).ok();
            }
        }
    }

    fn observe(&self, ext: &str, sink: Option<TerminalSink>) -> Vec<TerminalInfo> {
        let mut state = self.lock();
        let State {
            terminals,
            extensions,
            ..
        } = &mut *state;
        let extension = extensions.entry(ext.to_owned()).or_default();
        let observing = sink.is_some();
        extension.observer = sink;
        if !observing {
            return Vec::new();
        }
        // New handles go to older terminals first.
        let mut known: Vec<_> = terminals.iter().collect();
        known.sort_by_key(|(id, _)| **id);
        let mut infos: Vec<_> = known
            .into_iter()
            .map(|(id, known)| known.info(ext, extension.handle_for(*id)))
            .collect();
        infos.sort_by_key(|info| info.handle);
        infos
    }

    /// An extension's profiles and granted `terminal:env` (design §B.1, §B.4), set by the runtime
    /// on every load; an empty integration removes them, and so does [`TerminalHost::close_all`].
    /// Terminals already open keep their environment. Menu entries name the extension by its id
    /// until the runtime passes a display name.
    fn set_integration(&self, ext: &str, integration: ui_ext::terminal::TerminalIntegration) {
        let ui_ext::terminal::TerminalIntegration {
            profiles,
            env,
            env_all_new,
            package_dir,
        } = integration;
        let integration = crate::TerminalIntegration {
            extension_name: ext.to_owned(),
            profiles,
            env,
            env_all_new,
            package_dir,
        };
        self.send(Command::Integration(ext.to_owned(), integration))
            .ok();
    }

    fn t3_target(&self, ext: &str, handle: TerminalHandle) -> Result<T3Target, TerminalError> {
        let id = self.t3_terminal(ext, handle)?;
        let state = self.lock();
        let known = state.terminals.get(&id).ok_or(TerminalError::NotFound)?;
        let origin = match &known.owner {
            Some(owner) if owner.extension == ext || owner.origin == TerminalOrigin::Profile => {
                owner.origin
            }
            _ => TerminalOrigin::User,
        };
        Ok(T3Target {
            terminal: id.as_u64(),
            origin,
            title: known.title.clone(),
            cwd: known.cwd.as_ref().map(std::path::PathBuf::from),
        })
    }

    fn read(
        &self,
        ext: &str,
        handle: TerminalHandle,
        scope: ReadScope,
        lines: u16,
        id: u64,
        sink: TerminalSink,
    ) {
        let terminal = match self.t3_terminal(ext, handle) {
            Ok(terminal) => terminal,
            Err(error) => {
                sink(TerminalEvent::ReadDone(id, Err(error.to_string())));
                return;
            }
        };
        let command = Command::Read {
            id: terminal,
            scope,
            lines: lines.clamp(1, MAX_READ_LINES),
            reply: T3Reply { id, sink },
        };
        if let Err(error) = self.commands.unbounded_send(command)
            && let Command::Read { reply, .. } = error.into_inner()
        {
            (reply.sink)(TerminalEvent::ReadDone(id, Err("unavailable".into())));
        }
    }

    fn write(
        &self,
        ext: &str,
        handle: TerminalHandle,
        text: String,
        submit: bool,
        id: u64,
        sink: TerminalSink,
    ) {
        let checked = self
            .t3_terminal(ext, handle)
            .map_err(|error| error.to_string())
            .and_then(|terminal| {
                ui_ext::terminal::check_write_text(&text)
                    .map(|()| terminal)
                    .map_err(|error| error.message)
            });
        let terminal = match checked {
            Ok(terminal) => terminal,
            Err(error) => {
                sink(TerminalEvent::WriteDone(id, Err(error)));
                return;
            }
        };
        let command = Command::Write {
            id: terminal,
            extension: ext.to_owned(),
            text,
            submit,
            reply: T3Reply { id, sink },
        };
        if let Err(error) = self.commands.unbounded_send(command)
            && let Command::Write { reply, .. } = error.into_inner()
        {
            (reply.sink)(TerminalEvent::WriteDone(id, Err("unavailable".into())));
        }
    }

    fn close_all(&self, ext: &str) {
        let (statuses, owned, extension) = {
            let mut state = self.lock();
            let extension = state.extensions.remove(ext);
            let statuses: Vec<_> = state
                .terminals
                .values()
                .map(|known| known.status.clone())
                .collect();
            let owned: Vec<_> = state
                .terminals
                .iter()
                .filter(|(_, known)| known.owning_extension() == Some(ext))
                .map(|(id, _)| *id)
                .collect();
            (statuses, owned, extension)
        };
        drop(extension);
        self.send(Command::Integration(
            ext.to_owned(),
            crate::TerminalIntegration::default(),
        ))
        .ok();
        for id in owned {
            self.send(Command::Close(id)).ok();
        }
        for status in statuses {
            status.set_extension_status(ext, None);
            self.commands
                .unbounded_send(Command::Repaint(status.id()))
                .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::channel::mpsc;
    use terminal_ext::TerminalExt;

    use super::*;

    fn status(state: StatusState) -> TerminalStatus {
        TerminalStatus {
            state,
            label: "Review".into(),
            detail: None,
            progress: None,
            attention: false,
        }
    }

    fn recorder() -> (TerminalSink, Arc<Mutex<Vec<TerminalEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink: TerminalSink = Arc::new({
            let events = events.clone();
            move |event| events.lock().unwrap().push(event)
        });
        (sink, events)
    }

    #[test]
    fn observe_set_status_and_close_all() {
        let (commands, mut repaints) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let user = TerminalExt::new();
        host.terminal_opened(
            user.status_handle(),
            None,
            "cargo test".into(),
            Some("/w".into()),
        );

        // Unknown extension or handle.
        assert_eq!(
            host.set_status(
                "acme",
                TerminalHandle(1),
                Some(status(StatusState::Working))
            ),
            Err(TerminalError::NotFound)
        );

        let (sink, events) = recorder();
        let infos = host.observe("acme", Some(sink));
        assert_eq!(infos.len(), 1);
        let info = &infos[0];
        assert_eq!(
            (info.origin, info.title.as_str()),
            (TerminalOrigin::User, REDACTED)
        );
        assert_eq!(info.cwd.as_deref(), Some("/w"));
        let handle = info.handle;

        host.set_status("acme", handle, Some(status(StatusState::Working)))
            .unwrap();
        let shown = user.status().unwrap();
        assert_eq!(shown.source.describe(), "from extension acme");
        assert!(matches!(repaints.try_recv(), Ok(Command::Repaint(id)) if id == user.id()));

        // The main thread reports the change on Wakeup; observers hear about it once.
        host.terminal_changed(user.id(), "cargo test".into(), Some("/w".into()));
        host.terminal_changed(user.id(), "cargo test".into(), Some("/w".into()));
        let recorded = events.lock().unwrap().clone();
        assert!(matches!(
            recorded.as_slice(),
            [TerminalEvent::StatusChanged(info)]
                if info.status.as_ref().map(|s| s.state) == Some(StatusState::Working)
        ));

        // Four per second, then rate limited.
        for _ in 0..3 {
            host.set_status("acme", handle, Some(status(StatusState::Working)))
                .unwrap();
        }
        assert_eq!(
            host.set_status("acme", handle, Some(status(StatusState::Done))),
            Err(TerminalError::RateLimited)
        );

        // Another extension gets its own handle space.
        let (other_sink, _) = recorder();
        let other = host.observe("other", Some(other_sink));
        assert_eq!(other[0].handle, TerminalHandle(1));

        host.close_all("acme");
        assert!(user.status().is_none());
        assert_eq!(
            host.set_status("acme", handle, None),
            Err(TerminalError::NotFound)
        );

        host.terminal_closed(user.id());
        assert!(host.observe("other", None).is_empty());
    }

    #[test]
    fn owners_see_their_titles_and_profiles_show_their_names() {
        let (commands, _repaints) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let created = TerminalExt::new();
        let profile = TerminalExt::new();
        host.terminal_opened(
            created.status_handle(),
            Some(Owner {
                extension: "acme".into(),
                origin: TerminalOrigin::Created,
                profile_name: None,
            }),
            "codex -c notify=x".into(),
            None,
        );
        host.terminal_opened(
            profile.status_handle(),
            Some(Owner {
                extension: "acme".into(),
                origin: TerminalOrigin::Profile,
                profile_name: Some("Claude Code".into()),
            }),
            "claude --ide".into(),
            None,
        );
        let titles = |ext: &str| {
            let (sink, _) = recorder();
            host.observe(ext, Some(sink))
                .into_iter()
                .map(|info| (info.origin, info.title))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles("acme"),
            [
                (TerminalOrigin::Created, "codex -c notify=x".to_owned()),
                (TerminalOrigin::Profile, "claude --ide".to_owned())
            ]
        );
        assert_eq!(
            titles("other"),
            [
                (TerminalOrigin::User, REDACTED.to_owned()),
                (TerminalOrigin::User, "Claude Code".to_owned())
            ]
        );
    }

    fn pseudo_request(title: &str) -> PseudoRequest {
        PseudoRequest {
            title: title.into(),
            cols: None,
            rows: None,
            in_panel: false,
        }
    }

    #[test]
    fn pseudo_terminals_are_their_owners_and_get_sanitised_output() {
        let (commands, mut queued) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let (observer, observed) = recorder();
        host.observe("other", Some(observer));

        let (sink, owner_events) = recorder();
        let handle = host
            .open_pseudo("acme", pseudo_request("ssh box"), sink)
            .unwrap();
        let Ok(Command::OpenPseudo(open)) = queued.try_recv() else {
            panic!("no open command");
        };
        assert_eq!(open.handle, handle);
        assert_eq!(open.title, "ssh box");
        let id = open.ext.id();
        assert!(open.ext.is_pseudo());
        assert_eq!(
            open.ext.origin(),
            Some(terminal_ext::ExtensionOrigin::Pseudo {
                extension: "acme".into()
            })
        );
        // Others see a redacted user terminal.
        assert!(matches!(
            observed.lock().unwrap().as_slice(),
            [TerminalEvent::Created(info)] if info.title == REDACTED && info.origin == TerminalOrigin::User
        ));
        // The owner sees its own title and origin.
        let (own_observer, _) = recorder();
        let mine = host.observe("acme", Some(own_observer));
        assert_eq!(
            (mine[0].handle, mine[0].origin, mine[0].title.as_str()),
            (handle, TerminalOrigin::Pseudo, "ssh box")
        );

        // The view registering later keeps the owner and title.
        host.terminal_opened(open.ext.status_handle(), None, "Terminal".into(), None);
        host.terminal_changed(id, "Terminal".into(), None);
        assert_eq!(host.observe("acme", None).len(), 0);
        let (own_observer, _) = recorder();
        assert_eq!(host.observe("acme", Some(own_observer))[0].title, "ssh box");

        // Output is sanitised; a query alone queues nothing.
        host.write_output("acme", handle, b"hi\x1b[6n\x1b]52;c;aGk=\x07!".to_vec())
            .unwrap();
        assert!(
            matches!(queued.try_recv(), Ok(Command::Output(out_id, bytes)) if out_id == id && bytes == b"hi!")
        );
        host.write_output("acme", handle, b"\x1b[c".to_vec())
            .unwrap();
        assert!(queued.try_recv().is_err());

        // Only the owner may write; nobody may write more than one call's worth.
        assert_eq!(
            host.write_output("other", TerminalHandle(1), b"x".to_vec()),
            Err(TerminalError::NotFound)
        );
        assert!(matches!(
            host.write_output("acme", handle, vec![b'x'; MAX_WRITE_OUTPUT_BYTES + 1]),
            Err(TerminalError::TooLarge(_))
        ));
        // About 4 MiB in a burst (the bucket refills while the loop runs), then rate limited.
        let chunk = vec![b'x'; MAX_WRITE_OUTPUT_BYTES];
        let accepted = (0..200)
            .take_while(|_| host.write_output("acme", handle, chunk.clone()).is_ok())
            .count();
        let burst = MAX_PSEUDO_OUTPUT_BYTES_PER_SECOND / MAX_WRITE_OUTPUT_BYTES;
        assert!(
            (burst - 1..200).contains(&accepted),
            "{accepted} writes before the limit"
        );
        while queued.try_recv().is_ok() {}

        // Closing asks the main thread to close the tab; the owner hears once the view is gone.
        host.close("acme", handle);
        assert!(matches!(queued.try_recv(), Ok(Command::Close(close_id)) if close_id == id));
        host.terminal_closed(id);
        assert_eq!(
            owner_events.lock().unwrap().as_slice(),
            [TerminalEvent::Closed(handle)]
        );
        assert!(matches!(
            observed.lock().unwrap().last(),
            Some(TerminalEvent::Closed(TerminalHandle(1)))
        ));
        assert_eq!(
            host.write_output("acme", handle, b"x".to_vec()),
            Err(TerminalError::NotFound)
        );
        // Handles are not reused.
        let (sink, _) = recorder();
        let next = host
            .open_pseudo("acme", pseudo_request("again"), sink)
            .unwrap();
        assert_ne!(next, handle);
    }

    fn create_command(title: Option<&str>) -> CreateCommand {
        CreateCommand {
            program: std::path::PathBuf::from("/tools/codex.exe"),
            args: vec!["--quiet".into()],
            cwd: std::path::PathBuf::from("/work"),
            title: title.map(str::to_owned),
            env: Vec::new(),
        }
    }

    #[test]
    fn created_terminals_belong_to_their_extension_and_take_its_input() {
        let (commands, mut queued) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let (observer, observed) = recorder();
        host.observe("other", Some(observer));

        let (sink, owner_events) = recorder();
        let handle = host.create("acme", create_command(None), sink).unwrap();
        let Ok(Command::OpenCreated(open)) = queued.try_recv() else {
            panic!("no open command");
        };
        assert_eq!(open.handle, handle);
        let id = open.id;
        // The builder picks up the parked state: the tap is on the pty from the first byte.
        let mut env = HashMap::from([open.parked.env()]);
        let ext = TerminalExt::insert_env(&mut env);
        assert_eq!(ext.id(), id);
        assert!(ext.token().is_some());
        assert_eq!(
            ext.origin(),
            Some(terminal_ext::ExtensionOrigin::Created {
                extension: "acme".into()
            })
        );
        assert!(matches!(
            observed.lock().unwrap().as_slice(),
            [TerminalEvent::Created(info)] if info.title == REDACTED && info.origin == TerminalOrigin::User
        ));
        let (own_observer, _) = recorder();
        let mine = host.observe("acme", Some(own_observer));
        assert_eq!(
            (mine[0].handle, mine[0].origin, mine[0].title.as_str()),
            (handle, TerminalOrigin::Created, "codex")
        );
        assert_eq!(mine[0].cwd.as_deref(), Some("/work"));

        // Input: the owner's only, one call's worth at most.
        host.send_input("acme", handle, b"y\r".to_vec()).unwrap();
        assert!(
            matches!(queued.try_recv(), Ok(Command::Input(input_id, bytes)) if input_id == id && bytes == b"y\r")
        );
        assert_eq!(
            host.send_input("other", TerminalHandle(1), b"x".to_vec()),
            Err(TerminalError::NotFound)
        );
        assert!(matches!(
            host.send_input("acme", handle, vec![b'x'; MAX_SEND_INPUT_BYTES + 1]),
            Err(TerminalError::TooLarge(_))
        ));
        // A created terminal is not a pseudo-terminal, and a pseudo-terminal takes no input.
        assert_eq!(
            host.write_output("acme", handle, b"x".to_vec()),
            Err(TerminalError::NotFound)
        );
        let (pseudo_sink, _) = recorder();
        let pseudo = host
            .open_pseudo("acme", pseudo_request("p"), pseudo_sink)
            .unwrap();
        assert_eq!(
            host.send_input("acme", pseudo, b"x".to_vec()),
            Err(TerminalError::NotFound)
        );
        while queued.try_recv().is_ok() {}

        // Closing asks the main thread; the owner hears `Exited` from the pty, not `Closed`.
        host.close("acme", handle);
        assert!(matches!(queued.try_recv(), Ok(Command::Close(close_id)) if close_id == id));
        host.terminal_closed(id);
        assert!(owner_events.lock().unwrap().is_empty());
        assert!(matches!(
            observed.lock().unwrap().last(),
            Some(TerminalEvent::Closed(TerminalHandle(1)))
        ));
        assert_eq!(
            host.send_input("acme", handle, b"x".to_vec()),
            Err(TerminalError::NotFound)
        );
        drop(ext);
    }

    #[test]
    fn pastes_are_bracketed_clean_and_capped() {
        assert_eq!(
            paste_bytes("@src/main.rs#L5-10\tok\n\x1b[201~rm -rf /\r\x07\u{9b}x").unwrap(),
            b"\x1b[200~@src/main.rs#L5-10\tok\n[201~rm -rf /x\x1b[201~"
        );
        assert_eq!(paste_bytes("\x1b\x03\r"), None);
        let long = "é".repeat(MAX_PASTE_BYTES);
        let bytes = paste_bytes(&long).unwrap();
        let inner = &bytes[PASTE_START.len()..bytes.len() - PASTE_END.len()];
        assert!(inner.len() <= MAX_PASTE_BYTES && inner.len() > MAX_PASTE_BYTES - 2);
        assert!(std::str::from_utf8(inner).is_ok());
    }

    #[test]
    fn paste_goes_only_into_an_own_created_terminal_with_bracketed_paste_on() {
        let (commands, mut queued) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let (sink, _) = recorder();
        let handle = host.create("acme", create_command(None), sink).unwrap();
        let Ok(Command::OpenCreated(open)) = queued.try_recv() else {
            panic!("no open command");
        };
        assert!(matches!(
            host.paste("acme", handle, "x".into()),
            Err(TerminalError::Unavailable(_))
        ));
        let bracketed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        host.set_paste_probe(open.id, {
            let bracketed = bracketed.clone();
            Arc::new(move || bracketed.load(std::sync::atomic::Ordering::Relaxed))
        });
        assert!(matches!(
            host.paste("acme", handle, "x".into()),
            Err(TerminalError::Unavailable(_))
        ));
        bracketed.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            host.paste("other", handle, "x".into()),
            Err(TerminalError::NotFound)
        );
        host.paste("acme", handle, "x\n".into()).unwrap();
        assert!(matches!(
            queued.try_recv(),
            Ok(Command::Paste(id, bytes)) if id == open.id && bytes == b"\x1b[200~x\n\x1b[201~"
        ));
        let (pseudo_sink, _) = recorder();
        let pseudo = host
            .open_pseudo("acme", pseudo_request("p"), pseudo_sink)
            .unwrap();
        assert_eq!(
            host.paste("acme", pseudo, "x".into()),
            Err(TerminalError::NotFound)
        );
    }

    #[test]
    fn an_extension_has_at_most_8_created_terminals_and_close_all_ends_them() {
        let (commands, mut queued) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let (sink, _) = recorder();
        for _ in 0..MAX_CREATED_TERMINALS {
            host.create("acme", create_command(Some("t")), sink.clone())
                .unwrap();
        }
        assert!(matches!(
            host.create("acme", create_command(None), sink.clone()),
            Err(TerminalError::Unavailable(_))
        ));
        host.create("other", create_command(None), sink).unwrap();
        let mut opened = Vec::new();
        while let Ok(command) = queued.try_recv() {
            if let Command::OpenCreated(open) = command {
                opened.push(open);
            }
        }
        assert_eq!(opened.len(), MAX_CREATED_TERMINALS + 1);
        host.close_all("acme");
        let mut closed: Vec<_> = std::iter::from_fn(|| queued.try_recv().ok())
            .filter_map(|command| match command {
                Command::Close(id) => Some(id),
                _ => None,
            })
            .collect();
        closed.sort();
        let mut acme: Vec<_> = opened[..MAX_CREATED_TERMINALS]
            .iter()
            .map(|open| open.id)
            .collect();
        acme.sort();
        assert_eq!(closed, acme);
    }

    #[test]
    fn close_all_closes_an_extensions_pseudo_terminals() {
        let (commands, mut queued) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let (sink, _) = recorder();
        host.open_pseudo("acme", pseudo_request("a"), sink.clone())
            .unwrap();
        host.open_pseudo("other", pseudo_request("b"), sink)
            .unwrap();
        let mut ids = HashMap::new();
        while let Ok(Command::OpenPseudo(open)) = queued.try_recv() {
            ids.insert(open.title.clone(), open.ext.id());
            // Keep the terminal alive as the main thread would.
            std::mem::forget(open.ext);
        }
        host.close_all("acme");
        let closed: Vec<_> = std::iter::from_fn(|| queued.try_recv().ok())
            .filter_map(|command| match command {
                Command::Close(id) => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(closed, [ids["a"]]);
    }

    #[test]
    fn t3_targets_reads_and_writes_go_to_user_terminals_only() {
        let (commands, mut queued) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let closed = Arc::new(Mutex::new(Vec::new()));
        host.set_closed_hook(Arc::new({
            let closed = closed.clone();
            move |id| closed.lock().unwrap().push(id)
        }));
        let user = TerminalExt::new();
        host.terminal_opened(
            user.status_handle(),
            None,
            "vim notes".into(),
            Some("/w".into()),
        );
        let (pseudo_sink, _) = recorder();
        host.open_pseudo("acme", pseudo_request("ssh box"), pseudo_sink)
            .unwrap();
        while queued.try_recv().is_ok() {}

        let (observer, _) = recorder();
        let infos = host.observe("other", Some(observer));
        let user_handle = infos[0].handle;
        let acme_handle = infos[1].handle;

        // The card shows the real title; observers saw it redacted.
        assert_eq!(infos[0].title, REDACTED);
        let target = host.t3_target("other", user_handle).unwrap();
        assert_eq!(
            target,
            T3Target {
                terminal: user.id().as_u64(),
                origin: TerminalOrigin::User,
                title: "vim notes".into(),
                cwd: Some("/w".into()),
            }
        );
        assert_eq!(
            host.t3_target("other", acme_handle),
            Err(TerminalError::Denied),
            "another extension's terminal"
        );
        assert_eq!(
            host.t3_target("other", TerminalHandle(99)),
            Err(TerminalError::NotFound)
        );

        let (sink, answers) = recorder();
        host.read(
            "other",
            user_handle,
            ReadScope::LastLines,
            9999,
            1,
            sink.clone(),
        );
        assert!(matches!(
            queued.try_recv(),
            Ok(Command::Read { id, lines: MAX_READ_LINES, reply, .. })
                if id == user.id() && reply.id == 1
        ));
        host.read(
            "other",
            acme_handle,
            ReadScope::LastLines,
            5,
            2,
            sink.clone(),
        );
        host.write(
            "other",
            user_handle,
            "rm -rf /".into(),
            true,
            3,
            sink.clone(),
        );
        host.write("other", user_handle, "ls	-la".into(), true, 4, sink);
        assert!(matches!(
            queued.try_recv(),
            Ok(Command::Write { id, extension, submit: true, reply, .. })
                if id == user.id() && extension == "other" && reply.id == 4
        ));
        let answers = answers.lock().unwrap().clone();
        assert!(matches!(
            answers.as_slice(),
            [
                TerminalEvent::ReadDone(2, Err(_)),
                TerminalEvent::WriteDone(3, Err(_)),
            ]
        ));

        host.terminal_closed(user.id());
        assert_eq!(*closed.lock().unwrap(), [user.id().as_u64()]);
    }

    #[test]
    fn focus_rule_and_relay_handles() {
        let (commands, _repaints) = mpsc::unbounded();
        let host = HostFront::new(commands);
        let user = TerminalExt::new();
        let profile = TerminalExt::new();
        host.terminal_opened(user.status_handle(), None, "zsh".into(), None);
        host.terminal_opened(
            profile.status_handle(),
            Some(Owner {
                extension: "acme".into(),
                origin: TerminalOrigin::Profile,
                profile_name: Some("Claude Code".into()),
            }),
            "claude".into(),
            None,
        );
        assert!(!host.extension_terminal_focused("acme"));

        host.terminal_focus_changed(profile.id(), true);
        assert!(host.extension_terminal_focused("acme"));
        assert!(!host.extension_terminal_focused("other"));

        // The new view's focus-in may come before the old one's focus-out.
        host.terminal_focus_changed(user.id(), true);
        host.terminal_focus_changed(profile.id(), false);
        assert!(!host.extension_terminal_focused("acme"));
        host.terminal_focus_changed(profile.id(), true);
        host.terminal_closed(profile.id());
        assert!(!host.extension_terminal_focused("acme"));

        // A relay request names only a terminal the extension has a handle for.
        assert_eq!(host.existing_handle("acme", user.id()), None);
        let (sink, _) = recorder();
        let infos = host.observe("acme", Some(sink));
        assert_eq!(
            host.existing_handle("acme", user.id()),
            Some(infos[0].handle)
        );
        assert_eq!(host.existing_handle("other", user.id()), None);
    }
}
