//! Secrets for agents by reference (plan 7.4). The values live in the system credential store
//! (Keychain on macOS, Credential Manager on Windows, the Secret Service on Linux) through gpui's
//! credentials API; the agent only ever writes and reads `secret://name`.
//!
//! When an agent starts a terminal command that refers to a secret, the user is asked once for that
//! command. If they allow it, the value goes into the process environment only: a reference in the
//! command becomes the shell's reference to `FYX_SECRET_<NAME>`, a reference in an environment
//! value the agent passed is replaced in place. The output the agent reads back has the value, and
//! its base64, hex and URL encodings, replaced by `secret://name`. Every use is journaled without
//! its value.
//!
//! Limits: a reference inside single quotes is not expanded by the shell, and a value the command
//! transforms in another way (split, hashed, encrypted) is not recognized in its output.

use std::{
    collections::{BTreeSet, HashMap},
    io::Write as _,
    path::PathBuf,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use acp_thread::fyx_secrets::{AgentProcess, AgentSecrets, SECRET_SCHEME, set_agent_secrets};
use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext as _, Context, Entity, Global, PromptLevel, Task, WeakEntity};
use serde::{Deserialize, Serialize};

/// Where secret values are kept.
pub trait SecretStore: 'static {
    fn read(&self, name: &str, cx: &App) -> Task<Result<Option<String>>>;
    fn write(&self, name: &str, value: &str, cx: &App) -> Task<Result<()>>;
    fn delete(&self, name: &str, cx: &App) -> Task<Result<()>>;
}

/// Asks the user whether a command may use secrets.
pub trait SecretApprover: 'static {
    fn approve(&self, request: SecretRequest, cx: &mut App) -> Task<bool>;
}

/// One command that wants secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretRequest {
    pub session_id: Option<acp_v2::SessionId>,
    /// The secrets it refers to that exist, sorted.
    pub names: Vec<String>,
    /// The command as the agent wrote it, with references, never values.
    pub command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretUseOutcome {
    Allowed,
    Denied,
    /// No secret has that name.
    Missing,
}

/// A journal entry: who used which secret, never its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretUse {
    pub unix_seconds: u64,
    pub session: Option<String>,
    pub secret: String,
    pub outcome: SecretUseOutcome,
}

/// The reserved entry of the store that lists the secret names (not a valid secret name).
const INDEX: &str = ".index";
/// Values shorter than this are not redacted: they would garble unrelated output.
const MIN_REDACTED_LEN: usize = 6;
/// Journal entries kept in memory.
const MAX_JOURNAL: usize = 512;

/// The system credential store, through gpui.
struct CredentialsStore;

fn credentials_url(name: &str) -> String {
    format!("fyx-secret://{name}")
}

impl SecretStore for CredentialsStore {
    fn read(&self, name: &str, cx: &App) -> Task<Result<Option<String>>> {
        let read = cx.read_credentials(&credentials_url(name));
        cx.background_spawn(async move {
            match read.await? {
                Some((_, bytes)) => Ok(Some(
                    String::from_utf8(bytes).context("a secret is not UTF-8")?,
                )),
                None => Ok(None),
            }
        })
    }

    fn write(&self, name: &str, value: &str, cx: &App) -> Task<Result<()>> {
        cx.write_credentials(&credentials_url(name), "fyx", value.as_bytes())
    }

    fn delete(&self, name: &str, cx: &App) -> Task<Result<()>> {
        cx.delete_credentials(&credentials_url(name))
    }
}

/// The in-app prompt of the active window (or any window). Without a window, nothing is allowed.
struct WindowApprover;

impl SecretApprover for WindowApprover {
    fn approve(&self, request: SecretRequest, cx: &mut App) -> Task<bool> {
        let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
            return Task::ready(false);
        };
        let message = format!("The agent wants to use {}", describe_names(&request.names));
        let mut command = request.command;
        if command.chars().count() > 600 {
            command = command.chars().take(600).collect::<String>() + "…";
        }
        let detail = format!(
            "Only this command gets the values, in its environment. The agent sees only the \
             references, also in the command's output.\n\n{command}"
        );
        let answer = window.update(cx, |_, window, cx| {
            window.prompt(
                PromptLevel::Warning,
                &message,
                Some(&detail),
                &["Allow once", "Deny"],
                cx,
            )
        });
        match answer {
            Ok(answer) => cx.background_spawn(async move { answer.await == Ok(0) }),
            Err(error) => {
                log::error!("fyx: could not ask about secrets: {error:#}");
                Task::ready(false)
            }
        }
    }
}

