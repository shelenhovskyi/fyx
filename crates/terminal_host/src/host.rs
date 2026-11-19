//! `ui_ext::terminal::TerminalHost` for `fyx:ui@0.4` extensions (design `docs/fyx/wit-0.4/design.md`
//! §B). Extensions call it from the runtime's threads, so this is a `Send + Sync` front: it checks,
//! allocates handles and rate-limits on the calling thread, and queues work that needs GPUI to the
//! main thread ([`Command`]). The main thread keeps it told about every terminal ([`HostFront::
//! terminal_opened`] and friends), so `observe` answers without a round trip.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use futures::channel::mpsc::UnboundedSender;
use terminal_ext::inbox::{Rate, RateLimiter};
use terminal_ext::{ExtensionStatus, StatusHandle, StatusSource, TerminalId, osc};
use ui_ext::terminal::{
    CreateCommand, MAX_STATUS_UPDATES_PER_SECOND, PseudoRequest, StatusState, TerminalError,
    TerminalEvent, TerminalHandle, TerminalHost, TerminalInfo, TerminalOrigin, TerminalSink,
    TerminalStatus,
};

/// The title another extension sees for a terminal that is not its own (W2c, 282e99d69a).
pub const REDACTED: &str = "•••";

/// Work for the main thread.
#[derive(Debug)]
pub(crate) enum Command {
    /// A status changed off the main thread: send the terminal a `Wakeup` so its tab repaints.
    Repaint(TerminalId),
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
}

pub struct HostFront {
    state: Mutex<State>,
    commands: UnboundedSender<Command>,
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
        }
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
            let reported = status.status().as_ref().map(to_wit_status);
            let known = Known {
                status,
                owner,
                title,
                cwd,
                reported,
            };
            let State {
                terminals,
                extensions,
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
            } = &mut *state;
            let Some(known) = terminals.get_mut(&id) else {
                return;
            };
            known.title = title;
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

    /// Main thread: a terminal view went away.
    pub(crate) fn terminal_closed(&self, id: TerminalId) {
        let events = {
            let mut state = self.lock();
            if state.terminals.remove(&id).is_none() {
                return;
            }
            state
                .extensions
                .values_mut()
                .filter_map(|extension| {
                    let handle = extension.ids.get(&id).copied();
                    let sink = extension.observer.clone();
                    if let Some(handle) = handle {
                        extension.status_limits.remove(&handle);
                    }
                    Some((sink?, TerminalEvent::Closed(handle?)))
                })
                .collect::<Vec<_>>()
        };
        deliver(events);
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
        _ext: &str,
        _request: PseudoRequest,
        _sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError> {
        Err(TerminalError::Unavailable(
            "pseudo-terminals are not implemented yet".into(),
        ))
    }

    fn create(
        &self,
        _ext: &str,
        _command: CreateCommand,
        _sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError> {
        Err(TerminalError::Unavailable(
            "created terminals are not implemented yet".into(),
        ))
    }

    fn write_output(
        &self,
        _ext: &str,
        _handle: TerminalHandle,
        _bytes: Vec<u8>,
    ) -> Result<(), TerminalError> {
        Err(TerminalError::NotFound)
    }

    fn send_input(
        &self,
        _ext: &str,
        _handle: TerminalHandle,
        _bytes: Vec<u8>,
    ) -> Result<(), TerminalError> {
        Err(TerminalError::NotFound)
    }

    fn paste(
        &self,
        _ext: &str,
        _handle: TerminalHandle,
        _text: String,
    ) -> Result<(), TerminalError> {
        Err(TerminalError::Unavailable(
            "paste is not implemented yet".into(),
        ))
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
        // A user terminal stays open: the extension only drops its status on it.
        self.set_status(ext, handle, None).ok();
    }

    fn observe(&self, ext: &str, sink: Option<TerminalSink>) -> Vec<TerminalInfo> {
        let mut state = self.lock();
        let State {
            terminals,
            extensions,
        } = &mut *state;
        let extension = extensions.entry(ext.to_owned()).or_default();
        let observing = sink.is_some();
        extension.observer = sink;
        if !observing {
            return Vec::new();
        }
        // New handles go to older terminals first, so the order is defined.
        let mut known: Vec<_> = terminals.iter().collect();
        known.sort_by_key(|(id, _)| **id);
        let mut infos: Vec<_> = known
            .into_iter()
            .map(|(id, known)| known.info(ext, extension.handle_for(*id)))
            .collect();
        infos.sort_by_key(|info| info.handle);
        infos
    }

    fn close_all(&self, ext: &str) {
        let (statuses, extension) = {
            let mut state = self.lock();
            let extension = state.extensions.remove(ext);
            let statuses: Vec<_> = state
                .terminals
                .values()
                .map(|known| known.status.clone())
                .collect();
            (statuses, extension)
        };
        drop(extension);
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
}
