//! The pure part of the fyx:ui 0.4 terminal capabilities (docs/fyx/wit-0.4/design.md §B.1–§B.6,
//! §C.6, plan 5.5): the types and limits, the manifest tables `[[fyx_ui.terminal_profiles]]`,
//! `[[fyx_ui.terminal_programs]]` and `[fyx_ui.terminal_env]`, the `terminal:env` name rules,
//! the `${…}` templates, and [`TerminalHost`], the trait the terminal host implements.
//!
//! The split agreed with W1 (2026-11-19): the import (`ui_ext_wasm`, W2c) checks capabilities,
//! declared programs and argument patterns, untrusted worktrees, per-call sizes, the per-extension
//! caps and handle ownership (through [`TerminalError::NotFound`]); the host (`terminal_host`, W1)
//! enforces rates, sanitises pseudo-terminal output, owns process trees, `FYX_TERMINAL_ID` and
//! `FYX_TERMINAL_TOKEN`, allocates handles and injects env into profile and "all new" terminals.
//! No I/O here, so the harness can make the same checks.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::{
    process,
    v0_2::{HostError, HostErrorCode, Icon},
};

pub const CAPABILITY_PROFILES: &str = "terminal:profiles";
pub const CAPABILITY_PSEUDO: &str = "terminal:pseudo";
pub const CAPABILITY_CREATE: &str = "terminal:create";
pub const CAPABILITY_ENV: &str = "terminal:env";
pub const CAPABILITY_STATUS: &str = "terminal:status";
pub const CAPABILITY_OBSERVE: &str = "terminal:observe";
/// T3: asked at the moment of action, on one terminal (§C.3).
pub const CAPABILITY_READ: &str = "terminal:read";
/// T3: every command confirmed (§C.3).
pub const CAPABILITY_WRITE: &str = "terminal:write";
pub const CAPABILITY_PACKAGE_FILES: &str = "package:files";

// ---------------------------------------------------------------------------------------------
// Limits

/// §B.1.
pub const MAX_PROFILES: usize = 16;
pub const MAX_PROFILE_ARGS: usize = 64;
pub const MAX_PROFILE_ARG_BYTES: usize = 1024;
/// §B.2: one `write-output` call, and the host's rate per pseudo-terminal.
pub const MAX_WRITE_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_PSEUDO_OUTPUT_BYTES_PER_SECOND: usize = 4 << 20;
/// §B.2: `terminal-input` events, coalesced by the host.
pub const MAX_INPUT_EVENTS_PER_SECOND: u32 = 30;
pub const MAX_INPUT_EVENT_BYTES: usize = 64 * 1024;
/// §B.3: `terminal-output` of a created terminal; beyond the buffer the oldest output is dropped.
pub const MAX_OUTPUT_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_EVENTS_PER_SECOND: u32 = 30;
pub const MAX_OUTPUT_BUFFER_BYTES: usize = 1 << 20;
/// §B.3: one `send-input` call.
pub const MAX_SEND_INPUT_BYTES: usize = 64 * 1024;
/// §B.3: created terminals per extension.
pub const MAX_CREATED_TERMINALS: usize = 8;
/// §B.5.
pub const MAX_STATUS_UPDATES_PER_SECOND: u32 = 4;
pub const MAX_STATUS_LABEL_BYTES: usize = 64;
pub const MAX_STATUS_DETAIL_BYTES: usize = 512;
pub const MAX_TITLE_BYTES: usize = 256;
/// §B.4.
pub const MAX_ENV_VARS: usize = 32;
pub const MAX_ENV_VALUE_BYTES: usize = 4096;
/// §B.11: one `paste`, after the host strips control characters.
pub const MAX_PASTE_BYTES: usize = 16 * 1024;
/// §C.3: `request-read` with `last-lines`.
pub const MAX_READ_LINES: u16 = 500;
/// §C.3: one `request-write`, whose card shows the exact text.
pub const MAX_T3_WRITE_BYTES: usize = 16 * 1024;

// ---------------------------------------------------------------------------------------------
// Types (the WIT `terminals` interface, design "WIT diff")

