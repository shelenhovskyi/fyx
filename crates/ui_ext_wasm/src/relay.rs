//! The fyx:ui 0.4 `relay` import and `io-event::relay-request` (design wit-0.4 §B.9, plan 5.12):
//! `relay:hooks`. The relay endpoint (W1's `fyx_relay`) maps a hook's terminal token to the
//! extension and hands the request to [`crate::UiExtension::relay_request`]; this side checks the
//! channel, the grant, the sizes and the in-flight limit, assigns the id, delivers the request
//! and calls the endpoint's [`RelayReply`] exactly once with the extension's `relay.respond`.
//! A reply dropped without a call (refused, expired, revoked, or the instance gone) makes the
//! endpoint answer the channel's `on_failure`.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use futures::{StreamExt as _, channel::mpsc};
use ui_ext::{
    integration::{
        CAPABILITY_RELAY_HOOKS, MAX_RELAY_ANSWER_BYTES, MAX_RELAY_IN_FLIGHT, MAX_RELAY_INPUT_BYTES,
        RelayAnswer, RelayChannelSpec, RelayReply, RelayRequest,
    },
    terminal::TerminalHandle,
    v0_2::{HostError, HostErrorCode},
};

use crate::{
    network::{IoEvent, Running},
    permissions::{CapabilityCheck, PermissionState},
};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

/// A delivered request the extension has not answered yet.
struct Pending {
    reply: RelayReply,
    /// After the channel's `timeout_ms` the endpoint has answered `on_failure`: the entry no
    /// longer counts and a late `respond` is an unknown id.
    deadline: Instant,
}

#[derive(Default)]
struct PendingSet {
    next_id: u64,
    by_id: HashMap<u64, Pending>,
}

impl PendingSet {
    /// Drops the expired entries (their replies with them).
    fn expire(&mut self, now: Instant) {
        self.by_id.retain(|_, pending| pending.deadline > now);
    }
}

/// One instance's relay channels and unanswered requests.
pub(crate) struct RelayHandle {
    extension_id: Arc<str>,
    /// `[[fyx_ui.relay_channels]]`; empty unless the component is 0.4 and declares
    /// `relay:hooks`.
    channels: Vec<RelayChannelSpec>,
    broker: Option<Arc<dyn CapabilityCheck>>,
    pending: Mutex<PendingSet>,
    sender: mpsc::UnboundedSender<RelayRequest>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<RelayRequest>>>,
}

