//! [`TerminalId`] and the per-terminal [`TerminalExt`] handle that Fyx keeps on every `Terminal`.

use std::collections::HashMap;
use std::fmt;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use rand::TryRngCore as _;
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
pub const BLANKED_INHERITED_ENV: &[&str] = &["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"];

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
        let mut bytes = [0u8; 32];
        OsRng
            .try_fill_bytes(&mut bytes)
            .expect("the OS random number generator failed");
        Self(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
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
}

/// State shared between the terminal and its pty tap on the IO thread.
#[derive(Debug)]
pub(crate) struct Shared {
    #[allow(dead_code, reason = "the OSC parser (phase 1, step 3) reports with it")]
    pub(crate) id: TerminalId,
}

impl TerminalExt {
    /// For terminals without a process (display-only, pseudo-terminals): an id, no token, no env.
    pub fn new() -> Self {
        Self::with_token(None)
    }

    /// For local pty terminals: allocates the id and token and writes Fyx's variables into the
    /// terminal's environment.
    pub fn insert_env<S: BuildHasher>(env: &mut HashMap<String, String, S>) -> Self {
        let this = Self::with_token(Some(TerminalToken::generate()));
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
            shared: Arc::new(Shared { id }),
        }
    }

    pub fn id(&self) -> TerminalId {
        self.id
    }

    pub fn token(&self) -> Option<&TerminalToken> {
        self.token.as_ref()
    }
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

    #[test]
    fn display_only_terminals_get_an_id_without_a_token() {
        let ext = TerminalExt::new();
        assert!(ext.token().is_none());
        assert_ne!(TerminalExt::new().id(), ext.id());
    }
}
