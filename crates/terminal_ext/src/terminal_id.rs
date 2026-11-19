//! [`TerminalId`] and the per-terminal [`TerminalExt`] handle that Fyx keeps on every `Terminal`.

use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use rand::TryRngCore as _;

use crate::env_hook::{ACCOUNT_CHOICE_ENV, AccountBadge, PARKED_ACCOUNTS_ENV, decode_badges};
use crate::inbox::Inbox;
use crate::osc::OscEvent;
use crate::status::{ExtensionStatus, StatusModel, TerminalStatus};
use rand::rngs::OsRng;

/// Env var with the terminal's [`TerminalId`], in decimal.
pub const TERMINAL_ID_ENV: &str = "FYX_TERMINAL_ID";
/// Env var with the terminal's secret token (64 hex digits). Programs in the terminal present it to
/// Fyx's loopback services to prove which terminal they run in.
pub const TERMINAL_TOKEN_ENV: &str = "FYX_TERMINAL_TOKEN";

/// Variables a Fyx terminal must not inherit from whatever launched Fyx. The env block always merges the
/// parent environment, so these are overridden with an empty value instead of removed.
///
/// `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` is set by Claude Desktop and turns off Claude Code's OSC 7501
/// status reports, so a Fyx started from there would never show Claude's state.
pub const BLANKED_INHERITED_ENV: &[&str] = &[
    "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
    PARKED_TERMINAL_ENV,
    ACCOUNT_CHOICE_ENV,
    PARKED_ACCOUNTS_ENV,
];

/// Env entry naming a [`ParkedTerminal`]. [`TerminalExt::insert_env`] removes it, so the program
/// never sees it.
pub const PARKED_TERMINAL_ENV: &str = "FYX_PARKED_TERMINAL";

/// Process-unique id of a terminal. Never reused while Fyx runs; not meaningful across restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TerminalId(u64);

impl TerminalId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A terminal's secret token. `Debug` never prints it.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TerminalToken(String);

impl TerminalToken {
    fn generate() -> Self {
        Self(random_hex())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TerminalToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TerminalToken(..)")
    }
}

fn random_hex() -> String {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .expect("the OS random number generator failed");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

static TOKENS: LazyLock<Mutex<HashMap<TerminalToken, TerminalId>>> =
    LazyLock::new(Default::default);

/// Which live terminal a token belongs to, if any.
pub fn terminal_for_token(token: &str) -> Option<TerminalId> {
    TOKENS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .get(&TerminalToken(token.to_owned()))
        .copied()
}

/// Fyx's state for one terminal, owned by the upstream `Terminal`. Dropping it forgets the token.
#[derive(Debug)]
pub struct TerminalExt {
    id: TerminalId,
    token: Option<TerminalToken>,
    pub(crate) shared: Arc<Shared>,
    pseudo: Option<Pseudo>,
}

/// The backend of a pseudo-terminal (`fyx:ui` §B.2): an extension. It gets what the user types and
/// the size; upstream never calls it with the terminal's own answers to output.
pub trait PseudoBackend: Send + Sync + 'static {
    /// Called on the main thread; must only queue.
    fn input(&self, bytes: &[u8]);
    /// Called on the main thread whenever the grid's size changes; must only queue.
    fn resized(&self, cols: u16, rows: u16);
}

struct Pseudo(Arc<dyn PseudoBackend>);

impl fmt::Debug for Pseudo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pseudo(..)")
    }
}

/// The raw output of a created terminal (`fyx:ui` §B.3): its extension reads every byte its
/// program writes, from the first one, as the pty wrote it.
pub trait RawTap: Send + Sync + 'static {
    /// Called on the pty's IO thread with the bytes of one read; must only queue.
    fn output(&self, bytes: &[u8]);
    /// The program is gone: its exit code, or `None` for a signal or a program that never
    /// started. May be called more than once (the pty closing, then the terminal's state); only
    /// the first call counts.
    fn finished(&self, code: Option<i32>);
}