/// An extension's name for a terminal. Allocated by the host per extension and never reused
/// within one load; an unknown or foreign one is [`TerminalError::NotFound`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TerminalHandle(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalOrigin {
    /// The extension is the backend (§B.2).
    Pseudo,
    /// A declared program the extension started (§B.3).
    Created,
    /// The user started one of the extension's profiles (§B.1). The user's terminal.
    Profile,
    /// Any other terminal: the user's.
    User,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusState {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TerminalStatus {
    pub state: StatusState,
    /// At most [`MAX_STATUS_LABEL_BYTES`].
    pub label: String,
    /// At most [`MAX_STATUS_DETAIL_BYTES`].
    pub detail: Option<String>,
    /// `0..=100`.
    pub progress: Option<u8>,
    pub attention: bool,
}

impl TerminalStatus {
    /// The import's check of `set-status`; the host still draws the source next to it.
    pub fn check(&self) -> Result<(), HostError> {
        check_text("status label", &self.label, MAX_STATUS_LABEL_BYTES)?;
        if let Some(detail) = &self.detail {
            check_text("status detail", detail, MAX_STATUS_DETAIL_BYTES)?;
        }
        if self.progress.is_some_and(|progress| progress > 100) {
            return Err(invalid("status progress must be 0..=100"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TerminalInfo {
    pub handle: TerminalHandle,
    pub origin: TerminalOrigin,
    /// Redacted without a T3 read (§B.5), since a tab title is often the running command: `•••`
    /// for a user terminal and for another extension's terminal, the profile's name for a profile
    /// terminal, and the extension's own title for its own terminals (agreed with W1,
    /// 2026-11-19).
    pub title: String,
    pub cwd: Option<String>,
    pub status: Option<TerminalStatus>,
    /// The account profile's name, never its path (§C.6).
    pub account: Option<String>,
}

/// `terminals.open-pseudo` (§B.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PseudoRequest {
    pub title: String,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// Shown in the extension's panel by a `terminal` node rather than as a tab.
    pub in_panel: bool,
}

impl PseudoRequest {
    pub fn check(&self) -> Result<(), HostError> {
        check_text("terminal title", &self.title, MAX_TITLE_BYTES)?;
        if self.cols == Some(0) || self.rows == Some(0) {
            return Err(invalid("a terminal has at least one column and one row"));
        }
        Ok(())
    }
}

/// A created terminal's command (§B.3), after the import resolved and checked it: the program is
/// a declared `[[fyx_ui.terminal_programs]]` entry, the arguments fit its patterns, the cwd is in
/// a trusted worktree, and `env` is the extension's granted `terminal:env` with its templates
/// resolved. The host adds the user's terminal env, `FYX_TERMINAL_ID` and `FYX_TERMINAL_TOKEN`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadScope {
    /// The last command block; needs shell integration (plan 5.17).
    LastCommand,
    /// The last N lines, N at most [`MAX_READ_LINES`].
    LastLines,
}

/// What the host delivers to an extension about its terminals (the 0.4 `io-event` cases).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Keyboard input to a pseudo-terminal, coalesced.
    Input(TerminalHandle, Vec<u8>),
    /// The last size only.
    Resized(TerminalHandle, u16, u16),
    /// A created terminal's output, and how many bytes were dropped before it.
    Output(TerminalHandle, Vec<u8>, u64),
    /// `None` for a signal or a spawn that failed.
    Exited(TerminalHandle, Option<i32>),
    /// `terminal:observe`.
    Created(TerminalInfo),
    Closed(TerminalHandle),
    StatusChanged(TerminalInfo),
    /// §C.3: the answer to `request-read` (by its id): the text, or why not.
    ReadDone(u64, Result<String, String>),
    /// §C.3: the answer to `request-write`.
    WriteDone(u64, Result<(), String>),
}

/// P2.4: what the host needs to start an extension's profile terminals and inject its env
/// into them (and into every new terminal for `scope = "all-new"`), as granted at load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalIntegration {
    pub profiles: Vec<TerminalProfile>,
    /// `[fyx_ui.terminal_env]` when `terminal:env` is granted.
    pub env: Option<TerminalEnv>,
    /// The second, stronger checkbox of §B.4 is granted too.
    pub env_all_new: bool,
    /// The extension's package directory, when `package:files` is declared.
    pub package_dir: Option<PathBuf>,
}

/// §C.3: what a T3 card and the import's checks need about a terminal the extension names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T3Target {
    /// The host's id of the terminal (`ui_ext_wasm::permissions::HostTerminalId`).
    pub terminal: u64,
    pub origin: TerminalOrigin,
    /// Not redacted: the card shows it to the user.
    pub title: String,
    /// The terminal's working directory, for the untrusted-worktree check.
    pub cwd: Option<PathBuf>,
}

/// §C.3: `request-write` text is printable, with tab; no control keys, no ESC.
pub fn check_write_text(text: &str) -> Result<(), HostError> {
    if text.len() > MAX_T3_WRITE_BYTES {
        return Err(invalid(format!(
            "request-write text is at most {MAX_T3_WRITE_BYTES} bytes"
        )));
    }
    if text.chars().any(|c| c != '\t' && c.is_control()) {
        return Err(invalid(
            "request-write text is printable characters and tab only (submit adds Enter)",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TerminalError {
    #[error("the capability is denied")]
    Denied,
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("rate limited")]
    RateLimited,
    #[error("no such terminal")]
    NotFound,
    #[error("too large: {0}")]
    TooLarge(String),
    #[error("{0}")]
    Other(String),
}

impl From<TerminalError> for HostError {
    fn from(error: TerminalError) -> Self {
        let code = match &error {
            TerminalError::Denied => HostErrorCode::Denied,
            TerminalError::Unavailable(_) | TerminalError::Other(_) => HostErrorCode::Unavailable,
            TerminalError::RateLimited | TerminalError::TooLarge(_) => HostErrorCode::LimitExceeded,
            TerminalError::NotFound => HostErrorCode::InvalidArgument,
        };
        HostError {
            code,
            message: error.to_string(),
        }
    }
}

/// Where the host sends [`TerminalEvent`]s. It may be called from any thread (the pty IO thread,
/// background executors), never while the host holds a lock of its own. It must not block and
/// must not call back into [`TerminalHost`] synchronously: it queues onto the extension's calls.
pub type TerminalSink = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

/// The terminal host, implemented by W1's `terminal_host` crate and passed to the runtime by
/// `ui_ext_host`. `ext` is the extension id. Synchronous: work that takes time finishes on the
/// host's side (a spawn that fails arrives as [`TerminalEvent::Exited`] with `None`).
pub trait TerminalHost: Send + Sync + 'static {
    /// §B.2. The host queues, sanitises and rate-limits the output.
    fn open_pseudo(
        &self,
        ext: &str,
        request: PseudoRequest,
        sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError>;
    /// §B.3. Killed with its process tree by [`TerminalHost::close_all`].
    fn create(
        &self,
        ext: &str,
        command: CreateCommand,
        sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError>;
    /// At most [`MAX_WRITE_OUTPUT_BYTES`] (checked by the import); [`TerminalError::RateLimited`]
    /// above [`MAX_PSEUDO_OUTPUT_BYTES_PER_SECOND`].
    fn write_output(
        &self,
        ext: &str,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), TerminalError>;
    /// A created terminal's input, at most [`MAX_SEND_INPUT_BYTES`].
    fn send_input(
        &self,
        ext: &str,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), TerminalError>;
    /// §B.11: only in bracketed-paste mode; the host strips control characters except tab and
    /// newline, wraps the text and caps it at [`MAX_PASTE_BYTES`].
    fn paste(&self, ext: &str, handle: TerminalHandle, text: String) -> Result<(), TerminalError>;
    /// §B.5: at most [`MAX_STATUS_UPDATES_PER_SECOND`]; the host draws the source.
    fn set_status(
        &self,
        ext: &str,
        handle: TerminalHandle,
        status: Option<TerminalStatus>,
    ) -> Result<(), TerminalError>;
    fn close(&self, ext: &str, handle: TerminalHandle);
    /// `terminal:observe`: the terminals now, command lines redacted; with a sink, later
    /// `Created`, `Closed` and `StatusChanged` events go to it, and `None` stops them.
    fn observe(&self, ext: &str, sink: Option<TerminalSink>) -> Vec<TerminalInfo>;
    /// Reload, disable, uninstall, revocation: every terminal of `ext` closes, created ones with
    /// their process trees, and observing stops.
    fn close_all(&self, ext: &str);
    /// P2.4: the extension's profiles and env, sent at load and again when a grant changes;
    /// [`TerminalHost::close_all`] clears it.
    fn set_integration(&self, ext: &str, integration: TerminalIntegration) {
        let _ = (ext, integration);
    }
    /// §C.3: the terminal behind a handle from `observe` or `list`, for a T3 card.
    fn t3_target(&self, ext: &str, handle: TerminalHandle) -> Result<T3Target, TerminalError> {
        let _ = (ext, handle);
        Err(TerminalError::Unavailable(
            "this window cannot share terminals".into(),
        ))
    }
    /// §C.3, after the user agreed: the text as plain text without ANSI, sent as
    /// [`TerminalEvent::ReadDone`] through `sink`.
    fn read(
        &self,
        ext: &str,
        handle: TerminalHandle,
        scope: ReadScope,
        lines: u16,
        id: u64,
        sink: TerminalSink,
    ) {
        let _ = (ext, handle, scope, lines);
        sink(TerminalEvent::ReadDone(id, Err("unavailable".into())));
    }
    /// §C.3, after the card confirmed the exact text (checked by [`check_write_text`]): typed,
    /// with Enter when `submit`; the tab shows "typed by <extension>" for 5 s. Answered with
    /// [`TerminalEvent::WriteDone`] through `sink`.
    fn write(
        &self,
        ext: &str,
        handle: TerminalHandle,
        text: String,
        submit: bool,
        id: u64,
        sink: TerminalSink,
    ) {
        let _ = (ext, handle, text, submit);
        sink(TerminalEvent::WriteDone(id, Err("unavailable".into())));
    }
}

// ---------------------------------------------------------------------------------------------
// `terminal:env` names (§B.4, §C.6)

/// Names that move where a shell finds programs and its configuration, and the host's own
/// (integration-6 N1), refused for `terminal:env` beside [`process::is_forbidden_env`].
const SHELL_ENV_NAMES: &[&str] = &[
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "SHELL",
    "ZDOTDIR",
    "INPUTRC",
    "FPATH",
    "CDPATH",
    // p5-security-0.4 P1: prompts are expanded, so `PS1='$(cmd)'` runs cmd in the user's shell
    // (bash `promptvars`, zsh `PROMPT_SUBST`); `HISTFILE=~/.bashrc` appends typed lines to it.
    "PS0",
    "PS1",
    "PS2",
    "PS3",
    "PS4",
    "PROMPT",
    "RPROMPT",
    "RPS1",
    "RPS2",
    "HISTFILE",
    // p5-security-0.4 P2: proxies and certificate stores move a tool's traffic, with the user's
    // credentials in it, to another host.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "FTP_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "NODE_TLS_REJECT_UNAUTHORIZED",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
];
const SHELL_ENV_PREFIXES: &[&str] = &["XDG_", "TERM", "FYX_", "ZED_"];
/// p5-security-0.4 P2: a tool's API endpoint (`ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`,
/// `OPENAI_API_BASE`, ...) gets the user's key with every request.
const ENDPOINT_ENV_SUFFIXES: &[&str] = &["_BASE_URL", "_API_BASE", "_API_URL", "_ENDPOINT"];

/// Account config dirs (§C.6 rule 5). Every env name an account-profile preset uses must be in
/// this list (`fyx.accounts` presets are settings data, W1).
pub const ACCOUNT_DIR_ENV_NAMES: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "GEMINI_CLI_HOME",
    "COPILOT_HOME",
    "OPENCODE_CONFIG_DIR",
    "OPENCODE_CONFIG",
    "OPENCODE_CONFIG_CONTENT",
    "QWEN_CODE_HOME",
    "AMP_HOME",
];

/// The host's account profiles (`fyx.accounts`, §C.6), as the extension runtime needs them:
/// `${account.<tool>}` for a terminal an extension creates, the user's own tools' config-dir
/// variables (refused like [`ACCOUNT_DIR_ENV_NAMES`]), and the dirs extensions stay out of.
/// Dirs are opaque paths: neither side looks inside, and the extension never sees one.
pub trait AccountDirs: Send + Sync + 'static {
    /// Every account profile's dir, `~` expanded.
    fn dirs(&self) -> Vec<PathBuf>;
    /// The config-dir variable of every tool in `fyx.accounts.tools`.
    fn env_names(&self) -> Vec<String>;
    /// The dir of the account chosen for `tool` in the project that holds `cwd`, if any. Never
    /// another account of the tool: no choice means no dir.
    fn account_dir(&self, tool: &str, cwd: &Path) -> Option<PathBuf>;
}

/// Whether `path` is one of `dirs` or inside one, compared as written and, where they exist, as
/// the files they name (links resolved), in any letter case on Windows.
pub fn in_account_dirs(path: &Path, dirs: &[PathBuf]) -> bool {
    fn key(path: &Path) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(path.to_string_lossy().to_lowercase())
        } else {
            path.to_path_buf()
        }
    }
    let spellings = |path: &Path| {
        let mut spellings = vec![key(path)];
        if let Ok(real) = path.canonicalize() {
            let real = real
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_owned();
            spellings.push(key(Path::new(&real)));
        }
        spellings
    };
    let paths = spellings(path);
    dirs.iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| spellings(dir))
        .any(|dir| paths.iter().any(|path| path.starts_with(&dir)))
}