fn describe_names(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("{SECRET_SCHEME}{name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub struct SecretsVault {
    store: Rc<dyn SecretStore>,
    approver: Rc<dyn SecretApprover>,
    /// The values used in this run, by name: what output is redacted for.
    used: HashMap<String, String>,
    /// Redaction patterns, longest first: (encoded value, name).
    patterns: Vec<(String, String)>,
    journal: Vec<SecretUse>,
    journal_path: Option<PathBuf>,
}

struct GlobalSecretsVault(Entity<SecretsVault>);

impl Global for GlobalSecretsVault {}

/// Installs the vault over the system credential store and the window prompt.
pub fn init(cx: &mut App) {
    let journal_path = paths::data_dir()
        .join("agent_review")
        .join("secret_uses.jsonl");
    install(
        Rc::new(CredentialsStore),
        Rc::new(WindowApprover),
        Some(journal_path),
        cx,
    );
}

/// Installs a vault with the given store and approver (tests use fakes).
pub fn install(
    store: Rc<dyn SecretStore>,
    approver: Rc<dyn SecretApprover>,
    journal_path: Option<PathBuf>,
    cx: &mut App,
) -> Entity<SecretsVault> {
    let vault = cx.new(|_| SecretsVault {
        store,
        approver,
        used: HashMap::new(),
        patterns: Vec::new(),
        journal: Vec::new(),
        journal_path,
    });
    cx.set_global(GlobalSecretsVault(vault.clone()));
    set_agent_secrets(Rc::new(VaultHook(vault.downgrade())), cx);
    vault
}

impl SecretsVault {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalSecretsVault>()
            .map(|global| global.0.clone())
    }

    /// The uses so far, oldest first.
    pub fn journal(&self) -> &[SecretUse] {
        &self.journal
    }

    /// The names of the stored secrets, sorted.
    pub fn names(&self, cx: &App) -> Task<Result<Vec<String>>> {
        let read = self.store.read(INDEX, cx);
        cx.background_spawn(async move { Ok(parse_index(read.await?.as_deref())) })
    }

    /// Stores `value` as `secret://name`, replacing any value it had.
    pub fn add_secret(&self, name: &str, value: &str, cx: &App) -> Task<Result<()>> {
        if let Err(error) = check_name(name) {
            return Task::ready(Err(error));
        }
        if value.is_empty() {
            return Task::ready(Err(anyhow::anyhow!("a secret cannot be empty")));
        }
        let write = self.store.write(name, value, cx);
        self.update_index(name.to_string(), true, write, cx)
    }

    pub fn remove_secret(&mut self, name: &str, cx: &App) -> Task<Result<()>> {
        if self.used.remove(name).is_some() {
            self.rebuild_patterns();
        }
        let delete = self.store.delete(name, cx);
        self.update_index(name.to_string(), false, delete, cx)
    }

    fn update_index(
        &self,
        name: String,
        present: bool,
        change: Task<Result<()>>,
        cx: &App,
    ) -> Task<Result<()>> {
        let store = self.store.clone();
        cx.spawn(async move |cx| {
            change.await?;
            let mut names = parse_index(cx.update(|cx| store.read(INDEX, cx)).await?.as_deref())
                .into_iter()
                .collect::<BTreeSet<_>>();
            let changed = if present {
                names.insert(name)
            } else {
                names.remove(&name)
            };
            if changed {
                let text = serde_json::to_string(&names)?;
                cx.update(|cx| store.write(INDEX, &text, cx)).await?;
            }
            Ok(())
        })
    }

    fn resolve(&self, process: AgentProcess, cx: &mut Context<Self>) -> Task<AgentProcess> {
        let names = referenced_names(&process);
        if names.is_empty() {
            return Task::ready(process);
        }
        let store = self.store.clone();
        let approver = self.approver.clone();
        cx.spawn(async move |vault, cx| {
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for name in names {
                match cx.update(|cx| store.read(&name, cx)).await {
                    Ok(Some(value)) if !value.is_empty() => found.push((name, value)),
                    Ok(_) => missing.push(name),
                    Err(error) => {
                        log::error!("fyx: could not read {SECRET_SCHEME}{name}: {error:#}");
                        missing.push(name);
                    }
                }
            }
            let session = process.session_id.as_ref().map(|id| id.to_string());
            let mut uses = missing
                .iter()
                .map(|name| (name.clone(), SecretUseOutcome::Missing))
                .collect::<Vec<_>>();
            let mut process = process;
            if !found.is_empty() {
                let request = SecretRequest {
                    session_id: process.session_id.clone(),
                    names: found.iter().map(|(name, _)| name.clone()).collect(),
                    command: display_command(&process),
                };
                let allowed = cx.update(|cx| approver.approve(request, cx)).await;
                let outcome = if allowed {
                    SecretUseOutcome::Allowed
                } else {
                    SecretUseOutcome::Denied
                };
                uses.extend(found.iter().map(|(name, _)| (name.clone(), outcome)));
                if allowed {
                    process = substitute(process, &found);
                } else {
                    found.clear();
                }
            }
            vault
                .update(cx, |vault, cx| {
                    vault.remember(found);
                    vault.record(session, uses, cx);
                })
                .ok();
            process
        })
    }

    fn remember(&mut self, found: Vec<(String, String)>) {
        if found.is_empty() {
            return;
        }
        self.used.extend(found);
        self.rebuild_patterns();
    }

    fn rebuild_patterns(&mut self) {
        let mut patterns = self
            .used
            .iter()
            .flat_map(|(name, value)| {
                encodings(value)
                    .into_iter()
                    .map(move |encoded| (encoded, name.clone()))
            })
            .filter(|(encoded, _)| encoded.len() >= MIN_REDACTED_LEN)
            .collect::<Vec<_>>();
        patterns
            .sort_by(|(left, _), (right, _)| right.len().cmp(&left.len()).then(left.cmp(right)));
        patterns.dedup_by(|(left, _), (right, _)| left == right);
        self.patterns = patterns;
    }

    fn record(
        &mut self,
        session: Option<String>,
        uses: Vec<(String, SecretUseOutcome)>,
        cx: &mut Context<Self>,
    ) {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let entries = uses
            .into_iter()
            .map(|(secret, outcome)| SecretUse {
                unix_seconds,
                session: session.clone(),
                secret,
                outcome,
            })
            .collect::<Vec<_>>();
        for entry in &entries {
            log::info!(
                "fyx: {SECRET_SCHEME}{} {:?} for session {}",
                entry.secret,
                entry.outcome,
                entry.session.as_deref().unwrap_or("-")
            );
        }
        if let Some(path) = self.journal_path.clone() {
            let entries = entries.clone();
            cx.background_spawn(async move {
                if let Err(error) = append_journal(&path, &entries) {
                    log::error!("fyx: could not journal secret uses: {error:#}");
                }
            })
            .detach();
        }
        self.journal.extend(entries);
        let excess = self.journal.len().saturating_sub(MAX_JOURNAL);
        self.journal.drain(..excess);
    }

    /// `text` with every used value and its encodings replaced by its reference.
    pub fn redact(&self, text: &str) -> Option<String> {
        redact_with(text, &self.patterns)
    }
}