/// Which extension a terminal belongs to, for the tab's source line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionOrigin {
    /// The extension is the backend (§B.2).
    Pseudo { extension: String },
    /// The extension started a declared program in it (§B.3).
    Created { extension: String },
}

impl ExtensionOrigin {
    pub fn extension(&self) -> &str {
        match self {
            Self::Pseudo { extension } | Self::Created { extension } => extension,
        }
    }

    /// "pseudo-terminal of extension acme", "started by extension acme".
    pub fn describe(&self) -> String {
        match self {
            Self::Pseudo { extension } => format!("pseudo-terminal of extension {extension}"),
            Self::Created { extension } => format!("started by extension {extension}"),
        }
    }
}

/// State shared between the terminal and its pty tap on the IO thread.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) inbox: Mutex<Inbox>,
    pub(crate) status: Mutex<StatusModel>,
    pub(crate) origin: Mutex<Option<ExtensionOrigin>>,
    pub(crate) accounts: Mutex<Vec<AccountBadge>>,
    pub(crate) raw_tap: Option<Tap>,
}

pub(crate) struct Tap(pub(crate) Arc<dyn RawTap>);

impl fmt::Debug for Tap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tap(..)")
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // The terminal and its pty are both gone; normally the pty reported the exit already.
        if let Some(Tap(tap)) = &self.raw_tap {
            tap.finished(None);
        }
    }
}

static PARKED: LazyLock<Mutex<HashMap<String, TerminalExt>>> = LazyLock::new(Default::default);

/// A [`TerminalExt`] waiting for `TerminalBuilder` to pick it up in [`TerminalExt::insert_env`].
/// Put [`ParkedTerminal::env`] into the builder's env. Dropping this drops the state if the
/// builder has not taken it.
#[derive(Debug)]
pub struct ParkedTerminal {
    key: String,
}

impl ParkedTerminal {
    /// The env entry that tells `insert_env` to use the parked state.
    pub fn env(&self) -> (String, String) {
        (PARKED_TERMINAL_ENV.to_owned(), self.key.clone())
    }
}

impl Drop for ParkedTerminal {
    fn drop(&mut self) {
        // Dropped outside the lock: the state's drop may call its tap.
        let parked = lock(&PARKED).remove(&self.key);
        drop(parked);
    }
}

impl TerminalExt {
    /// For terminals without a process (display-only, pseudo-terminals): an id, no token, no env.
    pub fn new() -> Self {
        Self::with_token(None)
    }

    /// For a pseudo-terminal: no token, no env, and input and resizes go to `backend`. The host
    /// builds a display-only `Terminal` and puts this in its `fyx` field.
    pub fn pseudo(extension: &str, backend: Arc<dyn PseudoBackend>) -> Self {
        let mut this = Self::with_token(None);
        this.pseudo = Some(Pseudo(backend));
        this.set_origin(Some(ExtensionOrigin::Pseudo {
            extension: extension.to_owned(),
        }));
        this
    }

    /// For a created terminal (§B.3): an id and a token like any pty terminal, the extension as
    /// its origin, and `tap` sees every byte of its output. [`Self::park`] it and let
    /// `TerminalBuilder` pick it up, so the tap is on the pty before the program starts.
    pub fn created(extension: &str, tap: Arc<dyn RawTap>) -> Self {
        let this = Self::with_parts(Some(TerminalToken::generate()), Some(Tap(tap)));
        this.set_origin(Some(ExtensionOrigin::Created {
            extension: extension.to_owned(),
        }));
        this
    }

    /// Parks this state for the `TerminalBuilder` whose env carries [`ParkedTerminal::env`].
    pub fn park(self) -> ParkedTerminal {
        let key = random_hex();
        lock(&PARKED).insert(key.clone(), self);
        ParkedTerminal { key }
    }

