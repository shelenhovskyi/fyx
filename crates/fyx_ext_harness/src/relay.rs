//! A fake relay endpoint for the `fyx:ui` 0.4 `relay` (design.md B.9), with the checks of
//! the host's (`ui_ext_wasm`'s relay requests, W1's `fyx_relay`): the channel is one of the
//! manifest's `[[fyx_ui.relay_channels]]`, the extension declares `relay:hooks`, stdin and argv
//! are at most [`MAX_RELAY_INPUT_BYTES`], at most [`MAX_RELAY_IN_FLIGHT`] requests wait, and each
//! request is answered at most once with at most [`MAX_RELAY_ANSWER_BYTES`].
//!
//! A test sends a hook as the relay would ([`crate::Harness::relay_request`]);
//! [`crate::Harness::deliver_io`] delivers `relay-request`, and
//! [`crate::Harness::relay_answer`] reads the extension's `relay.respond`. A request the host
//! refuses is an `Err` here: the real relay prints the channel's `on_failure` then.

use std::collections::{BTreeMap, VecDeque};

use ui_ext::integration::{
    CAPABILITY_RELAY_HOOKS, MAX_RELAY_ANSWER_BYTES, MAX_RELAY_IN_FLIGHT, MAX_RELAY_INPUT_BYTES,
    RelayAnswer, RelayChannelSpec, RelayRequest,
};
use ui_ext::terminal::TerminalHandle;
use ui_ext::v0_2::{HostError, HostErrorCode};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

#[derive(Debug)]
pub(crate) struct FakeRelay {
    next_id: u64,
    channels: Vec<RelayChannelSpec>,
    /// Delivered or queued, not answered.
    waiting: BTreeMap<u64, String>,
    answers: BTreeMap<u64, RelayAnswer>,
    pub(crate) events: VecDeque<RelayRequest>,
}

impl Default for FakeRelay {
    fn default() -> Self {
        Self {
            next_id: 1,
            channels: Vec::new(),
            waiting: BTreeMap::new(),
            answers: BTreeMap::new(),
            events: VecDeque::new(),
        }
    }
}

impl FakeRelay {
    pub(crate) fn set_channels(&mut self, channels: Vec<RelayChannelSpec>) {
        self.channels = channels;
    }

    pub(crate) fn channels(&self) -> &[RelayChannelSpec] {
        &self.channels
    }

    /// `channel` as the hook passes it, `<extension-id>/<name>` or the bare name; the extension
    /// gets the bare name, as from the host.
    pub(crate) fn request(
        &mut self,
        capabilities: &[String],
        extension_id: Option<&str>,
        channel: &str,
        terminal: Option<TerminalHandle>,
        argv: Vec<String>,
        stdin: String,
    ) -> Result<u64, HostError> {
        let name = match channel.split_once('/') {
            Some((extension, name)) => {
                if extension_id.is_some_and(|id| id != extension) {
                    return Err(error(
                        HostErrorCode::InvalidArgument,
                        format!("channel {channel:?} is not this extension's"),
                    ));
                }
                name
            }
            None => channel,
        };
        if !self.channels.iter().any(|spec| spec.name == name) {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("no [[fyx_ui.relay_channels]] named {name:?}"),
            ));
        }
        if !capabilities
            .iter()
            .any(|name| name == CAPABILITY_RELAY_HOOKS)
        {
            return Err(error(
                HostErrorCode::NotPermitted,
                format!("{CAPABILITY_RELAY_HOOKS:?} is not in the manifest"),
            ));
        }
        let input = stdin.len() + argv.iter().map(String::len).sum::<usize>();
        if input > MAX_RELAY_INPUT_BYTES {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("relay input of {input} bytes, at most {MAX_RELAY_INPUT_BYTES}"),
            ));
        }
        if self.waiting.len() >= MAX_RELAY_IN_FLIGHT {
            return Err(error(
                HostErrorCode::Unavailable,
                format!("{MAX_RELAY_IN_FLIGHT} relay requests are already waiting"),
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.waiting.insert(id, name.to_owned());
        self.events.push_back(RelayRequest {
            id,
            channel: name.to_owned(),
            terminal,
            argv,
            stdin,
        });
        Ok(id)
    }

    pub(crate) fn respond(&mut self, id: u64, answer: RelayAnswer) -> Result<(), HostError> {
        let size = answer.stdout.len() + answer.stderr.len();
        if size > MAX_RELAY_ANSWER_BYTES {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("relay answer of {size} bytes, at most {MAX_RELAY_ANSWER_BYTES}"),
            ));
        }
        if self.waiting.remove(&id).is_none() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{id} is not a waiting relay request"),
            ));
        }
        self.answers.insert(id, answer);
        Ok(())
    }

    pub(crate) fn take_answer(&mut self, id: u64) -> Option<RelayAnswer> {
        self.answers.remove(&id)
    }

    /// The requests not answered yet, by id.
    pub(crate) fn waiting(&self) -> Vec<u64> {
        self.waiting.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::integration::OnFailure;

    fn relay() -> FakeRelay {
        let mut relay = FakeRelay::default();
        relay.set_channels(vec![RelayChannelSpec {
            name: "stop".into(),
            timeout_ms: 5000,
            on_failure: OnFailure::default(),
            reason: "ends turns".into(),
        }]);
        relay
    }

    #[test]
    fn requests_are_checked_and_answered_once() {
        let granted = vec![CAPABILITY_RELAY_HOOKS.to_owned()];
        let mut relay = relay();
        let id = relay
            .request(
                &granted,
                Some("ext"),
                "ext/stop",
                Some(TerminalHandle(2)),
                Vec::new(),
                "{}".into(),
            )
            .unwrap();
        let event = relay.events.pop_front().unwrap();
        assert_eq!(event.channel, "stop");
        assert_eq!(event.terminal, Some(TerminalHandle(2)));
        let none = |code| move |error: HostError| assert_eq!(error.code, code);
        relay
            .request(
                &granted,
                Some("ext"),
                "other/stop",
                None,
                Vec::new(),
                String::new(),
            )
            .map(drop)
            .unwrap_or_else(none(HostErrorCode::InvalidArgument));
        relay
            .request(&granted, None, "start", None, Vec::new(), String::new())
            .map(drop)
            .unwrap_or_else(none(HostErrorCode::InvalidArgument));
        relay
            .request(&[], None, "stop", None, Vec::new(), String::new())
            .map(drop)
            .unwrap_or_else(none(HostErrorCode::NotPermitted));
        assert!(
            relay
                .request(
                    &granted,
                    None,
                    "stop",
                    None,
                    Vec::new(),
                    "x".repeat(MAX_RELAY_INPUT_BYTES + 1)
                )
                .is_err()
        );
        assert_eq!(relay.waiting(), [id]);
        relay.respond(id, RelayAnswer::default()).unwrap();
        assert!(relay.respond(id, RelayAnswer::default()).is_err());
        assert_eq!(relay.take_answer(id), Some(RelayAnswer::default()));
        assert_eq!(relay.take_answer(id), None);
        for _ in 0..MAX_RELAY_IN_FLIGHT {
            relay
                .request(&granted, None, "stop", None, Vec::new(), String::new())
                .unwrap();
        }
        let full = relay
            .request(&granted, None, "stop", None, Vec::new(), String::new())
            .unwrap_err();
        assert_eq!(full.code, HostErrorCode::Unavailable);
    }
}
