//! `fyx-relay` (design 0.4 §B.9, `relay:hooks`): most CLI tools' hooks can only run a command, so a
//! hook runs `"$FYX_RELAY" --channel <extension-id>/<name> [args…]`. The binary reads its argv and
//! at most 1 MiB of stdin and sends them to the running Fyx over a loopback endpoint
//! ([`Endpoint`], port in `FYX_RELAY_PORT`), authenticated by the terminal's `FYX_TERMINAL_TOKEN`.
//! The host routes the request to the extension ([`Router`]) and the relay prints the answer and
//! exits with its code.
//!
//! It fails open: no token, Fyx not running, an unknown channel, a revoked capability or a timeout
//! end with the channel's `on_failure` (default exit 0, no output). Fyx records the channels that
//! do not fail open in a [`State`] file, so the relay knows them while Fyx is closed.
//!
//! The wire format is one JSON [`Request`] from the relay, then (after the relay shuts down its
//! write half) one JSON [`Answer`] from the host. Std only: the binary must start fast and the
//! endpoint runs on its own threads.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal as _, Read, Write as _};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The relay binary's absolute path, in every Fyx terminal.
pub const RELAY_ENV: &str = "FYX_RELAY";
/// The endpoint's port, in every Fyx terminal.
pub const PORT_ENV: &str = "FYX_RELAY_PORT";
/// The [`State`] file, in every Fyx terminal (a custom `--user-data-dir` moves it).
pub const STATE_ENV: &str = "FYX_RELAY_STATE";
/// The terminal's token; `terminal_ext::TERMINAL_TOKEN_ENV` is the same name.
pub const TERMINAL_TOKEN_ENV: &str = "FYX_TERMINAL_TOKEN";
/// The binary's file name, next to the `fyx` executable.
pub const BINARY_NAME: &str = if cfg!(windows) {
    "fyx-relay.exe"
} else {
    "fyx-relay"
};

pub const MAX_STDIN_BYTES: usize = 1 << 20;
pub const MAX_ARGV_BYTES: usize = 1 << 20;
/// stdout and stderr of an answer together.
pub const MAX_ANSWER_BYTES: usize = 1 << 20;
/// Requests waiting for one extension's answer.
pub const MAX_IN_FLIGHT: usize = 8;
/// `timeout_ms` of a channel is at most this.
pub const MAX_TIMEOUT: Duration = Duration::from_millis(600_000);

const VERSION: u32 = 1;
/// A request is 1 MiB of stdin and 1 MiB of argv, escaped for JSON.
const MAX_REQUEST_BYTES: u64 = 16 << 20;
/// An answer is 1 MiB, escaped for JSON.
const MAX_ANSWER_WIRE_BYTES: u64 = 8 << 20;
const MAX_CONNECTIONS: usize = 32;
/// The kernel completes a loopback handshake without the app, so a running Fyx connects in well
/// under this; Windows takes about 2 s to refuse a closed loopback port, which would hold up every
/// hook while Fyx is closed.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// The host answers within the channel's timeout; this only stops a relay whose Fyx hangs.
const ANSWER_READ_TIMEOUT: Duration = Duration::from_secs(600 + 30);

/// What the relay sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub token: String,
    /// `<extension-id>/<name>`.
    pub channel: String,
    pub argv: Vec<String>,
    pub stdin: String,
}

/// What the relay prints and exits with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub exit: u8,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
}

impl Answer {
    /// A channel's `on_failure`: the exit code and no output.
    pub fn failure(exit: u8) -> Self {
        Self {
            exit,
            ..Self::default()
        }
    }

    /// stdout and stderr cut to [`MAX_ANSWER_BYTES`] together, at character boundaries.
    pub fn truncated(mut self) -> Self {
        truncate(&mut self.stdout, MAX_ANSWER_BYTES);
        truncate(&mut self.stderr, MAX_ANSWER_BYTES - self.stdout.len());
        self
    }
}

fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// `<extension-id>/<name>` split in two.
pub fn split_channel(channel: &str) -> Option<(&str, &str)> {
    let (extension, name) = channel.split_once('/')?;
    (!extension.is_empty() && !name.is_empty() && !name.contains('/')).then_some((extension, name))
}

// ---------------------------------------------------------------------------------------------
// The state file

