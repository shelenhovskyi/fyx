//! The pure rules of Fyx's security modes (design: `docs/fyx/security-modes.md`), without GPUI,
//! projects or settings, so they build and test in seconds: the spellings of a path
//! ([`paths`]), agent configs and a repository's `.fyx/security.toml` ([`RepoPolicy`]), Fyx's own
//! config and the account dirs, secret-looking variables, the reasons a denial gives, and what
//! each mode does with it. `fyx_guard` applies them to projects and settings and re-exports them.

pub mod paths;
mod policy;

use std::{
    cmp::Ordering,
    io,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use collections::HashMap;

pub use policy::{
    RepoPolicy, RepoState, SECRET_ENV_PATTERNS, SECURITY_FILE, env_name_matches, is_credential_url,
    is_within, protected_config, protected_config_rel, sanitize_for_log, secret_env_names,
    secret_env_vars,
};

/// Where the last settings the user accepted are kept, in Fyx's data dir, so a lowering they
/// declined does not apply silently at the next start (security-2 T3).
pub const CONFIRMED_FILE: &str = "fyx_security_confirmed.toml";

/// Writes [`CONFIRMED_FILE`] (and `agent_review`'s confirmed rules) off the main thread. Each change queues its contents and spawns a
/// [`Self::flush`]; flushes may run in any order and at the same time, so each one writes the
/// newest contents, not its own, and through a temporary file, so neither an older value nor half
/// a file is left behind (a stale "off" would let a later lowering through without a prompt).
#[derive(Debug)]
pub struct ConfirmedWriter {
    path: PathBuf,
    newest: Mutex<Option<Vec<u8>>>,
    writing: Mutex<()>,
}

impl ConfirmedWriter {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            newest: Mutex::new(None),
            writing: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Records `contents` as the newest; call on the thread that orders the changes.
    pub fn queue(&self, contents: Vec<u8>) {
        *self.newest.lock().unwrap_or_else(PoisonError::into_inner) = Some(contents);
    }

    /// Writes the newest queued contents, if a flush has not written them already.
    pub fn flush(&self) -> io::Result<()> {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(contents) = self
            .newest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return Ok(());
        };
        if let Some(directory) = self.path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let temporary = self
            .path
            .with_file_name(format!("{name}.{}.tmp", std::process::id()));
        std::fs::write(&temporary, contents)?;
        std::fs::rename(&temporary, &self.path)
    }
}

/// Why a path whose name Windows rewrites is not written.
pub const WINDOWS_ALIAS_DENIAL: &str = "its name uses a spelling Windows rewrites (a trailing dot \
     or space, or ':'), so it could name a protected file";
/// Why a path that a link takes out of the project is not written.
pub const LINK_OUT_DENIAL: &str = "it resolves outside the project through a link";
/// Why a file matching the global `private_files` is not read.
pub const GLOBAL_PRIVATE_DENIAL: &str = "it matches the global `private_files` setting";
/// Why a file matching the worktree's `private_files` is not read.
pub const WORKTREE_PRIVATE_DENIAL: &str = "it matches the worktree `private_files` setting";

/// Ordered by strength: `Off < Audit < Shield`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SecurityMode {
    Off,
    /// Shield as a dry run: log what would be blocked, block nothing.
    Audit,
    Shield,
}

impl SecurityMode {
    fn name(self) -> &'static str {
        match self {
            SecurityMode::Off => "off",
            SecurityMode::Audit => "audit",
            SecurityMode::Shield => "shield",
        }
    }
}

/// `fyx_security`: the mode and the secret-looking variables an agent terminal keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct SecurityConfig {
    pub mode: SecurityMode,
    pub allow_env: Vec<String>,
}

/// [`SecurityConfig`] as stored in [`CONFIRMED_FILE`].
#[derive(serde::Serialize, serde::Deserialize)]
struct ConfirmedFile {
    mode: String,
    allow_env: Vec<String>,
}