/// Credentials (§C.6 rule 5), beside every name ending in one of [`CREDENTIAL_ENV_SUFFIXES`].
pub const CREDENTIAL_ENV_NAMES: &[&str] = &[
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "COPILOT_GITHUB_TOKEN",
];
pub const CREDENTIAL_ENV_SUFFIXES: &[&str] =
    &["_API_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_CREDENTIALS"];

/// Whether `name` selects an account's config dir or carries a credential (§C.6 rule 5). Also
/// refused for `[fyx_ui.process] env`, which is an allowlist that already leaves them out.
pub fn is_account_env(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ACCOUNT_DIR_ENV_NAMES.contains(&upper.as_str())
        || CREDENTIAL_ENV_NAMES.contains(&upper.as_str())
        || CREDENTIAL_ENV_SUFFIXES
            .iter()
            .any(|suffix| upper.ends_with(suffix))
}

/// Why `terminal:env` may not declare `name`, or `None`. In any letter case.
fn terminal_env_refusal(name: &str) -> Option<&'static str> {
    let upper = name.to_ascii_uppercase();
    if process::is_forbidden_env(name) {
        Some("loads code into the shell or the programs it starts")
    } else if terminal_env_refusal_without_credentials(&upper).is_some() {
        Some("changes where the shell finds programs or its configuration, or is Fyx's own")
    } else if ACCOUNT_DIR_ENV_NAMES.contains(&upper.as_str()) {
        Some("selects an account's config dir, which only Fyx's account profiles set")
    } else if is_account_env(name) {
        Some("carries a credential, which Fyx never passes for an extension")
    } else {
        None
    }
}