fn append_journal(path: &std::path::Path, entries: &[SecretUse]) -> Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for entry in entries {
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
    }
    Ok(())
}

struct VaultHook(WeakEntity<SecretsVault>);

impl AgentSecrets for VaultHook {
    fn resolve(&self, process: AgentProcess, cx: &mut App) -> Task<AgentProcess> {
        match self.0.upgrade() {
            Some(vault) => vault.update(cx, |vault, cx| vault.resolve(process, cx)),
            None => Task::ready(process),
        }
    }

    fn redact(&self, text: &str, cx: &App) -> Option<String> {
        self.0.upgrade()?.read(cx).redact(text)
    }
}

/// Whether `name` can follow `secret://`: 1 to 64 lowercase letters, digits, `-` and `_`.
pub fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 || !name.chars().all(is_name_character) {
        bail!("a secret name has 1 to 64 lowercase letters, digits, '-' and '_'");
    }
    Ok(())
}

fn is_name_character(character: char) -> bool {
    character.is_ascii_lowercase() || character.is_ascii_digit() || matches!(character, '-' | '_')
}

fn parse_index(text: Option<&str>) -> Vec<String> {
    let Some(text) = text else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(text)
        .inspect_err(|error| log::error!("fyx: the secret index is unreadable: {error}"))
        .unwrap_or_default()
}

