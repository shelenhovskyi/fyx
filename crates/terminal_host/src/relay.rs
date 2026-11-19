//! The host side of `fyx-relay` (plan 5.12, design §B.9, `relay:hooks`): the loopback endpoint,
//! the table of granted channels, the [`fyx_relay::State`] file the relay reads while Fyx is
//! closed, and the env every local Fyx terminal gets (`FYX_RELAY`, `FYX_RELAY_PORT`,
//! `FYX_RELAY_STATE`).
//!
//! A request is delivered only when its `FYX_TERMINAL_TOKEN` names a live Fyx terminal and the
//! channel is in the table; the runtime then hands it to the extension ([`take_relay_requests`]).
//! Everything else, and an extension that does not answer within the channel's timeout, ends with
//! the channel's `on_failure` (the endpoint's rules).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use futures::channel::mpsc as async_mpsc;
use fyx_relay::{Answer, ChannelInfo, Endpoint, Incoming, State};
use gpui::{App, Global};
use terminal_ext::TerminalId;
use ui_ext::integration::RelayChannelSpec;

/// A relay request for an extension.
#[derive(Debug)]
pub struct RelayDelivery {
    pub extension_id: String,
    /// The channel's name, without the extension id.
    pub channel: String,
    /// The Fyx terminal the hook runs in.
    pub terminal: TerminalId,
    pub argv: Vec<String>,
    pub stdin: String,
    pub reply: RelayReply,
}

/// Answers one [`RelayDelivery`]. Dropping it unanswered ends the request with `on_failure`.
#[derive(Debug)]
pub struct RelayReply(mpsc::Sender<Answer>);

impl RelayReply {
    pub fn send(self, exit: u8, stdout: String, stderr: String) {
        self.0
            .send(Answer {
                exit,
                stdout,
                stderr,
            })
            .ok();
    }
}

#[derive(Default)]
struct Table {
    /// This run's granted channels, by extension.
    channels: BTreeMap<String, BTreeMap<String, ChannelInfo>>,
    /// The extensions this run has set, with or without channels.
    seen: BTreeSet<String>,
    /// The state file as this run found it: kept for extensions not loaded yet.
    at_start: State,
}

impl Table {
    fn state(&self) -> State {
        let mut on_failure: BTreeMap<String, u8> = self
            .at_start
            .on_failure
            .iter()
            .filter(|(channel, _)| {
                fyx_relay::split_channel(channel)
                    .is_some_and(|(extension, _)| !self.seen.contains(extension))
            })
            .map(|(channel, exit)| (channel.clone(), *exit))
            .collect();
        for (extension, channels) in &self.channels {
            for (name, info) in channels {
                if info.on_failure != 0 {
                    on_failure.insert(format!("{extension}/{name}"), info.on_failure);
                }
            }
        }
        State { on_failure }
    }
}

struct Router {
    table: Mutex<Table>,
    requests: async_mpsc::UnboundedSender<RelayDelivery>,
    terminal_for_token: fn(&str) -> Option<TerminalId>,
}