/// Whether `terminal:env` may never declare `name`: [`process::is_forbidden_env`], the shell and
/// host names, and [`is_account_env`]. One exception, [`check_terminal_env`]'s: a name with a
/// credential suffix whose value is exactly one listener's host token.
pub fn is_forbidden_terminal_env(name: &str) -> bool {
    terminal_env_refusal(name).is_some()
}

// ---------------------------------------------------------------------------------------------
// Templates (§B.4, §B.6, §B.7, §B.9, §C.6)

/// One part of a template value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplatePart {
    Literal(String),
    /// `${package}`: the extension version's installed folder (§B.6).
    Package,
    /// `${listen.<name>.port}`.
    ListenPort(String),
    /// `${listen.<name>.token}`, only for a listener with `token_in_env = true`.
    ListenToken(String),
    /// `${relay}`: the `fyx-relay` binary (§B.9).
    Relay,
    /// `${account.<tool>}`: the terminal's account profile dir for `tool` (§C.6).
    Account(String),
    /// `${pid}`: the Fyx process id, which IDE-integration tools match against their
    /// discovery files (§B.8; `terminal:env` values only).
    Pid,
}

/// A value with `${…}` placeholders, resolved by the host when a terminal starts. The extension
/// never sees the resolved text.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Template(pub Vec<TemplatePart>);

/// What a template may refer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateUse {
    /// Profile `args`: `${package}` and `${relay}`.
    ProfileArg,
    /// `terminal:env` values: every placeholder.
    EnvValue,
}

/// The host's side of [`Template::resolve`]. `None` leaves the whole value out.
pub trait TemplateContext {
    fn package(&self) -> Option<String>;
    fn listen_port(&self, listener: &str) -> Option<u16>;
    fn listen_token(&self, listener: &str) -> Option<String>;
    fn relay(&self) -> Option<String>;
    fn account_dir(&self, tool: &str) -> Option<String>;
    /// `${pid}`: this process unless the host says otherwise.
    fn pid(&self) -> Option<u32> {
        Some(std::process::id())
    }
}

impl Template {
    /// Parses `text`. `$` not followed by `{` is literal; `${` must close and name a known
    /// placeholder that `usage` allows. `listener` says whether a listener of that name is
    /// declared, and whether it has `token_in_env`.
    pub fn parse(
        text: &str,
        usage: TemplateUse,
        listener: &dyn Fn(&str) -> Option<bool>,
    ) -> Result<Self, String> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            literal.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after
                .find('}')
                .ok_or_else(|| format!("{text:?}: an unclosed ${{"))?;
            let part = parse_placeholder(&after[..end], usage, listener)
                .map_err(|why| format!("{text:?}: {why}"))?;
            if !literal.is_empty() {
                parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
            }
            parts.push(part);
            rest = &after[end + 1..];
        }
        literal.push_str(rest);
        if !literal.is_empty() {
            parts.push(TemplatePart::Literal(literal));
        }
        // p5-security-0.4 P6: `${package}/../..` would name files outside the extension under
        // "files from this extension", and `${account.<tool>}/../<other>` another account's dir,
        // which the extension never chooses (§C.6). No `..` component after either.
        let mut after_dir = false;
        for part in &parts {
            match part {
                TemplatePart::Package | TemplatePart::Account(_) => after_dir = true,
                TemplatePart::Literal(text)
                    if after_dir && text.split(['/', '\\']).any(|component| component == "..") =>
                {
                    return Err(format!(
                        "{text:?}: no .. after ${{package}} or ${{account.…}}"
                    ));
                }
                _ => {}
            }
        }
        Ok(Self(parts))
    }

    /// The text, or `None` when a placeholder has no value (a listener that is not open, no
    /// account profile for the tool): then the variable or argument is left out.
    pub fn resolve(&self, cx: &dyn TemplateContext) -> Option<String> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                TemplatePart::Literal(text) => out.push_str(text),
                TemplatePart::Package => out.push_str(&cx.package()?),
                TemplatePart::ListenPort(name) => out.push_str(&cx.listen_port(name)?.to_string()),
                TemplatePart::ListenToken(name) => out.push_str(&cx.listen_token(name)?),
                TemplatePart::Relay => out.push_str(&cx.relay()?),
                TemplatePart::Account(tool) => out.push_str(&cx.account_dir(tool)?),
                TemplatePart::Pid => out.push_str(&cx.pid()?.to_string()),
            }
        }
        Some(out)
    }

    /// Whether the value points into the package (shown as "files from this extension").
    pub fn uses_package(&self) -> bool {
        self.0.contains(&TemplatePart::Package)
    }

    /// Whether the value is exactly one listener's token.
    fn is_only_token(&self) -> bool {
        matches!(self.0.as_slice(), [TemplatePart::ListenToken(_)])
    }
}