    pub fn is_pseudo(&self) -> bool {
        self.pseudo.is_some()
    }

    /// Upstream hook in `Terminal::write_to_pty`: a pseudo-terminal's input goes to its backend.
    /// Returns whether it did (then nothing else must see it).
    pub fn forward_input(&self, bytes: &[u8]) -> bool {
        match &self.pseudo {
            Some(Pseudo(backend)) => {
                backend.input(bytes);
                true
            }
            None => false,
        }
    }

    /// Upstream hook where the grid is resized.
    pub fn forward_resize(&self, cols: usize, rows: usize) {
        if let Some(Pseudo(backend)) = &self.pseudo {
            let clamp = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
            backend.resized(clamp(cols), clamp(rows));
        }
    }

    /// The extension the terminal belongs to, drawn on its tab.
    pub fn origin(&self) -> Option<ExtensionOrigin> {
        lock(&self.shared.origin).clone()
    }

    pub fn set_origin(&self, origin: Option<ExtensionOrigin>) {
        *lock(&self.shared.origin) = origin;
    }

    /// For local pty terminals: allocates the id and token (or takes the [`ParkedTerminal`] the
    /// env names) and writes Fyx's variables into the terminal's environment.
    pub fn insert_env<S: BuildHasher>(env: &mut HashMap<String, String, S>) -> Self {
        let parked = env
            .remove(PARKED_TERMINAL_ENV)
            .and_then(|key| lock(&PARKED).remove(&key));
        let this = parked.unwrap_or_else(|| Self::with_token(Some(TerminalToken::generate())));
        if let Some(badges) = env.remove(PARKED_ACCOUNTS_ENV) {
            *lock(&this.shared.accounts) = decode_badges(&badges);
        }
        env.insert(TERMINAL_ID_ENV.to_owned(), this.id.to_string());
        if let Some(token) = &this.token {
            env.insert(TERMINAL_TOKEN_ENV.to_owned(), token.expose().to_owned());
        }
        for name in BLANKED_INHERITED_ENV {
            if std::env::var_os(name).is_some() {
                env.insert((*name).to_owned(), String::new());
            }
        }
        this
    }

    fn with_token(token: Option<TerminalToken>) -> Self {
        Self::with_parts(token, None)
    }

    fn with_parts(token: Option<TerminalToken>, raw_tap: Option<Tap>) -> Self {
        let id = TerminalId::next();
        if let Some(token) = &token {
            TOKENS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .insert(token.clone(), id);
        }
        Self {
            id,
            token,
            shared: Arc::new(Shared {
                inbox: Mutex::default(),
                status: Mutex::default(),
                origin: Mutex::default(),
                accounts: Mutex::default(),
                raw_tap,
            }),
            pseudo: None,
        }
    }

    pub fn id(&self) -> TerminalId {
        self.id
    }

    pub fn token(&self) -> Option<&TerminalToken> {
        self.token.as_ref()
    }

    /// OSC events parsed on the IO thread since the last call, oldest first, and how many were
    /// dropped by rate limits or a full queue. Call it on the terminal's `Wakeup`: alacritty sends
    /// one after every read, so every queued event is followed by one.
    /// What the tab shows: the program's own report (OSC 7501, else OSC 9;4 progress).
    pub fn status(&self) -> Option<TerminalStatus> {
        lock(&self.shared.status).current()
    }

    /// A `Send + Sync` handle to this terminal's status, for hosts off the main thread.
    pub fn status_handle(&self) -> StatusHandle {
        StatusHandle {
            id: self.id,
            shared: self.shared.clone(),
        }
    }

    /// The account profiles the terminal was started with (design §C.6), for its tab.
    pub fn accounts(&self) -> Vec<AccountBadge> {
        lock(&self.shared.accounts).clone()
    }

    /// The user has seen the terminal (it got focus).
    pub fn clear_attention(&self) {
        lock(&self.shared.status).clear_attention();
    }