impl SecurityConfig {
    /// The contents of [`CONFIRMED_FILE`].
    pub fn to_confirmed_toml(&self) -> Option<Vec<u8>> {
        toml::to_string(&ConfirmedFile {
            mode: self.mode.name().into(),
            allow_env: self.allow_env.clone(),
        })
        .ok()
        .map(String::into_bytes)
    }

    /// An unreadable or unknown file counts as `shield` with no `allow_env`: whoever damaged it
    /// does not get a lower mode for it.
    pub fn from_confirmed_toml(bytes: &[u8]) -> Self {
        let file = std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| toml::from_str::<ConfirmedFile>(text).ok());
        let mode = match file.as_ref().map(|file| file.mode.as_str()) {
            Some("off") => SecurityMode::Off,
            Some("audit") => SecurityMode::Audit,
            _ => SecurityMode::Shield,
        };
        Self {
            mode,
            allow_env: file.map(|file| file.allow_env).unwrap_or_default(),
        }
    }

    /// Whether `requested` protects less than `self`. A higher mode protects more whatever its
    /// `allow_env`, since a lower mode blanks nothing.
    pub fn lowered_by(&self, requested: &SecurityConfig) -> bool {
        match requested.mode.cmp(&self.mode) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => {
                self.mode != SecurityMode::Off
                    && requested
                        .allow_env
                        .iter()
                        .any(|entry| !self.allow_env.contains(entry))
            }
        }
    }

    /// For the log and the lowering prompt: `"shield"`, or `"audit" with allow_env [...]`.
    pub fn describe(&self) -> String {
        let mode = self.mode.name();
        if self.allow_env.is_empty() || self.mode == SecurityMode::Off {
            format!("\"{mode}\"")
        } else {
            format!("\"{mode}\" with allow_env {:?}", self.allow_env)
        }
    }
}

/// Applies `mode` to a denial: shield turns it into the message for the agent, audit only logs it.
/// `action` is described only for a denial.
pub fn enforce(
    mode: SecurityMode,
    action: impl FnOnce() -> String,
    denial: Option<String>,
) -> Result<(), String> {
    let Some(reason) = denial else {
        return Ok(());
    };
    match mode {
        SecurityMode::Off => Ok(()),
        SecurityMode::Audit => {
            log::warn!(
                "fyx_guard audit: would block {}: {}",
                sanitize_for_log(&action()),
                sanitize_for_log(&reason)
            );
            Ok(())
        }
        SecurityMode::Shield => {
            let action = action();
            log::warn!(
                "fyx_guard: blocked {}: {}",
                sanitize_for_log(&action),
                sanitize_for_log(&reason)
            );
            Err(format!(
                "Fyx's security shield blocked {action}: {reason}. Do not retry or work around \
                 this; ask the user if the task needs it."
            ))
        }
    }
}

/// Why a protected agent config (`rule`, from [`protected_config`] or Fyx's own config dir) is
/// not written.
pub fn read_only_config_denial(rule: &str) -> String {
    format!("agent configs ({rule}) are read-only for agents")
}

/// `path` with a leading `~` replaced by the home dir.
pub fn expand_home(path: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => util::paths::home_dir().join(rest),
        Err(_) => path.to_path_buf(),
    }
}

/// Fyx's own config directory: the user settings file sets the mode, and `tasks.json` and
/// `keymap.json` there run commands, like `.zed/` does for a project. Also [`CONFIRMED_FILE`]
/// in `data_dir`. The rule, for [`read_only_config_denial`].
pub fn user_config_rule(path: &Path, config_dir: &Path, data_dir: &Path) -> Option<String> {
    let path = expand_home(path);
    if is_within(&path, config_dir) {
        return Some(format!("{}/", config_dir.display()));
    }
    let confirmed = data_dir.join(CONFIRMED_FILE);
    is_within(&path, &confirmed).then(|| confirmed.display().to_string())
}

/// One account of `fyx.accounts`: the tool, the account's name and its dir (`~` expanded).
#[derive(Clone, Copy, Debug)]
pub struct AccountDir<'a> {
    pub tool: &'a str,
    pub name: &'a str,
    pub dir: &'a Path,
}