fn parse_placeholder(
    inner: &str,
    usage: TemplateUse,
    listener: &dyn Fn(&str) -> Option<bool>,
) -> Result<TemplatePart, String> {
    let part = match inner.split('.').collect::<Vec<_>>().as_slice() {
        ["package"] => TemplatePart::Package,
        ["relay"] => TemplatePart::Relay,
        ["pid"] => TemplatePart::Pid,
        ["listen", name, "port"] => {
            listener(name).ok_or_else(|| format!("no listener named {name:?}"))?;
            TemplatePart::ListenPort(name.to_string())
        }
        ["listen", name, "token"] => match listener(name) {
            Some(true) => TemplatePart::ListenToken(name.to_string()),
            Some(false) => {
                return Err(format!(
                    "listener {name:?} does not declare token_in_env = true"
                ));
            }
            None => return Err(format!("no listener named {name:?}")),
        },
        ["account", tool] if is_id(tool, 32) => TemplatePart::Account(tool.to_string()),
        _ => return Err(format!("unknown placeholder ${{{inner}}}")),
    };
    let allowed = match usage {
        TemplateUse::EnvValue => true,
        TemplateUse::ProfileArg => matches!(part, TemplatePart::Package | TemplatePart::Relay),
    };
    if !allowed {
        return Err(format!("${{{inner}}} is not allowed in profile args"));
    }
    Ok(part)
}

// ---------------------------------------------------------------------------------------------
// Manifest tables

/// `cwd` of a profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileCwd {
    /// The active worktree.
    #[default]
    Worktree,
    Home,
}

/// One `[[fyx_ui.terminal_profiles]]` entry as written (§B.1).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalProfileSpec {
    /// `[a-z0-9-]{1,32}`, unique in the extension.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<Icon>,
    /// Resolved like `[[fyx_ui.processes]]` (0.3 design §9): on `PATH` outside the worktrees,
    /// or `path` in a trusted worktree.
    pub program: String,
    #[serde(default)]
    pub path: Option<String>,
    /// Literal, with `${package}` and `${relay}`.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: ProfileCwd,
    /// The tool whose account profile the user picks for this terminal (§C.6).
    #[serde(default)]
    pub account_tool: Option<String>,
}

/// A checked profile, for the "+" menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalProfile {
    pub id: String,
    pub name: String,
    pub icon: Option<Icon>,
    pub program: String,
    pub path: Option<String>,
    pub args: Vec<Template>,
    pub cwd: ProfileCwd,
    pub account_tool: Option<String>,
}

impl TerminalProfile {
    /// Whether the profile starts a program that runs any command typed into it (the menu
    /// entry says so; owner's Q2).
    pub fn runs_any_code(&self) -> bool {
        process::runs_any_code(&process::ProcessSpec {
            name: self.program.clone(),
            args: vec!["**".into()],
            path: self.path.clone(),
            reason: None,
        })
    }
}