/// Written by Fyx when extensions load, read by the relay only when Fyx does not answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// `<extension-id>/<name>` → exit code, for the channels that do not fail open.
    #[serde(default)]
    pub on_failure: BTreeMap<String, u8>,
}

impl State {
    /// The file's state, or the empty one (every channel fails open).
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes to a temporary name and renames, so a relay never reads half a file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&temporary, path)
    }

    pub fn exit_for(&self, channel: &str) -> u8 {
        self.on_failure.get(channel).copied().unwrap_or(0)
    }
}

/// The state file in `data_dir`.
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("extensions").join("fyx").join("relay.json")
}

/// The state file in Fyx's default data dir, as `paths::data_dir` finds it (kept in step with it;
/// the relay does not link `paths`).
pub fn default_state_path() -> Option<PathBuf> {
    let data_dir = if cfg!(target_os = "macos") {
        dirs::home_dir()?.join("Library/Application Support/Fyx")
    } else if cfg!(target_os = "windows") {
        dirs::data_local_dir()?.join("Fyx")
    } else {
        match std::env::var_os("FLATPAK_XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => dirs::data_local_dir()?,
        }
        .join("fyx")
    };
    Some(state_path(&data_dir))
}

// ---------------------------------------------------------------------------------------------
// The relay

/// What the relay takes from its environment.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub port: Option<String>,
    pub token: Option<String>,
    pub state: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Self {
        Self {
            port: std::env::var(PORT_ENV).ok(),
            token: std::env::var(TERMINAL_TOKEN_ENV).ok(),
            state: std::env::var_os(STATE_ENV)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .or_else(default_state_path),
        }
    }
}

/// The binary: the process's env, and stdin unless it is a terminal (a hook always pipes it).
pub fn run_from_env(args: impl IntoIterator<Item = String>, stdin: io::StdinLock<'_>) -> Answer {
    let env = Env::from_process();
    if stdin.is_terminal() {
        run(args, io::empty(), &env)
    } else {
        run(args, stdin, &env)
    }
}

/// `--channel <extension-id>/<name>` (or `--channel=…`) first, then the hook's own arguments.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Option<(String, Vec<String>)> {
    let mut args = args.into_iter();
    let first = args.next()?;
    let channel = match first.strip_prefix("--channel=") {
        Some(channel) => channel.to_owned(),
        None if first == "--channel" => args.next()?,
        None => return None,
    };
    split_channel(&channel)?;
    Some((channel, args.collect()))
}

/// Sends the request and returns what to print and exit with.
pub fn run(args: impl IntoIterator<Item = String>, stdin: impl Read, env: &Env) -> Answer {
    let Some((channel, argv)) = parse_args(args) else {
        return Answer {
            exit: 0,
            stdout: String::new(),
            stderr: "usage: fyx-relay --channel <extension-id>/<name> [args...]\n".into(),
        };
    };
    let failure = || {
        Answer::failure(
            env.state
                .as_deref()
                .map(State::load)
                .unwrap_or_default()
                .exit_for(&channel),
        )
    };
    let (Some(port), Some(token)) = (
        env.port
            .as_deref()
            .and_then(|port| port.parse::<u16>().ok()),
        env.token.clone().filter(|token| !token.is_empty()),
    ) else {
        return failure();
    };
    if argv.iter().map(String::len).sum::<usize>() > MAX_ARGV_BYTES {
        return failure();
    }
    let mut input = Vec::new();
    if stdin
        .take(MAX_STDIN_BYTES as u64 + 1)
        .read_to_end(&mut input)
        .is_err()
        || input.len() > MAX_STDIN_BYTES
    {
        return failure();
    }
    let request = Request {
        version: VERSION,
        token,
        channel: channel.clone(),
        argv,
        stdin: String::from_utf8_lossy(&input).into_owned(),
    };
    exchange(port, &request).unwrap_or_else(|_| failure())
}

fn exchange(port: u16, request: &Request) -> io::Result<Answer> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(ANSWER_READ_TIMEOUT))?;
    stream.write_all(&serde_json::to_vec(request)?)?;
    stream.shutdown(Shutdown::Write)?;
    let mut bytes = Vec::new();
    (&stream)
        .take(MAX_ANSWER_WIRE_BYTES)
        .read_to_end(&mut bytes)?;
    let answer: Answer = serde_json::from_slice(&bytes)?;
    Ok(answer.truncated())
}