/// An account dir of `fyx.accounts` (design wit-0.4 §C.6, rule 5): only the tool itself signs in
/// and keeps its credentials there, so agents and extensions neither read nor write in it. The
/// reason names the account, never the dir: an extension sees only an account's name.
pub fn account_dir_denial<'a>(
    path: &Path,
    accounts: impl IntoIterator<Item = AccountDir<'a>>,
) -> Option<String> {
    let path = expand_home(path);
    accounts
        .into_iter()
        .find(|account| is_within(&path, account.dir))
        .map(|account| {
            format!(
                "it is in the {} account dir of \"{}\", which only the tool itself uses",
                account.tool, account.name
            )
        })
}

/// Blanks the secret-looking variables of an agent terminal's `env` not in `allow`, under shield;
/// audit only logs them. `inherited` is Fyx's own environment, with values where they are UTF-8:
/// a value that is a credential URL is secret whatever the variable is called. Returns the
/// secret names.
pub fn scrub(
    env: &mut HashMap<String, String>,
    inherited: &[(String, Option<String>)],
    mode: SecurityMode,
    allow: &[String],
) -> Vec<String> {
    let vars = env
        .iter()
        .map(|(name, value)| (name, Some(value.as_str())))
        .chain(
            inherited
                .iter()
                .map(|(name, value)| (name, value.as_deref())),
        );
    let secret = secret_env_vars(vars, allow);
    if secret.is_empty() {
        return secret;
    }
    match mode {
        SecurityMode::Off => {}
        SecurityMode::Audit => log::warn!(
            "fyx_guard audit: would blank in an agent terminal: {}",
            secret.join(", ")
        ),
        SecurityMode::Shield => {
            for name in &secret {
                env.insert(name.clone(), String::new());
            }
            log::info!(
                "fyx_guard: blanked in an agent terminal: {}",
                secret.join(", ")
            );
        }
    }
    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforce_blocks_only_in_shield() {
        let denial = || Some("it matches `private_files`".to_string());
        let action = || "a read".to_string();
        assert!(enforce(SecurityMode::Off, action, denial()).is_ok());
        assert!(enforce(SecurityMode::Audit, action, denial()).is_ok());
        let error = enforce(SecurityMode::Shield, action, denial()).unwrap_err();
        assert!(error.contains("private_files"), "{error}");
        assert!(enforce(SecurityMode::Shield, action, None).is_ok());
        assert!(
            enforce(
                SecurityMode::Shield,
                || panic!("the action is described only for a denial"),
                None
            )
            .is_ok()
        );
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn scrub_blanks_in_shield_and_only_logs_in_audit() {
        let original = env(&[
            ("PATH", "/usr/bin"),
            ("GITHUB_TOKEN", "ghp_x"),
            ("OPENAI_API_KEY", "sk-x"),
            ("SSH_AUTH_SOCK", "/tmp/agent"),
        ]);
        let inherited = [
            ("AWS_SECRET_ACCESS_KEY".to_string(), Some("x".to_string())),
            ("HOME".to_string(), Some("/home/u".to_string())),
            (
                "CI_REPOSITORY".to_string(),
                Some("https://ci:glpat-x@gitlab.example/a.git".to_string()),
            ),
        ];

        let mut audited = original.clone();
        let secret = scrub(&mut audited, &inherited, SecurityMode::Audit, &[]);
        assert_eq!(audited, original);
        assert_eq!(
            secret,
            [
                "AWS_SECRET_ACCESS_KEY",
                "CI_REPOSITORY",
                "GITHUB_TOKEN",
                "OPENAI_API_KEY",
                "SSH_AUTH_SOCK"
            ]
        );

        let mut shielded = original;
        scrub(
            &mut shielded,
            &inherited,
            SecurityMode::Shield,
            &["GITHUB_TOKEN".to_string()],
        );
        assert_eq!(shielded["PATH"], "/usr/bin");
        assert_eq!(shielded["GITHUB_TOKEN"], "ghp_x");
        assert_eq!(shielded["OPENAI_API_KEY"], "");
        assert_eq!(shielded["SSH_AUTH_SOCK"], "");
        // Only inherited by the PTY from Fyx's own environment: overridden with an empty value.
        assert_eq!(shielded["AWS_SECRET_ACCESS_KEY"], "");
        assert!(!shielded.contains_key("HOME"));
    }

    fn config(mode: SecurityMode, allow_env: &[&str]) -> SecurityConfig {
        SecurityConfig {
            mode,
            allow_env: allow_env.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn lowering_is_a_lower_mode_or_more_allowed_variables() {
        use SecurityMode::*;
        let shield = config(Shield, &["A"]);
        assert!(shield.lowered_by(&config(Audit, &[])));
        assert!(shield.lowered_by(&config(Shield, &["A", "B"])));
        assert!(!shield.lowered_by(&config(Shield, &[])));
        assert!(!config(Audit, &[]).lowered_by(&config(Shield, &["A", "B"])));
        // Off blanks nothing, so allowing more there lowers nothing.
        assert!(!config(Off, &[]).lowered_by(&config(Off, &["A"])));
        assert_eq!(shield.describe(), "\"shield\" with allow_env [\"A\"]");
        assert_eq!(config(Off, &["A"]).describe(), "\"off\"");
    }

    #[test]
    fn confirmed_files_round_trip_and_fail_closed() {
        let audit = config(SecurityMode::Audit, &["GITHUB_TOKEN"]);
        let bytes = audit.to_confirmed_toml().unwrap();
        assert_eq!(SecurityConfig::from_confirmed_toml(&bytes), audit);
        for damaged in [&b"mode = \"of\"\nallow_env = []"[..], b"\xff", b"mode = 1"] {
            assert_eq!(
                SecurityConfig::from_confirmed_toml(damaged).mode,
                SecurityMode::Shield
            );
        }
    }

    #[test]
    fn confirmed_writer_keeps_the_newest_whatever_order_flushes_run_in() {
        let directory =
            std::env::temp_dir().join(format!("fyx-guard-core-writer-{}", std::process::id()));
        let path = directory.join(CONFIRMED_FILE);
        std::fs::remove_file(&path).ok();
        let writer = ConfirmedWriter::new(path.clone());
        let off = config(SecurityMode::Off, &[]).to_confirmed_toml().unwrap();
        let shield = config(SecurityMode::Shield, &[])
            .to_confirmed_toml()
            .unwrap();
        // Two changes, then their flushes: the first flush writes the newest, the second
        // finds nothing left and must not write the older value it was spawned for.
        writer.queue(off);
        writer.queue(shield.clone());
        writer.flush().unwrap();
        writer.flush().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), shield);
        let leftovers: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path() != path)
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn user_config_and_account_dirs() {
        let root = if cfg!(windows) { "C:\\u" } else { "/u" };
        let config_dir = Path::new(root).join("config");
        let data_dir = Path::new(root).join("data");
        assert_eq!(
            user_config_rule(&config_dir.join("settings.json"), &config_dir, &data_dir),
            Some(format!("{}/", config_dir.display()))
        );
        assert!(user_config_rule(&data_dir.join(CONFIRMED_FILE), &config_dir, &data_dir).is_some());
        assert!(user_config_rule(&data_dir.join("other"), &config_dir, &data_dir).is_none());
        assert_eq!(
            read_only_config_denial(".claude/"),
            "agent configs (.claude/) are read-only for agents"
        );

        let dir = Path::new(root).join("accounts").join("work");
        let accounts = [AccountDir {
            tool: "claude",
            name: "work",
            dir: &dir,
        }];
        assert_eq!(
            account_dir_denial(&dir.join(".credentials.json"), accounts).as_deref(),
            Some("it is in the claude account dir of \"work\", which only the tool itself uses")
        );
        assert!(account_dir_denial(&Path::new(root).join("accounts"), accounts).is_none());
    }
}