impl Router {
    fn table(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl fyx_relay::Router for Router {
    fn channel(&self, extension_id: &str, channel: &str) -> Option<ChannelInfo> {
        let table = self.table();
        if let Some(channels) = table.channels.get(extension_id) {
            return channels.get(channel).copied();
        }
        // Not loaded yet this run: a channel that fails closed stays closed.
        if table.seen.contains(extension_id) {
            return None;
        }
        table
            .at_start
            .on_failure
            .get(&format!("{extension_id}/{channel}"))
            .map(|exit| ChannelInfo {
                timeout: Duration::ZERO,
                on_failure: *exit,
            })
    }

    fn deliver(&self, incoming: Incoming, reply: mpsc::Sender<Answer>) -> bool {
        let Some(terminal) = (self.terminal_for_token)(&incoming.token) else {
            return false;
        };
        let granted = self
            .table()
            .channels
            .get(&incoming.extension_id)
            .is_some_and(|channels| channels.contains_key(&incoming.channel));
        granted
            && self
                .requests
                .unbounded_send(RelayDelivery {
                    extension_id: incoming.extension_id,
                    channel: incoming.channel,
                    terminal,
                    argv: incoming.argv,
                    stdin: incoming.stdin,
                    reply: RelayReply(reply),
                })
                .is_ok()
    }
}

struct Relay {
    router: Arc<Router>,
    endpoint: Endpoint,
    binary: Option<PathBuf>,
    state: PathBuf,
    /// Saves are queued here so they reach the file in order.
    saves: Mutex<mpsc::Sender<State>>,
}

impl Relay {
    fn start(
        state: PathBuf,
        binary: Option<PathBuf>,
        terminal_for_token: fn(&str) -> Option<TerminalId>,
    ) -> std::io::Result<(Self, async_mpsc::UnboundedReceiver<RelayDelivery>)> {
        let (requests, received) = async_mpsc::unbounded();
        let router = Arc::new(Router {
            table: Mutex::new(Table {
                at_start: State::load(&state),
                ..Table::default()
            }),
            requests,
            terminal_for_token,
        });
        let endpoint = Endpoint::start(router.clone())?;
        let (saves, queued) = mpsc::channel::<State>();
        let path = state.clone();
        std::thread::Builder::new()
            .name("fyx-relay-state".into())
            .spawn(move || {
                for state in queued {
                    if let Err(error) = state.save(&path) {
                        log::warn!("fyx-relay: could not save {}: {error}", path.display());
                    }
                }
            })?;
        let relay = Self {
            router,
            endpoint,
            binary,
            state,
            saves: Mutex::new(saves),
        };
        Ok((relay, received))
    }

    fn set_channels(&self, extension_id: &str, channels: &[RelayChannelSpec]) {
        let state = {
            let mut table = self.router.table();
            table.seen.insert(extension_id.to_owned());
            if channels.is_empty() {
                table.channels.remove(extension_id);
            } else {
                table.channels.insert(
                    extension_id.to_owned(),
                    channels
                        .iter()
                        .map(|spec| {
                            (
                                spec.name.clone(),
                                ChannelInfo {
                                    timeout: Duration::from_millis(spec.timeout_ms.into()),
                                    on_failure: spec.on_failure.exit,
                                },
                            )
                        })
                        .collect(),
                );
            }
            table.state()
        };
        if let Ok(saves) = self.saves.lock() {
            saves.send(state).ok();
        }
    }

    fn env(&self) -> Vec<(String, String)> {
        vec![
            (
                fyx_relay::RELAY_ENV.to_owned(),
                self.binary
                    .as_ref()
                    .map(|binary| binary.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            (
                fyx_relay::PORT_ENV.to_owned(),
                self.endpoint.port().to_string(),
            ),
            (
                fyx_relay::STATE_ENV.to_owned(),
                self.state.to_string_lossy().into_owned(),
            ),
        ]
    }
}

struct GlobalRelay {
    relay: Relay,
    requests: Option<async_mpsc::UnboundedReceiver<RelayDelivery>>,
}

impl Global for GlobalRelay {}

/// `fyx-relay` next to this executable, if it is there.
fn find_binary() -> Option<PathBuf> {
    let binary = std::env::current_exe()
        .ok()?
        .parent()?
        .join(fyx_relay::BINARY_NAME);
    binary.is_file().then_some(binary)
}

/// Starts the endpoint, with the state file in `data_dir`. Called once by `main.rs`.
pub fn init_relay(data_dir: &Path, cx: &mut App) {
    let binary = find_binary();
    if binary.is_none() {
        log::info!(
            "fyx-relay: no {} next to Fyx; relay hooks fail open",
            fyx_relay::BINARY_NAME
        );
    }
    match Relay::start(
        fyx_relay::state_path(data_dir),
        binary,
        terminal_ext::terminal_for_token,
    ) {
        Ok((relay, requests)) => cx.set_global(GlobalRelay {
            relay,
            requests: Some(requests),
        }),
        Err(error) => log::warn!("fyx-relay: the endpoint did not start: {error}"),
    }
}

/// The relay requests for extensions; the runtime takes them once.
pub fn take_relay_requests(cx: &mut App) -> Option<async_mpsc::UnboundedReceiver<RelayDelivery>> {
    cx.try_global::<GlobalRelay>()?;
    cx.global_mut::<GlobalRelay>().requests.take()
}

/// Replaces `extension_id`'s granted channels (`relay:hooks`); an empty list removes them, on
/// unload, disable, uninstall and revocation.
pub fn set_relay_channels(extension_id: &str, channels: &[RelayChannelSpec], cx: &App) {
    if let Some(relay) = cx.try_global::<GlobalRelay>() {
        relay.relay.set_channels(extension_id, channels);
    }
}

/// The relay binary, for `${relay}`.
pub fn relay_binary(cx: &App) -> Option<PathBuf> {
    cx.try_global::<GlobalRelay>()?.relay.binary.clone()
}

/// For local terminals: where the relay and its endpoint are. Empty values without an endpoint,
/// so a Fyx started from a Fyx terminal never passes on its parent's.
pub(crate) fn relay_env(cx: &App) -> Vec<(String, String)> {
    match cx.try_global::<GlobalRelay>() {
        Some(relay) => relay.relay.env(),
        None => [
            fyx_relay::RELAY_ENV,
            fyx_relay::PORT_ENV,
            fyx_relay::STATE_ENV,
        ]
        .into_iter()
        .map(|name| (name.to_owned(), String::new()))
        .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;
    use ui_ext::integration::OnFailure;

    const TOKEN: &str = "live-terminal";

    fn terminal_for_token(token: &str) -> Option<TerminalId> {
        (token == TOKEN).then(|| terminal_ext::TerminalExt::new().id())
    }

    fn spec(name: &str, exit: u8) -> RelayChannelSpec {
        RelayChannelSpec {
            name: name.into(),
            timeout_ms: 2_000,
            on_failure: OnFailure { exit },
            reason: "r".into(),
        }
    }

    fn env(relay: &Relay, token: &str) -> fyx_relay::Env {
        fyx_relay::Env {
            port: Some(relay.endpoint.port().to_string()),
            token: Some(token.into()),
            state: Some(relay.state.clone()),
        }
    }

    fn call(relay: &Relay, channel: &str, token: &str) -> Answer {
        fyx_relay::run(
            ["--channel".to_owned(), channel.to_owned(), "x".to_owned()],
            &b"in"[..],
            &env(relay, token),
        )
    }

    /// Waits for the state file to hold `expected` (saves run on their own thread).
    fn wait_for_state(path: &Path, expected: &State) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while State::load(path) != *expected {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                State::load(path)
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn granted_channels_reach_the_runtime_and_the_rest_fail_as_declared() {
        let root = tempfile::tempdir().unwrap();
        let state = fyx_relay::state_path(root.path());
        State {
            on_failure: BTreeMap::from([("later/guard".to_owned(), 2), ("ext/old".to_owned(), 4)]),
        }
        .save(&state)
        .unwrap();
        let (relay, mut requests) = Relay::start(state.clone(), None, terminal_for_token).unwrap();
        relay.set_channels("ext", &[spec("pre-tool", 0), spec("guard", 3)]);
        wait_for_state(
            &state,
            &State {
                on_failure: BTreeMap::from([
                    ("ext/guard".to_owned(), 3),
                    ("later/guard".to_owned(), 2),
                ]),
            },
        );

        let answering = std::thread::spawn(move || {
            let delivery = futures::executor::block_on(requests.next()).unwrap();
            assert_eq!(
                (
                    delivery.extension_id.as_str(),
                    delivery.channel.as_str(),
                    delivery.argv.as_slice(),
                    delivery.stdin.as_str()
                ),
                ("ext", "pre-tool", &["x".to_owned()][..], "in")
            );
            delivery.reply.send(5, "out".into(), String::new());
            requests
        });
        assert_eq!(
            call(&relay, "ext/pre-tool", TOKEN),
            Answer {
                exit: 5,
                stdout: "out".into(),
                stderr: String::new(),
            }
        );
        let mut requests = answering.join().unwrap();

        // No live terminal, an unknown channel, an extension not loaded yet that fails closed.
        assert_eq!(call(&relay, "ext/guard", "dead"), Answer::failure(3));
        assert_eq!(call(&relay, "ext/nope", TOKEN), Answer::failure(0));
        assert_eq!(call(&relay, "later/guard", TOKEN), Answer::failure(2));
        assert_eq!(
            call(&relay, "ext/old", TOKEN),
            Answer::failure(0),
            "ext has loaded without it"
        );
        assert!(requests.try_recv().is_err(), "nothing else was delivered");

        // Unloaded: its channels are gone, and so is its line in the file.
        relay.set_channels("ext", &[]);
        assert_eq!(call(&relay, "ext/guard", TOKEN), Answer::failure(0));
        wait_for_state(
            &state,
            &State {
                on_failure: BTreeMap::from([("later/guard".to_owned(), 2)]),
            },
        );
    }

    #[test]
    fn an_unanswered_request_ends_with_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let (relay, requests) =
            Relay::start(fyx_relay::state_path(root.path()), None, terminal_for_token).unwrap();
        relay.set_channels("ext", &[spec("guard", 2)]);
        let dropping = std::thread::spawn(move || {
            let mut requests = requests;
            let delivery = futures::executor::block_on(requests.next()).unwrap();
            drop(delivery.reply);
        });
        assert_eq!(call(&relay, "ext/guard", TOKEN), Answer::failure(2));
        dropping.join().unwrap();
        let env = relay.env();
        assert_eq!(env[0], (fyx_relay::RELAY_ENV.to_owned(), String::new()));
        assert_eq!(env[1].1, relay.endpoint.port().to_string());
    }
}