// ---------------------------------------------------------------------------------------------
// The endpoint

/// A declared channel of an extension that has `relay:hooks`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelInfo {
    pub timeout: Duration,
    pub on_failure: u8,
}

/// A request for the extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incoming {
    /// The caller's `FYX_TERMINAL_TOKEN`, which the host maps to its terminal.
    pub token: String,
    pub extension_id: String,
    /// The channel's name, without the extension id.
    pub channel: String,
    pub argv: Vec<String>,
    pub stdin: String,
}

/// The host's side of the endpoint. Called on the endpoint's threads.
pub trait Router: Send + Sync + 'static {
    /// The channel, when the extension declares it and has `relay:hooks`.
    fn channel(&self, extension_id: &str, channel: &str) -> Option<ChannelInfo>;

    /// Hands the request to the extension, which answers on `reply`. `false` when the token
    /// names no Fyx terminal or the extension cannot take it now.
    fn deliver(&self, incoming: Incoming, reply: mpsc::Sender<Answer>) -> bool;
}

/// The loopback endpoint on `127.0.0.1:<port>`. Dropping it stops accepting.
pub struct Endpoint {
    port: u16,
    stop: Arc<AtomicBool>,
}

struct Shared {
    router: Arc<dyn Router>,
    in_flight: Mutex<BTreeMap<String, usize>>,
    connections: AtomicUsize,
}

impl Endpoint {
    pub fn start(router: Arc<dyn Router>) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            router,
            in_flight: Mutex::new(BTreeMap::new()),
            connections: AtomicUsize::new(0),
        });
        let stopped = stop.clone();
        std::thread::Builder::new()
            .name("fyx-relay-endpoint".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    if shared.connections.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
                        shared.connections.fetch_sub(1, Ordering::AcqRel);
                        continue;
                    }
                    let spawned = std::thread::Builder::new()
                        .name("fyx-relay-request".into())
                        .spawn({
                            let shared = shared.clone();
                            move || {
                                shared.serve(stream);
                                shared.connections.fetch_sub(1, Ordering::AcqRel);
                            }
                        });
                    if spawned.is_err() {
                        shared.connections.fetch_sub(1, Ordering::AcqRel);
                    }
                }
            })?;
        Ok(Self { port, stop })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wakes the accept loop so it sees `stop`.
        TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)),
            CONNECT_TIMEOUT,
        )
        .ok();
    }
}

/// Counts one request of an extension while it waits.
struct InFlight<'a> {
    shared: &'a Shared,
    extension_id: String,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut in_flight = self
            .shared
            .in_flight
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(count) = in_flight.get_mut(&self.extension_id) {
            *count -= 1;
            if *count == 0 {
                in_flight.remove(&self.extension_id);
            }
        }
    }
}