/// Checks `[[fyx_ui.terminal_profiles]]`.
pub fn check_profiles(specs: Vec<TerminalProfileSpec>) -> Result<Vec<TerminalProfile>, String> {
    if specs.len() > MAX_PROFILES {
        return Err(format!(
            "[[fyx_ui.terminal_profiles]] has {} entries, at most {MAX_PROFILES}",
            specs.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    let no_listener = |_: &str| None;
    specs
        .into_iter()
        .map(|spec| {
            let at = |why: String| format!("terminal profile {:?}: {why}", spec.id);
            if !is_id(&spec.id, 32) {
                return Err(at("the id must be [a-z0-9-]{1,32}".into()));
            }
            if !seen.insert(spec.id.clone()) {
                return Err(at("the id is used twice".into()));
            }
            check_text("name", &spec.name, 64).map_err(|error| at(error.message))?;
            check_program_name(&spec.program).map_err(at)?;
            if let Some(tool) = &spec.account_tool
                && !is_id(tool, 32)
            {
                return Err(at("account_tool must be [a-z0-9-]{1,32}".into()));
            }
            if spec.args.len() > MAX_PROFILE_ARGS {
                return Err(at(format!("at most {MAX_PROFILE_ARGS} args")));
            }
            let args = spec
                .args
                .iter()
                .map(|arg| {
                    if arg.len() > MAX_PROFILE_ARG_BYTES || arg.contains('\0') {
                        return Err(at(format!(
                            "an argument has a NUL or more than {MAX_PROFILE_ARG_BYTES} bytes"
                        )));
                    }
                    Template::parse(arg, TemplateUse::ProfileArg, &no_listener).map_err(at)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TerminalProfile {
                id: spec.id,
                name: spec.name,
                icon: spec.icon,
                program: spec.program,
                path: spec.path,
                args,
                cwd: spec.cwd,
                account_tool: spec.account_tool,
            })
        })
        .collect()
}

/// One `[[fyx_ui.terminal_programs]]` entry (§B.3): a `[[fyx_ui.processes]]` entry for
/// `terminal:create`, with the author's reason for the install dialog.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalProgramSpec {
    pub name: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    /// Required for a T2 item: plain text, at most [`MAX_REASON_BYTES`] (§C.2).
    pub reason: String,
}

impl TerminalProgramSpec {
    pub fn as_process(&self) -> process::ProcessSpec {
        process::ProcessSpec {
            name: self.name.clone(),
            args: self.args.clone(),
            path: self.path.clone(),
            reason: Some(self.reason.clone()),
        }
    }
}

/// The author's one-line reason for a T2 item (§C.2).
pub const MAX_REASON_BYTES: usize = 200;

/// Checks a T2 item's `reason`: present, one line, no control characters, at most
/// [`MAX_REASON_BYTES`].
pub fn check_reason(what: &str, reason: &str) -> Result<(), String> {
    if reason.trim().is_empty() {
        return Err(format!(
            "{what} needs a reason = \"…\" for the install dialog"
        ));
    }
    if reason.len() > MAX_REASON_BYTES || reason.chars().any(char::is_control) {
        return Err(format!(
            "{what}: the reason must be one line of at most {MAX_REASON_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Checks `[[fyx_ui.terminal_programs]]`; the argument patterns are `[[fyx_ui.processes]]`'s.
pub fn check_programs(specs: &[TerminalProgramSpec]) -> Result<(), String> {
    if specs.len() > MAX_CREATED_TERMINALS * 4 {
        return Err("[[fyx_ui.terminal_programs]] has too many entries".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for spec in specs {
        let at = |why: String| format!("terminal program {:?}: {why}", spec.name);
        check_program_name(&spec.name).map_err(at)?;
        if !seen.insert(spec.name.to_ascii_lowercase()) {
            return Err(at("declared twice".into()));
        }
        check_reason("a terminal program", &spec.reason).map_err(at)?;
    }
    Ok(())
}

/// `scope` of `[fyx_ui.terminal_env]` (§B.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvScope {
    /// The extension's own terminals: its profiles and created terminals.
    #[default]
    Profiles,
    /// Also every new terminal: a second, stronger checkbox at install, unticked by default.
    AllNew,
}

/// `[fyx_ui.terminal_env]` as written.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalEnvSpec {
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub scope: EnvScope,
    pub reason: String,
}

/// Checked `terminal:env`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TerminalEnv {
    pub vars: Vec<(String, Template)>,
    pub scope: EnvScope,
}

impl TerminalEnv {
    /// The variables for a terminal, resolved; one whose template has no value is left out.
    pub fn resolve(&self, cx: &dyn TemplateContext) -> Vec<(String, String)> {
        self.vars
            .iter()
            .filter_map(|(name, value)| Some((name.clone(), value.resolve(cx)?)))
            .collect()
    }
}

/// Checks `[fyx_ui.terminal_env]` (§B.4). Names match `[A-Za-z_][A-Za-z0-9_]*` and are not
/// [`is_forbidden_terminal_env`], except that a name with a credential suffix (`_TOKEN`, …) may
/// carry exactly one listener's host token (`${listen.<name>.token}` with `token_in_env`): that
/// is Fyx's own per-run secret, not a credential of the user's (W2c's reading of §B.4 with
/// §C.6, 2026-11-19). The names [`CREDENTIAL_ENV_NAMES`] lists stay refused. Values have at most
/// [`MAX_ENV_VALUE_BYTES`], no NUL and no newline. `listener` is as for [`Template::parse`].
pub fn check_terminal_env(
    spec: TerminalEnvSpec,
    listener: &dyn Fn(&str) -> Option<bool>,
) -> Result<TerminalEnv, String> {
    if spec.vars.len() > MAX_ENV_VARS {
        return Err(format!(
            "[fyx_ui.terminal_env] has {} names, at most {MAX_ENV_VARS}",
            spec.vars.len()
        ));
    }
    check_reason("[fyx_ui.terminal_env]", &spec.reason)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut vars = Vec::new();
    for (name, value) in spec.vars {
        let at = |why: &str| format!("terminal:env name {name}: {why}");
        let mut bytes = name.bytes();
        if !bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(at("must be [A-Za-z_][A-Za-z0-9_]*"));
        }
        if !seen.insert(name.to_ascii_uppercase()) {
            return Err(at("declared twice in different letter case"));
        }
        if value.len() > MAX_ENV_VALUE_BYTES || value.contains(['\0', '\n', '\r']) {
            return Err(at(&format!(
                "the value has a NUL, a newline or more than {MAX_ENV_VALUE_BYTES} bytes"
            )));
        }
        let template =
            Template::parse(&value, TemplateUse::EnvValue, listener).map_err(|why| at(&why))?;
        if let Some(why) = terminal_env_refusal(&name) {
            let upper = name.to_ascii_uppercase();
            let host_token = template.is_only_token()
                && !CREDENTIAL_ENV_NAMES.contains(&upper.as_str())
                && !ACCOUNT_DIR_ENV_NAMES.contains(&upper.as_str())
                && !process::is_forbidden_env(&name)
                && terminal_env_refusal_without_credentials(&upper).is_none();
            if !host_token {
                return Err(at(why));
            }
        }
        vars.push((name, template));
    }
    Ok(TerminalEnv {
        vars,
        scope: spec.scope,
    })
}

/// [`terminal_env_refusal`] for the shell, host and endpoint names alone.
fn terminal_env_refusal_without_credentials(upper: &str) -> Option<()> {
    (SHELL_ENV_NAMES.contains(&upper)
        || SHELL_ENV_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
        || ENDPOINT_ENV_SUFFIXES
            .iter()
            .any(|suffix| upper.ends_with(suffix)))
    .then_some(())
}

// ---------------------------------------------------------------------------------------------
// Helpers

fn invalid(message: impl Into<String>) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message: message.into(),
    }
}

fn check_text(what: &str, text: &str, max: usize) -> Result<(), HostError> {
    if text.len() > max {
        return Err(invalid(format!("the {what} has more than {max} bytes")));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(format!("the {what} has a control character")));
    }
    Ok(())
}

/// `[a-z0-9-]{1,max}`.
fn is_id(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn check_program_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || name.contains(['/', '\\', '\0'])
        || name.chars().any(char::is_control)
    {
        return Err("the program must be a bare name (a worktree program uses `path`)".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[(&str, &str)]) -> TerminalEnvSpec {
        TerminalEnvSpec {
            vars: vars
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            scope: EnvScope::Profiles,
            reason: "lets the CLI find this extension".into(),
        }
    }

    fn listeners(name: &str) -> Option<bool> {
        match name {
            "ide" => Some(false),
            "hooks" => Some(true),
            _ => None,
        }
    }

    /// p5-security-0.4 P6: no `..` after `${package}` or `${account.<tool>}`.
    #[test]
    fn no_parent_after_package_or_account() {
        for text in [
            "${package}/../../.ssh",
            "${account.claude}/../work",
            "${package}\\..\\x",
        ] {
            assert!(
                Template::parse(text, TemplateUse::EnvValue, &listeners).is_err(),
                "{text}"
            );
        }
        for text in [
            "../${package}",
            "${package}/a..b/c",
            "${account.claude}/settings.json",
        ] {
            assert!(
                Template::parse(text, TemplateUse::EnvValue, &listeners).is_ok(),
                "{text}"
            );
        }
    }

    /// Plan 5.5 "done when": `PATH`, `HOME`, `CLAUDE_CONFIG_DIR` and `FOO_API_KEY` are refused, in
    /// any letter case, with the rest of §B.4 and §C.6.
    #[test]
    fn forbidden_terminal_env_names() {
        for name in [
            "PATH",
            "path",
            "HOME",
            "UserProfile",
            "CLAUDE_CONFIG_DIR",
            "claude_config_dir",
            "FOO_API_KEY",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "NODE_OPTIONS",
            "BASH_ENV",
            "PROMPT_COMMAND",
            "XDG_CONFIG_HOME",
            "ZDOTDIR",
            "SHELL",
            "TERM",
            "TERMINFO",
            "FYX_TERMINAL_TOKEN",
            "ZED_FOO",
            "CODEX_HOME",
            "OPENCODE_CONFIG_CONTENT",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "MY_SECRET",
            "DB_PASSWORD",
            "GIT_SSH_COMMAND",
            "RUSTC_WRAPPER",
            // p5-security-0.4 P1/P2.
            "PS1",
            "ps0",
            "PROMPT",
            "HISTFILE",
            "https_proxy",
            "NODE_EXTRA_CA_CERTS",
            "ANTHROPIC_BASE_URL",
            "OPENAI_API_BASE",
            "GIT_TERMINAL_PROMPT",
            "GOFLAGS",
            "CLASSPATH",
            "LUA_INIT_5_4",
            "GEM_HOME",
            "PIP_INDEX_URL",
            "UV_INDEX_URL",
        ] {
            assert!(is_forbidden_terminal_env(name), "{name}");
            let refused = check_terminal_env(env(&[(name, "x")]), &listeners);
            assert!(refused.is_err(), "{name}: {refused:?}");
        }
        for name in [
            "CLAUDE_CODE_SSE_PORT",
            "ENABLE_IDE_INTEGRATION",
            "CLAUDE_CODE_PLUGIN_DIRS",
            "GEMINI_CLI_IDE_SERVER_PORT",
            "OPENCODE_PORT",
            "TOKEN_COUNT",
        ] {
            assert!(!is_forbidden_terminal_env(name), "{name}");
        }
        assert!(is_account_env("anthropic_auth_token"));
        assert!(!is_account_env("CLAUDE_CODE_SSE_PORT"));
    }

    /// A `_TOKEN` name may carry exactly a listener's host token; a credential name never.
    #[test]
    fn host_token_exception() {
        let checked = check_terminal_env(
            env(&[("TOOL_IDE_TOKEN", "${listen.hooks.token}")]),
            &listeners,
        )
        .unwrap();
        assert_eq!(
            checked.vars,
            [(
                "TOOL_IDE_TOKEN".to_string(),
                Template(vec![TemplatePart::ListenToken("hooks".into())])
            )]
        );
        for (name, value) in [
            ("TOOL_IDE_TOKEN", "Bearer ${listen.hooks.token}"),
            ("TOOL_IDE_TOKEN", "literal"),
            ("GITHUB_TOKEN", "${listen.hooks.token}"),
            ("ANTHROPIC_AUTH_TOKEN", "${listen.hooks.token}"),
            ("FYX_TOKEN", "${listen.hooks.token}"),
            ("TOOL_IDE_TOKEN", "${listen.ide.token}"),
        ] {
            let refused = check_terminal_env(env(&[(name, value)]), &listeners);
            assert!(refused.is_err(), "{name}={value}: {refused:?}");
        }
    }

    #[test]
    fn env_values_and_templates() {
        let checked = check_terminal_env(
            env(&[
                ("CLAUDE_CODE_SSE_PORT", "${listen.ide.port}"),
                ("ENABLE_IDE_INTEGRATION", "true"),
                ("CLAUDE_CODE_PLUGIN_DIRS", "${package}/claude-plugin"),
                ("TOOL_HOOK", "$HOME is literal, ${relay} --channel x"),
                ("TOOL_DIR", "${account.claude}"),
                ("TOOL_IDE_PID", "${pid}"),
            ]),
            &listeners,
        )
        .unwrap();
        assert!(checked.vars.iter().any(|(_, value)| value.uses_package()));

        struct Cx;
        impl TemplateContext for Cx {
            fn package(&self) -> Option<String> {
                Some("/pkg/1.0".into())
            }
            fn listen_port(&self, name: &str) -> Option<u16> {
                (name == "ide").then_some(41234)
            }
            fn listen_token(&self, _: &str) -> Option<String> {
                None
            }
            fn relay(&self) -> Option<String> {
                Some("/fyx/fyx-relay".into())
            }
            fn account_dir(&self, _: &str) -> Option<String> {
                None
            }
        }
        let resolved: BTreeMap<_, _> = checked.resolve(&Cx).into_iter().collect();
        assert_eq!(resolved["CLAUDE_CODE_SSE_PORT"], "41234");
        assert_eq!(
            resolved["CLAUDE_CODE_PLUGIN_DIRS"],
            "/pkg/1.0/claude-plugin"
        );
        assert_eq!(
            resolved["TOOL_HOOK"],
            "$HOME is literal, /fyx/fyx-relay --channel x"
        );
        assert!(
            !resolved.contains_key("TOOL_DIR"),
            "no account profile: left out"
        );
        assert_eq!(resolved["TOOL_IDE_PID"], std::process::id().to_string());

        for (name, value) in [
            ("A", "${listen.nope.port}"),
            ("A", "${unknown}"),
            ("A", "${package"),
            ("A", "two\nlines"),
            ("A", "nul\0"),
            ("1A", "x"),
            ("A-B", "x"),
            ("", "x"),
        ] {
            let refused = check_terminal_env(env(&[(name, value)]), &listeners);
            assert!(refused.is_err(), "{name:?}={value:?}");
        }
        let refused = check_terminal_env(env(&[("A", "x"), ("a", "y")]), &listeners);
        assert!(refused.is_err(), "the same name in two letter cases");
        let long = "x".repeat(MAX_ENV_VALUE_BYTES + 1);
        assert!(check_terminal_env(env(&[("A", &long)]), &listeners).is_err());
        let many: Vec<_> = (0..=MAX_ENV_VARS)
            .map(|i| (format!("V{i}"), "x".to_string()))
            .collect();
        let many: Vec<_> = many.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        assert!(check_terminal_env(env(&many), &listeners).is_err());
        let mut no_reason = env(&[("A", "x")]);
        no_reason.reason = " ".into();
        assert!(check_terminal_env(no_reason, &listeners).is_err());
    }

    #[test]
    fn profiles() {
        let spec = |id: &str, args: &[&str]| TerminalProfileSpec {
            id: id.into(),
            name: "Claude Code".into(),
            icon: None,
            program: "claude".into(),
            path: None,
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: ProfileCwd::Worktree,
            account_tool: Some("claude".into()),
        };
        let checked =
            check_profiles(vec![spec("claude", &["--ide", "${package}/x", "${relay}"])]).unwrap();
        assert_eq!(checked[0].args.len(), 3);
        assert!(!checked[0].runs_any_code());
        let mut shell = spec("sh", &[]);
        shell.program = "bash".into();
        assert!(check_profiles(vec![shell]).unwrap()[0].runs_any_code());

        for bad in [
            vec![spec("Claude", &[])],
            vec![spec("", &[])],
            vec![spec("a", &[]), spec("a", &[])],
            vec![spec("a", &["${listen.ide.port}"])],
            vec![spec("a", &["${account.claude}"])],
        ] {
            assert!(check_profiles(bad.clone()).is_err(), "{bad:?}");
        }
        let mut path_program = spec("a", &[]);
        path_program.program = "bin/tool".into();
        assert!(check_profiles(vec![path_program]).is_err());
        let too_many: Vec<_> = (0..=MAX_PROFILES)
            .map(|i| spec(&format!("p{i}"), &[]))
            .collect();
        assert!(check_profiles(too_many).is_err());

        let parsed: TerminalProfileSpec = toml_like();
        assert_eq!(parsed.cwd, ProfileCwd::Home);
    }

    /// The manifest's form, through serde (JSON stands in for TOML here).
    fn toml_like() -> TerminalProfileSpec {
        serde_json::from_str(
            r#"{"id": "c", "name": "C", "icon": "terminal", "program": "c", "cwd": "home"}"#,
        )
        .unwrap()
    }

    #[test]
    fn programs_and_reasons() {
        let program = |name: &str, reason: &str| TerminalProgramSpec {
            name: name.into(),
            args: vec!["-c".into(), "notify=*".into(), "**".into()],
            path: None,
            reason: reason.into(),
        };
        assert!(check_programs(&[program("codex", "runs Codex in a panel")]).is_ok());
        assert!(check_programs(&[program("codex", "")]).is_err());
        assert!(check_programs(&[program("codex", "two\nlines")]).is_err());
        assert!(check_programs(&[program("codex", &"x".repeat(201))]).is_err());
        assert!(check_programs(&[program("codex", "a"), program("Codex", "b")]).is_err());
        assert!(check_programs(&[program("../codex", "a")]).is_err());
        assert_eq!(program("codex", "a").as_process().args.len(), 3);
    }

    #[test]
    fn status_and_requests() {
        let status = TerminalStatus {
            state: StatusState::Blocked,
            label: "Needs approval".into(),
            detail: None,
            progress: Some(40),
            attention: true,
        };
        assert!(status.check().is_ok());
        for bad in [
            TerminalStatus {
                label: "x".repeat(MAX_STATUS_LABEL_BYTES + 1),
                ..status.clone()
            },
            TerminalStatus {
                progress: Some(101),
                ..status.clone()
            },
            TerminalStatus {
                detail: Some("a\u{1b}[31m".into()),
                ..status
            },
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
        let request = PseudoRequest {
            title: "ssh".into(),
            cols: Some(80),
            rows: Some(24),
            in_panel: false,
        };
        assert!(request.check().is_ok());
        assert!(
            PseudoRequest {
                cols: Some(0),
                ..request
            }
            .check()
            .is_err()
        );
        assert_eq!(
            HostError::from(TerminalError::RateLimited).code,
            HostErrorCode::LimitExceeded
        );
        assert_eq!(
            HostError::from(TerminalError::Denied).code,
            HostErrorCode::Denied
        );
    }
}