    pub fn take_osc_events(&self) -> (Vec<OscEvent>, u64) {
        self.shared
            .inbox
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .drain()
    }
}

/// See [`TerminalExt::status_handle`]. Setting a status does not repaint the tab: the caller
/// sends the terminal a `Wakeup`.
#[derive(Clone, Debug)]
pub struct StatusHandle {
    id: TerminalId,
    shared: Arc<Shared>,
}

impl StatusHandle {
    pub fn id(&self) -> TerminalId {
        self.id
    }

    pub fn status(&self) -> Option<TerminalStatus> {
        lock(&self.shared.status).current()
    }

    /// `None` removes the extension's status.
    pub fn set_extension_status(&self, extension: &str, status: Option<ExtensionStatus>) {
        lock(&self.shared.status).set_extension_status(extension, status, Instant::now());
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Default for TerminalExt {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TerminalExt {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            TOKENS
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(&token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_env_sets_id_and_token_and_registers_the_token() {
        let mut env = HashMap::new();
        let ext = TerminalExt::insert_env(&mut env);
        assert_eq!(env[TERMINAL_ID_ENV], ext.id().to_string());
        let token = env[TERMINAL_TOKEN_ENV].clone();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(terminal_for_token(&token), Some(ext.id()));
        assert!(!format!("{ext:?}").contains(&token));

        let other = TerminalExt::insert_env(&mut HashMap::new());
        assert_ne!(other.id(), ext.id());
        assert_ne!(other.token(), ext.token());

        drop(ext);
        assert_eq!(terminal_for_token(&token), None);
    }

    #[derive(Default)]
    struct Recorder {
        output: Mutex<Vec<u8>>,
        finished: Mutex<Vec<Option<i32>>>,
    }

    impl RawTap for Recorder {
        fn output(&self, bytes: &[u8]) {
            lock(&self.output).extend_from_slice(bytes);
        }

        fn finished(&self, code: Option<i32>) {
            lock(&self.finished).push(code);
        }
    }

    #[test]
    fn insert_env_takes_a_parked_terminal_and_removes_its_key() {
        let tap = Arc::new(Recorder::default());
        let created = TerminalExt::created("acme", tap.clone());
        let id = created.id();
        let parked = created.park();
        let mut env = HashMap::from([parked.env()]);
        let ext = TerminalExt::insert_env(&mut env);
        assert_eq!(ext.id(), id);
        assert!(!env.contains_key(PARKED_TERMINAL_ENV));
        assert_eq!(env[TERMINAL_ID_ENV], id.to_string());
        assert_eq!(
            terminal_for_token(&env[TERMINAL_TOKEN_ENV]),
            Some(id),
            "a created terminal has a token like any pty terminal"
        );
        assert_eq!(
            ext.origin(),
            Some(ExtensionOrigin::Created {
                extension: "acme".into()
            })
        );

        // Taken once: the same key again makes a fresh terminal.
        let mut again = HashMap::from([parked.env()]);
        assert_ne!(TerminalExt::insert_env(&mut again).id(), id);
        drop(parked);
        assert!(lock(&tap.finished).is_empty());
        drop(ext);
        assert_eq!(*lock(&tap.finished), [None]);
    }

    #[test]
    fn dropping_an_unused_parked_terminal_finishes_its_tap() {
        let tap = Arc::new(Recorder::default());
        let parked = TerminalExt::created("acme", tap.clone()).park();
        let (_, key) = parked.env();
        drop(parked);
        assert!(!lock(&PARKED).contains_key(&key));
        assert_eq!(*lock(&tap.finished), [None]);
    }

    #[test]
    fn display_only_terminals_get_an_id_without_a_token() {
        let ext = TerminalExt::new();
        assert!(ext.token().is_none());
        assert_ne!(TerminalExt::new().id(), ext.id());
    }
}