/// The names referred to in `text`, in order, with repeats.
pub fn references(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(SECRET_SCHEME) {
        let after = &rest[start + SECRET_SCHEME.len()..];
        let length = after
            .find(|character| !is_name_character(character))
            .unwrap_or(after.len());
        if length > 0 && length <= 64 {
            names.push(after[..length].to_string());
        }
        rest = &after[length..];
    }
    names
}

fn referenced_names(process: &AgentProcess) -> Vec<String> {
    std::iter::once(&process.command)
        .chain(&process.args)
        .chain(process.env.values())
        .flat_map(|text| references(text))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn display_command(process: &AgentProcess) -> String {
    std::iter::once(process.command.as_str())
        .chain(process.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The environment variable that carries `secret://name` into a command.
pub fn env_var_name(name: &str) -> String {
    format!("FYX_SECRET_{}", name.to_ascii_uppercase().replace('-', "_"))
}

/// Replaces each reference to a found secret in `text`.
fn replace_references(
    text: &str,
    found: &[(String, String)],
    with: impl Fn(&str, &str) -> String,
) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(SECRET_SCHEME) {
        result.push_str(&rest[..start]);
        let after = &rest[start + SECRET_SCHEME.len()..];
        let length = after
            .find(|character| !is_name_character(character))
            .unwrap_or(after.len());
        let name = &after[..length];
        match found.iter().find(|(found_name, _)| found_name == name) {
            Some((name, value)) => result.push_str(&with(name, value)),
            None => result.push_str(&rest[start..start + SECRET_SCHEME.len() + length]),
        }
        rest = &after[length..];
    }
    result.push_str(rest);
    result
}

/// References in the command become shell variables set in the environment; references in
/// environment values become the values.
fn substitute(mut process: AgentProcess, found: &[(String, String)]) -> AgentProcess {
    let env_reference = process.env_reference.clone();
    let in_command = |name: &str, _: &str| env_reference(&env_var_name(name));
    let mut used_in_command = BTreeSet::new();
    for text in std::iter::once(&process.command).chain(&process.args) {
        used_in_command.extend(references(text));
    }
    process.command = replace_references(&process.command, found, in_command);
    process.args = process
        .args
        .iter()
        .map(|arg| replace_references(arg, found, in_command))
        .collect();
    for value in process.env.values_mut() {
        if value.contains(SECRET_SCHEME) {
            *value = replace_references(value, found, |_, value| value.to_string());
        }
    }
    for (name, value) in found {
        if used_in_command.contains(name) {
            process.env.insert(env_var_name(name), value.clone());
        }
    }
    process
}

/// The value and the encodings a command is likely to print it in.
fn encodings(value: &str) -> Vec<String> {
    let bytes = value.as_bytes();
    let standard = base64(bytes, STANDARD_ALPHABET);
    let url_safe = base64(bytes, URL_SAFE_ALPHABET);
    let mut encoded = vec![
        value.to_string(),
        standard.trim_end_matches('=').to_string(),
        standard,
        url_safe.trim_end_matches('=').to_string(),
        hex(bytes, false),
        hex(bytes, true),
        percent_encode(value),
    ];
    encoded.sort();
    encoded.dedup();
    encoded
}

const STANDARD_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64(bytes: &[u8], alphabet: &[u8; 64]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        let indices = [
            first >> 2,
            ((first & 0b11) << 4) | (second >> 4),
            ((second & 0b1111) << 2) | (third >> 6),
            third & 0b11_1111,
        ];
        for (position, index) in indices.into_iter().enumerate() {
            if position <= chunk.len() {
                encoded.push(char::from(alphabet[usize::from(index)]));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

fn hex(bytes: &[u8], upper: bool) -> String {
    let digits: &[u8; 16] = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(digits[usize::from(byte >> 4)]));
        encoded.push(char::from(digits[usize::from(byte & 0xf)]));
    }
    encoded
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn redact_with(text: &str, patterns: &[(String, String)]) -> Option<String> {
    if !patterns
        .iter()
        .any(|(pattern, _)| text.contains(pattern.as_str()))
    {
        return None;
    }
    let mut redacted = text.to_string();
    for (pattern, name) in patterns {
        if redacted.contains(pattern.as_str()) {
            redacted = redacted.replace(pattern.as_str(), &format!("{SECRET_SCHEME}{name}"));
        }
    }
    Some(redacted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use gpui::TestAppContext;

    /// An in-memory store: tests never touch the system credential store.
    #[derive(Default)]
    struct FakeStore(RefCell<HashMap<String, String>>);

    impl SecretStore for FakeStore {
        fn read(&self, name: &str, _: &App) -> Task<Result<Option<String>>> {
            Task::ready(Ok(self.0.borrow().get(name).cloned()))
        }

        fn write(&self, name: &str, value: &str, _: &App) -> Task<Result<()>> {
            self.0.borrow_mut().insert(name.into(), value.into());
            Task::ready(Ok(()))
        }

        fn delete(&self, name: &str, _: &App) -> Task<Result<()>> {
            self.0.borrow_mut().remove(name);
            Task::ready(Ok(()))
        }
    }

    struct FakeApprover {
        allow: bool,
        requests: RefCell<Vec<SecretRequest>>,
    }

    impl SecretApprover for FakeApprover {
        fn approve(&self, request: SecretRequest, _: &mut App) -> Task<bool> {
            self.requests.borrow_mut().push(request);
            Task::ready(self.allow)
        }
    }

    const FAKE_TOKEN: &str = "ghp_FAKEfakeFAKEfake0123456789notreal";

    async fn setup(
        allow: bool,
        cx: &mut TestAppContext,
    ) -> (Entity<SecretsVault>, Rc<FakeApprover>) {
        let approver = Rc::new(FakeApprover {
            allow,
            requests: RefCell::default(),
        });
        let vault =
            cx.update(|cx| install(Rc::new(FakeStore::default()), approver.clone(), None, cx));
        cx.update(|cx| vault.read(cx).add_secret("github", FAKE_TOKEN, cx))
            .await
            .unwrap();
        (vault, approver)
    }

    fn process(command: &str, env: &[(&str, &str)]) -> AgentProcess {
        AgentProcess {
            session_id: Some(acp_v2::SessionId::new("session-1")),
            command: command.into(),
            args: Vec::new(),
            env: env
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            env_reference: Rc::new(|name| format!("${{{name}}}")),
        }
    }

    async fn resolve(
        vault: &Entity<SecretsVault>,
        process: AgentProcess,
        cx: &mut TestAppContext,
    ) -> AgentProcess {
        let hook = VaultHook(vault.downgrade());
        cx.update(|cx| hook.resolve(process, cx)).await
    }

    #[gpui::test]
    async fn an_allowed_secret_reaches_only_the_environment(cx: &mut TestAppContext) {
        let (vault, approver) = setup(true, cx).await;
        let command =
            "curl -H \"Authorization: Bearer secret://github\" https://api.github.com/user";
        let resolved = resolve(
            &vault,
            process(
                command,
                &[("GH_TOKEN", "secret://github"), ("OTHER", "secret://none")],
            ),
            cx,
        )
        .await;
        assert_eq!(
            resolved.command,
            "curl -H \"Authorization: Bearer ${FYX_SECRET_GITHUB}\" https://api.github.com/user"
        );
        assert_eq!(resolved.env["FYX_SECRET_GITHUB"], FAKE_TOKEN);
        assert_eq!(resolved.env["GH_TOKEN"], FAKE_TOKEN);
        assert_eq!(resolved.env["OTHER"], "secret://none", "unknown names stay");
        {
            let requests = approver.requests.borrow();
            assert_eq!(requests.len(), 1, "one prompt per command");
            assert_eq!(requests[0].names, vec!["github"]);
            assert_eq!(requests[0].command, command, "the prompt shows references");
        }

        // `printenv` and an encoded echo come back as references.
        let output = format!(
            "GH_TOKEN={FAKE_TOKEN}\nFYX_SECRET_GITHUB={FAKE_TOKEN}\n{}\n",
            base64(FAKE_TOKEN.as_bytes(), STANDARD_ALPHABET)
        );
        vault.read_with(cx, |vault, _| {
            assert_eq!(
                vault.redact(&output).as_deref(),
                Some(
                    "GH_TOKEN=secret://github\nFYX_SECRET_GITHUB=secret://github\nsecret://github\n"
                )
            );
            let journal = vault
                .journal()
                .iter()
                .map(|entry| {
                    (
                        entry.secret.as_str(),
                        entry.outcome,
                        entry.session.as_deref(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                journal,
                vec![
                    ("none", SecretUseOutcome::Missing, Some("session-1")),
                    ("github", SecretUseOutcome::Allowed, Some("session-1")),
                ]
            );
            let serialized = serde_json::to_string(vault.journal()).unwrap();
            assert!(
                !serialized.contains(FAKE_TOKEN),
                "the journal holds no values"
            );
        });
    }

    #[gpui::test]
    async fn a_denied_secret_stays_a_reference(cx: &mut TestAppContext) {
        let (vault, approver) = setup(false, cx).await;
        let resolved = resolve(&vault, process("echo secret://github", &[]), cx).await;
        assert_eq!(resolved.command, "echo secret://github");
        assert!(!resolved.env.contains_key("FYX_SECRET_GITHUB"));
        assert_eq!(approver.requests.borrow().len(), 1);
        vault.read_with(cx, |vault, _| {
            assert_eq!(vault.journal()[0].outcome, SecretUseOutcome::Denied);
            assert_eq!(
                vault.redact(FAKE_TOKEN),
                None,
                "a value never used is unknown"
            );
        });
    }

    #[gpui::test]
    async fn commands_without_references_are_not_asked_about(cx: &mut TestAppContext) {
        let (vault, approver) = setup(true, cx).await;
        let resolved = resolve(&vault, process("ls -la", &[("PATH", "/bin")]), cx).await;
        assert_eq!(resolved.command, "ls -la");
        assert!(approver.requests.borrow().is_empty());
        assert!(vault.read_with(cx, |vault, _| vault.journal().is_empty()));
    }

    #[gpui::test]
    async fn the_index_lists_added_secrets(cx: &mut TestAppContext) {
        let (vault, _) = setup(true, cx).await;
        cx.update(|cx| vault.read(cx).add_secret("npm", "fake-npm-value", cx))
            .await
            .unwrap();
        let names = cx.update(|cx| vault.read(cx).names(cx)).await.unwrap();
        assert_eq!(names, vec!["github", "npm"]);
        cx.update(|cx| vault.update(cx, |vault, cx| vault.remove_secret("github", cx)))
            .await
            .unwrap();
        let names = cx.update(|cx| vault.read(cx).names(cx)).await.unwrap();
        assert_eq!(names, vec!["npm"]);
        let bad = cx.update(|cx| vault.read(cx).add_secret("Bad Name", "x", cx));
        assert!(bad.await.is_err());
    }

    #[test]
    fn references_are_found_in_text() {
        assert_eq!(
            references(
                "curl -H \"Authorization: Bearer secret://github\" secret://x-y_1/z secret://"
            ),
            vec!["github", "x-y_1"]
        );
        assert!(check_name("github").is_ok());
        assert!(check_name("GitHub").is_err());
        assert!(check_name(INDEX).is_err());
        assert_eq!(env_var_name("my-api_key"), "FYX_SECRET_MY_API_KEY");
    }

    #[test]
    fn encodings_match_the_usual_tools() {
        assert_eq!(base64(b"fake-token", STANDARD_ALPHABET), "ZmFrZS10b2tlbg==");
        assert_eq!(base64(b"ab?", URL_SAFE_ALPHABET), "YWI_");
        assert_eq!(base64(b"abc", STANDARD_ALPHABET), "YWJj");
        assert_eq!(hex(b"ok\n", false), "6f6b0a");
        assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn every_encoding_is_redacted() {
        let value = "fake+value/42";
        let mut vault_patterns = encodings(value)
            .into_iter()
            .map(|encoded| (encoded, "demo".to_string()))
            .collect::<Vec<_>>();
        vault_patterns.sort_by_key(|(encoded, _)| std::cmp::Reverse(encoded.len()));
        for encoded in encodings(value) {
            let output = format!("TOKEN={encoded}\n");
            assert_eq!(
                redact_with(&output, &vault_patterns).as_deref(),
                Some("TOKEN=secret://demo\n"),
                "{encoded}"
            );
        }
        assert_eq!(redact_with("nothing here", &vault_patterns), None);
    }
}