impl Shared {
    fn serve(&self, stream: TcpStream) {
        if stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if (&stream)
            .take(MAX_REQUEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() as u64 > MAX_REQUEST_BYTES
        {
            return;
        }
        // Anything else (a browser's HTTP request, a port scan) is closed without an answer.
        let Ok(request) = serde_json::from_slice::<Request>(&bytes) else {
            return;
        };
        let answer = self.answer(request);
        let mut stream = stream;
        if let Ok(bytes) = serde_json::to_vec(&answer) {
            stream.write_all(&bytes).ok();
        }
        stream.shutdown(Shutdown::Both).ok();
    }

    fn answer(&self, request: Request) -> Answer {
        let Some((extension_id, channel)) = split_channel(&request.channel) else {
            return Answer::failure(0);
        };
        let Some(info) = self.router.channel(extension_id, channel) else {
            return Answer::failure(0);
        };
        let failure = Answer::failure(info.on_failure);
        if request.version != VERSION
            || request.stdin.len() > MAX_STDIN_BYTES
            || request.argv.iter().map(String::len).sum::<usize>() > MAX_ARGV_BYTES
        {
            return failure;
        }
        let _in_flight = {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let count = in_flight.entry(extension_id.to_owned()).or_default();
            if *count >= MAX_IN_FLIGHT {
                log::warn!("fyx-relay: {extension_id} has {MAX_IN_FLIGHT} requests in flight");
                return failure;
            }
            *count += 1;
            InFlight {
                shared: self,
                extension_id: extension_id.to_owned(),
            }
        };
        let (reply, answer) = mpsc::channel();
        let incoming = Incoming {
            token: request.token,
            extension_id: extension_id.to_owned(),
            channel: channel.to_owned(),
            argv: request.argv,
            stdin: request.stdin,
        };
        if !self.router.deliver(incoming, reply) {
            return failure;
        }
        match answer.recv_timeout(info.timeout.min(MAX_TIMEOUT)) {
            Ok(answer) => answer.truncated(),
            Err(_) => failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const TOKEN: &str = "terminal-token";

    /// `ext/echo` answers at once, `ext/slow` never (exit 2 on failure), `ext/closed` fails
    /// closed with exit 3 and has no terminal.
    #[derive(Default)]
    struct Fake {
        seen: Mutex<Vec<Incoming>>,
        held: Mutex<Vec<mpsc::Sender<Answer>>>,
    }

    impl Router for Fake {
        fn channel(&self, extension_id: &str, channel: &str) -> Option<ChannelInfo> {
            let timeout = Duration::from_secs(5);
            match (extension_id, channel) {
                ("ext", "echo") => Some(ChannelInfo {
                    timeout,
                    on_failure: 0,
                }),
                ("ext", "slow") => Some(ChannelInfo {
                    timeout: Duration::from_millis(300),
                    on_failure: 2,
                }),
                ("ext", "closed") => Some(ChannelInfo {
                    timeout,
                    on_failure: 3,
                }),
                _ => None,
            }
        }

        fn deliver(&self, incoming: Incoming, reply: mpsc::Sender<Answer>) -> bool {
            if incoming.token != TOKEN || incoming.channel == "closed" {
                return false;
            }
            self.seen.lock().unwrap().push(incoming.clone());
            if incoming.channel == "echo" {
                reply
                    .send(Answer {
                        exit: 7,
                        stdout: format!("{}|{}", incoming.argv.join(","), incoming.stdin),
                        stderr: "e".into(),
                    })
                    .ok();
            } else {
                self.held.lock().unwrap().push(reply);
            }
            true
        }
    }

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    fn env(port: u16, token: &str, state: Option<PathBuf>) -> Env {
        Env {
            port: Some(port.to_string()),
            token: Some(token.to_owned()),
            state,
        }
    }

    #[test]
    fn a_hook_reaches_the_extension_and_prints_its_answer() {
        let router = Arc::new(Fake::default());
        let endpoint = Endpoint::start(router.clone()).unwrap();
        let answer = run(
            args(&["--channel", "ext/echo", "a", "b"]),
            &b"{\"tool\":\"Bash\"}"[..],
            &env(endpoint.port(), TOKEN, None),
        );
        assert_eq!(
            answer,
            Answer {
                exit: 7,
                stdout: "a,b|{\"tool\":\"Bash\"}".into(),
                stderr: "e".into(),
            }
        );
        assert_eq!(
            router.seen.lock().unwrap()[0],
            Incoming {
                token: TOKEN.into(),
                extension_id: "ext".into(),
                channel: "echo".into(),
                argv: args(&["a", "b"]),
                stdin: "{\"tool\":\"Bash\"}".into(),
            }
        );
    }

    #[test]
    fn with_fyx_closed_it_exits_at_once_with_on_failure() {
        let root = tempfile::tempdir().unwrap();
        let state_file = state_path(root.path());
        State {
            on_failure: BTreeMap::from([("ext/closed".to_owned(), 2)]),
        }
        .save(&state_file)
        .unwrap();
        let free_port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let started = Instant::now();
        for (channel, exit) in [("ext/open", 0), ("ext/closed", 2)] {
            let closed = env(free_port, TOKEN, Some(state_file.clone()));
            assert_eq!(
                run(args(&["--channel", channel]), io::empty(), &closed),
                Answer::failure(exit)
            );
            let no_env = Env {
                state: Some(state_file.clone()),
                ..Env::default()
            };
            assert_eq!(
                run(
                    args(&[&format!("--channel={channel}")]),
                    io::empty(),
                    &no_env
                ),
                Answer::failure(exit),
                "a terminal Fyx did not start"
            );
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "four calls, at most 300 ms each"
        );
        let missing = Env {
            state: Some(root.path().join("none.json")),
            ..Env::default()
        };
        assert_eq!(
            run(args(&["--channel", "ext/closed"]), io::empty(), &missing),
            Answer::failure(0)
        );
    }

    #[test]
    fn unknown_channels_wrong_tokens_and_timeouts_answer_on_failure() {
        let endpoint = Endpoint::start(Arc::new(Fake::default())).unwrap();
        let port = endpoint.port();
        let cases = [
            ("ext/nope", TOKEN, 0),
            ("other/echo", TOKEN, 0),
            ("ext/echo", "forged", 0),
            ("ext/closed", TOKEN, 3),
            ("ext/slow", TOKEN, 2),
        ];
        for (channel, token, exit) in cases {
            assert_eq!(
                run(
                    args(&["--channel", channel]),
                    io::empty(),
                    &env(port, token, None)
                ),
                Answer::failure(exit),
                "{channel} with {token}"
            );
        }
        let usage = run(args(&["ext/echo"]), io::empty(), &env(port, TOKEN, None));
        assert_eq!(usage.exit, 0);
        assert!(usage.stderr.starts_with("usage:"));
    }

    #[test]
    fn at_most_eight_requests_of_an_extension_wait() {
        let router = Arc::new(Fake::default());
        let endpoint = Endpoint::start(router.clone()).unwrap();
        let port = endpoint.port();
        let waiting = (0..MAX_IN_FLIGHT)
            .map(|_| {
                std::thread::spawn(move || {
                    run(
                        args(&["--channel", "ext/slow"]),
                        io::empty(),
                        &env(port, TOKEN, None),
                    )
                })
            })
            .collect::<Vec<_>>();
        let deadline = Instant::now() + Duration::from_secs(5);
        while router.held.lock().unwrap().len() < MAX_IN_FLIGHT {
            assert!(Instant::now() < deadline, "the first eight never arrived");
            std::thread::yield_now();
        }
        let started = Instant::now();
        let ninth = run(
            args(&["--channel", "ext/slow"]),
            io::empty(),
            &env(port, TOKEN, None),
        );
        assert_eq!(ninth, Answer::failure(2));
        assert_eq!(
            router.held.lock().unwrap().len(),
            MAX_IN_FLIGHT,
            "never delivered"
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "refused, not timed out"
        );
        for thread in waiting {
            assert_eq!(thread.join().unwrap(), Answer::failure(2));
        }
        // The slots are free again.
        assert_eq!(
            run(
                args(&["--channel", "ext/echo"]),
                io::empty(),
                &env(port, TOKEN, None)
            )
            .exit,
            7
        );
    }

    #[test]
    fn oversized_input_and_answers_are_cut_or_refused() {
        let router = Arc::new(Fake::default());
        let endpoint = Endpoint::start(router.clone()).unwrap();
        let big = vec![b'x'; MAX_STDIN_BYTES + 1];
        assert_eq!(
            run(
                args(&["--channel", "ext/echo"]),
                &big[..],
                &env(endpoint.port(), TOKEN, None)
            ),
            Answer::failure(0)
        );
        assert!(router.seen.lock().unwrap().is_empty());

        let answer = Answer {
            exit: 1,
            stdout: "é".repeat(MAX_ANSWER_BYTES),
            stderr: "lost".into(),
        }
        .truncated();
        assert!(answer.stdout.len() <= MAX_ANSWER_BYTES);
        assert!(answer.stdout.len() > MAX_ANSWER_BYTES - 2);
        assert!(answer.stderr.is_empty());
    }

    #[test]
    fn a_browser_request_gets_no_answer() {
        let endpoint = Endpoint::start(Arc::new(Fake::default())).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port())).unwrap();
        stream
            .write_all(b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://evil\r\n\r\n{}")
            .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).ok();
        assert!(bytes.is_empty());
    }

    #[test]
    fn channel_arguments() {
        assert_eq!(
            parse_args(args(&["--channel", "fyx-claude/pre-tool", "x"])),
            Some(("fyx-claude/pre-tool".into(), args(&["x"])))
        );
        for bad in [
            &["--channel"][..],
            &["--channel", "noslash"],
            &["--channel", "a/b/c"],
            &["--channel=/b"],
            &["x", "--channel", "a/b"],
            &[],
        ] {
            assert_eq!(parse_args(args(bad)), None, "{bad:?}");
        }
    }
}