impl RelayHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        channels: Vec<RelayChannelSpec>,
        broker: Option<Arc<dyn CapabilityCheck>>,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded();
        Self {
            extension_id,
            channels,
            broker,
            pending: Mutex::default(),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, PendingSet> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The declared channels (empty without `relay:hooks`), granted or not: the endpoint answers
    /// a refused request with the channel's `on_failure`.
    pub(crate) fn channels(&self) -> &[RelayChannelSpec] {
        &self.channels
    }

    /// Whether `relay:hooks` is granted now (and declared, with channels).
    pub(crate) fn granted(&self) -> bool {
        !self.channels.is_empty()
            && self.broker.as_ref().is_some_and(|broker| {
                broker.state(&self.extension_id, CAPABILITY_RELAY_HOOKS) == PermissionState::Granted
            })
    }

    /// Moves the requests onto the io channel, on the current tokio runtime. Once.
    pub(crate) fn forward(&self, running: &Arc<Running>) {
        let Some(mut receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let running = Arc::downgrade(running);
        tokio::spawn(async move {
            while let Some(request) = receiver.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::RelayRequest(request)).await;
            }
        });
    }

    /// A hook's request on `channel`. Refused (the reply dropped, so `on_failure`) when the
    /// channel is not declared, `relay:hooks` is not granted, the input is over 1 MiB or 8
    /// requests are unanswered; the reason is for the endpoint's log.
    pub(crate) fn request(
        &self,
        channel: &str,
        terminal: Option<TerminalHandle>,
        argv: Vec<String>,
        stdin: String,
        reply: RelayReply,
    ) -> Result<u64, String> {
        let Some(spec) = self.channels.iter().find(|spec| spec.name == channel) else {
            return Err(format!("no relay channel {channel:?} is declared"));
        };
        if !self.granted() {
            return Err(format!("{CAPABILITY_RELAY_HOOKS} is not granted"));
        }
        let input = stdin.len() + argv.iter().map(String::len).sum::<usize>();
        if input > MAX_RELAY_INPUT_BYTES {
            return Err(format!(
                "a relay request is at most {MAX_RELAY_INPUT_BYTES} bytes of stdin and argv"
            ));
        }
        let now = Instant::now();
        let id = {
            let mut pending = self.pending();
            pending.expire(now);
            if pending.by_id.len() >= MAX_RELAY_IN_FLIGHT {
                return Err(format!(
                    "{MAX_RELAY_IN_FLIGHT} relay requests are already unanswered"
                ));
            }
            pending.next_id += 1;
            let id = pending.next_id;
            pending.by_id.insert(
                id,
                Pending {
                    reply,
                    deadline: now + Duration::from_millis(spec.timeout_ms.into()),
                },
            );
            id
        };
        let request = RelayRequest {
            id,
            channel: channel.into(),
            terminal,
            argv,
            stdin,
        };
        if self.sender.unbounded_send(request).is_err() {
            self.pending().by_id.remove(&id);
            return Err("the extension is not running".into());
        }
        Ok(id)
    }

    /// `relay.respond`: calls the request's reply, once. An unknown, answered or expired id is
    /// `invalid-argument`; an answer over 1 MiB is `limit-exceeded` and the request stays open.
    pub(crate) fn respond(&self, id: u64, answer: RelayAnswer) -> Result<(), HostError> {
        if self.channels.is_empty() {
            return Err(error(
                HostErrorCode::NotPermitted,
                format!("respond needs {CAPABILITY_RELAY_HOOKS:?} and relay channels"),
            ));
        }
        if !self.granted() {
            return Err(error(
                HostErrorCode::Denied,
                format!("{CAPABILITY_RELAY_HOOKS} is not granted"),
            ));
        }
        if answer.stdout.len() + answer.stderr.len() > MAX_RELAY_ANSWER_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("a relay answer is at most {MAX_RELAY_ANSWER_BYTES} bytes"),
            ));
        }
        let pending = {
            let mut pending = self.pending();
            pending.expire(Instant::now());
            pending.by_id.remove(&id)
        };
        let Some(pending) = pending else {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("relay request {id} is unknown, answered or timed out"),
            ));
        };
        (pending.reply)(answer);
        Ok(())
    }

    /// `relay:hooks` was revoked: the unanswered requests fail open.
    pub(crate) fn revoke_all(&self) {
        let dropped = std::mem::take(&mut self.pending().by_id);
        drop(dropped);
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.pending().by_id.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ui_ext::integration::OnFailure;

    use super::*;
    use crate::permissions::CapabilityDenied;

    struct Broker(Mutex<PermissionState>);

    impl CapabilityCheck for Broker {
        fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
            Ok(())
        }
        fn state(&self, _: &str, capability: &str) -> PermissionState {
            assert_eq!(capability, CAPABILITY_RELAY_HOOKS);
            *self.0.lock().unwrap()
        }
    }

    fn channel(name: &str, timeout_ms: u32) -> RelayChannelSpec {
        RelayChannelSpec {
            name: name.into(),
            timeout_ms,
            on_failure: OnFailure::default(),
            reason: "hooks".into(),
        }
    }

    /// The handle, its broker, and the answers and drops of the replies it was given.
    fn handle(
        channels: Vec<RelayChannelSpec>,
    ) -> (
        RelayHandle,
        Arc<Broker>,
        Arc<Mutex<Vec<RelayAnswer>>>,
        Arc<AtomicUsize>,
    ) {
        let broker = Arc::new(Broker(Mutex::new(PermissionState::Granted)));
        let handle = RelayHandle::new("ext".into(), channels, Some(broker.clone()));
        (handle, broker, Arc::default(), Arc::default())
    }

    struct CountDrop(Arc<AtomicUsize>);

    impl Drop for CountDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn reply(answers: &Arc<Mutex<Vec<RelayAnswer>>>, drops: &Arc<AtomicUsize>) -> RelayReply {
        let answers = answers.clone();
        let guard = CountDrop(drops.clone());
        Box::new(move |answer| {
            let _guard = guard;
            answers.lock().unwrap().push(answer);
        })
    }

    fn answer(exit: u8, stdout: &str) -> RelayAnswer {
        RelayAnswer {
            exit,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[test]
    fn a_request_is_answered_once() {
        let (relay, _, answers, drops) = handle(vec![channel("pre-tool", 30_000)]);
        let id = relay
            .request(
                "pre-tool",
                Some(TerminalHandle(3)),
                vec!["--x".into()],
                "{}".into(),
                reply(&answers, &drops),
            )
            .unwrap();
        let mut receiver = relay.receiver.lock().unwrap().take().unwrap();
        let request = receiver.try_recv().unwrap();
        assert_eq!(
            request,
            RelayRequest {
                id,
                channel: "pre-tool".into(),
                terminal: Some(TerminalHandle(3)),
                argv: vec!["--x".into()],
                stdin: "{}".into(),
            }
        );
        relay.respond(id, answer(2, "no")).unwrap();
        assert_eq!(*answers.lock().unwrap(), vec![answer(2, "no")]);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let again = relay.respond(id, answer(0, "")).unwrap_err();
        assert_eq!(again.code, HostErrorCode::InvalidArgument);
        assert_eq!(answers.lock().unwrap().len(), 1);
        assert_eq!(
            relay.respond(99, answer(0, "")).unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
    }

    #[test]
    fn refusals_drop_the_reply() {
        let (relay, broker, answers, drops) = handle(vec![channel("pre-tool", 30_000)]);
        let refused = |relay: &RelayHandle, channel: &str, stdin: String| {
            relay
                .request(channel, None, Vec::new(), stdin, reply(&answers, &drops))
                .unwrap_err()
        };
        assert!(refused(&relay, "post-tool", String::new()).contains("no relay channel"));
        assert!(
            refused(&relay, "pre-tool", "x".repeat(MAX_RELAY_INPUT_BYTES + 1)).contains("1048576")
        );
        *broker.0.lock().unwrap() = PermissionState::Ask;
        assert!(refused(&relay, "pre-tool", String::new()).contains("not granted"));
        assert_eq!(drops.load(Ordering::SeqCst), 3);
        assert!(answers.lock().unwrap().is_empty());

        // Without `relay:hooks` there are no channels, and `respond` is not permitted.
        let (bare, _, _, _) = handle(Vec::new());
        assert!(!bare.granted());
        assert_eq!(
            bare.respond(1, answer(0, "")).unwrap_err().code,
            HostErrorCode::NotPermitted
        );
        assert_eq!(
            relay.respond(1, answer(0, "")).unwrap_err().code,
            HostErrorCode::Denied
        );
    }

    #[test]
    fn limits_in_flight_answers_and_timeouts() {
        let (relay, _, answers, drops) =
            handle(vec![channel("pre-tool", 30_000), channel("quick", 0)]);
        let ids: Vec<u64> = (0..MAX_RELAY_IN_FLIGHT)
            .map(|_| {
                relay
                    .request(
                        "pre-tool",
                        None,
                        Vec::new(),
                        String::new(),
                        reply(&answers, &drops),
                    )
                    .unwrap()
            })
            .collect();
        let ninth = relay
            .request(
                "pre-tool",
                None,
                Vec::new(),
                String::new(),
                reply(&answers, &drops),
            )
            .unwrap_err();
        assert!(ninth.contains("already unanswered"), "{ninth}");
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        // Too long an answer leaves the request open for a shorter one.
        let long = RelayAnswer {
            exit: 0,
            stdout: "x".repeat(MAX_RELAY_ANSWER_BYTES / 2 + 1),
            stderr: "y".repeat(MAX_RELAY_ANSWER_BYTES / 2),
        };
        assert_eq!(
            relay.respond(ids[0], long).unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        relay.respond(ids[0], answer(0, "ok")).unwrap();
        assert_eq!(relay.in_flight(), MAX_RELAY_IN_FLIGHT - 1);

        // An expired request no longer counts, and its late answer is an unknown id.
        relay.revoke_all();
        assert_eq!(drops.load(Ordering::SeqCst), 1 + MAX_RELAY_IN_FLIGHT);
        let quick = relay
            .request(
                "quick",
                None,
                Vec::new(),
                String::new(),
                reply(&answers, &drops),
            )
            .unwrap();
        assert_eq!(
            relay.respond(quick, answer(0, "late")).unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
        assert_eq!(relay.in_flight(), 0);
        assert_eq!(*answers.lock().unwrap(), vec![answer(0, "ok")]);
    }

    #[test]
    fn dropping_the_instance_drops_the_replies() {
        let (relay, _, answers, drops) = handle(vec![channel("pre-tool", 30_000)]);
        for _ in 0..2 {
            relay
                .request(
                    "pre-tool",
                    None,
                    Vec::new(),
                    String::new(),
                    reply(&answers, &drops),
                )
                .unwrap();
        }
        drop(relay);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(answers.lock().unwrap().is_empty());
    }
}
