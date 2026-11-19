//! The manifest side of the fyx:ui 0.4 integration kit (docs/fyx/wit-0.4/design.md §B.7–§B.9,
//! plan 5.5): `[[fyx_ui.listeners]]`, `[[fyx_ui.discovery_files]]` and
//! `[[fyx_ui.relay_channels]]`, with the checks made when a manifest is read. The listeners
//! themselves (5.11) and the file writer and relay (5.12, W1) build on these.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::terminal::check_reason;

pub const CAPABILITY_NETWORK_LISTEN: &str = "network:listen";
pub const CAPABILITY_DISCOVERY_FILE: &str = "fs:discovery-file";
pub const CAPABILITY_RELAY_HOOKS: &str = "relay:hooks";
pub const CAPABILITY_REVIEW_PROPOSE: &str = "review:propose";
pub const CAPABILITY_CONTEXT_EDITOR: &str = "context:editor";
pub const CAPABILITY_WORKSPACE_OPEN: &str = "workspace:open";
pub const CAPABILITY_APPROVALS: &str = "ui:approvals";

/// §B.7.
pub const MAX_LISTENERS: usize = 4;
/// §B.9.
pub const MAX_RELAY_CHANNELS: usize = 16;
pub const MAX_RELAY_TIMEOUT_MS: u32 = 600_000;
pub const DEFAULT_RELAY_TIMEOUT_MS: u32 = 30_000;
/// One discovery file per listener and account dir; the manifest declares at most this many.
pub const MAX_DISCOVERY_FILES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ListenerProtocol {
    McpWebsocket,
    McpStreamableHttp,
    Websocket,
    Http,
}

/// How a client presents the listener's token: one of the three.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerAuth {
    /// A header that carries the token as it is.
    #[serde(default)]
    pub header: Option<String>,
    /// `Authorization: Bearer <token>`.
    #[serde(default)]
    pub bearer: bool,
    /// A query parameter.
    #[serde(default)]
    pub query: Option<String>,
}

/// One `[[fyx_ui.listeners]]` entry (§B.7).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerSpec {
    /// `[a-z0-9-]{1,32}`, unique; `${listen.<name>.…}` names it.
    pub name: String,
    pub protocol: ListenerProtocol,
    pub auth: ListenerAuth,
    /// `${listen.<name>.token}` may go into terminal env.
    #[serde(default)]
    pub token_in_env: bool,
    pub reason: String,
}

/// What a relay channel answers when Fyx or the extension does not (§B.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnFailure {
    pub exit: u8,
}

impl Default for OnFailure {
    /// Fails open: exit 0, no output.
    fn default() -> Self {
        Self { exit: 0 }
    }
}

/// One `[[fyx_ui.relay_channels]]` entry (§B.9).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayChannelSpec {
    pub name: String,
    #[serde(default = "default_relay_timeout")]
    pub timeout_ms: u32,
    #[serde(default)]
    pub on_failure: OnFailure,
    pub reason: String,
}

fn default_relay_timeout() -> u32 {
    DEFAULT_RELAY_TIMEOUT_MS
}

/// One `[[fyx_ui.discovery_files]]` entry (§B.8): a template the host fills and writes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryFileSpec {
    pub listener: String,
    /// `${account.<tool>}/…` or `${tmp}/…`.
    pub dir: String,
    /// Under `~/`, used when no account profile applies.
    #[serde(default)]
    pub default_dir: Option<String>,
    /// Contains `${port}` or `${pid}`, ends in `.lock` or `.json`.
    pub name: String,
    /// Written as a JSON object; values are text with `${port}`, `${token}`, `${pid}`,
    /// `${worktrees}`, `${uid}` and `${tmp}`.
    pub content: BTreeMap<String, String>,
    pub reason: String,
}

/// §B.7: what a listener's clients send that the extension handles, as `io-event`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerEvent {
    /// An `mcp-*` listener's `tools/call` of a registered tool.
    McpCall(McpCall),
    /// A call whose connection closed, or that the client cancelled, before the answer.
    McpCancelled(u64),
    /// A `websocket` listener's text frame.
    Message {
        listener: String,
        connection: u64,
        text: String,
    },
    /// An `http` listener's request, answered with `listeners.reply(id, …)`.
    Request {
        id: u64,
        listener: String,
        path: String,
        body: String,
    },
}

/// `io-event::mcp-call`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpCall {
    pub call_id: u64,
    pub listener: String,
    pub connection: u64,
    pub tool: String,
    /// JSON, as text: the call's `arguments` (`{}` when absent).
    pub arguments: String,
}

/// `listeners.tool-def`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// A JSON schema object, as text.
    pub input_schema: String,
}

/// `listeners.mcp-error`: a JSON-RPC error the client gets instead of a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpError {
    pub code: i32,
    pub message: String,
}

/// At most this many tools per listener.
pub const MAX_TOOLS: usize = 64;

/// A tool name: `[A-Za-z0-9_-]{1,64}`, as MCP clients accept it.
pub fn is_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// An open listener, for the discovery-file writer (§B.8, plan 5.12) and `${listen.<name>.…}`.
/// The token is the host's: it never reaches the extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenListener {
    pub name: String,
    pub protocol: ListenerProtocol,
    pub port: u16,
    pub token: String,
    /// The listener's `[[fyx_ui.discovery_files]]` entries that are granted now.
    pub discovery_files: Vec<DiscoveryFileSpec>,
}

/// The host's side of the listeners: told when one opens, when its granted discovery files
/// change, and when it closes (reload, disable, uninstall, revocation). The discovery-file writer
/// writes exactly `discovery_files` for an open listener and removes every file it wrote for a
/// closed one.
pub trait ListenerObserver: Send + Sync {
    fn opened(&self, extension_id: &str, listener: &OpenListener);
    fn closed(&self, extension_id: &str, listener: &str);
}

/// The integration tables of one manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntegrationTables<'a> {
    pub listeners: &'a [ListenerSpec],
    pub discovery_files: &'a [DiscoveryFileSpec],
    pub relay_channels: &'a [RelayChannelSpec],
}

impl IntegrationTables<'_> {
    /// For [`crate::terminal::Template::parse`]: whether a listener is declared, and whether it
    /// has `token_in_env`.
    pub fn listener(&self, name: &str) -> Option<bool> {
        self.listeners
            .iter()
            .find(|listener| listener.name == name)
            .map(|listener| listener.token_in_env)
    }

    pub fn check(&self) -> Result<(), String> {
        check_listeners(self.listeners)?;
        check_relay_channels(self.relay_channels)?;
        if self.discovery_files.len() > MAX_DISCOVERY_FILES {
            return Err(format!(
                "[[fyx_ui.discovery_files]] has {} entries, at most {MAX_DISCOVERY_FILES}",
                self.discovery_files.len()
            ));
        }
        for file in self.discovery_files {
            check_discovery_file(file, |name| self.listener(name).is_some())?;
        }
        Ok(())
    }
}

fn check_listeners(specs: &[ListenerSpec]) -> Result<(), String> {
    if specs.len() > MAX_LISTENERS {
        return Err(format!(
            "[[fyx_ui.listeners]] has {} entries, at most {MAX_LISTENERS}",
            specs.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for spec in specs {
        let at = |why: &str| format!("listener {:?}: {why}", spec.name);
        if !is_name(&spec.name) {
            return Err(at("the name must be [a-z0-9-]{1,32}"));
        }
        if !seen.insert(&spec.name) {
            return Err(at("declared twice"));
        }
        let auth = &spec.auth;
        let ways = usize::from(auth.header.is_some())
            + usize::from(auth.bearer)
            + usize::from(auth.query.is_some());
        if ways != 1 {
            return Err(at("auth must be exactly one of header, bearer or query"));
        }
        for name in auth.header.iter().chain(&auth.query) {
            let valid = (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            if !valid {
                return Err(at(
                    "the auth header or query name must be [A-Za-z0-9_-]{1,64}",
                ));
            }
            if auth.header.as_deref() == Some(name.as_str())
                && matches!(
                    name.to_ascii_lowercase().as_str(),
                    "host" | "origin" | "cookie" | "content-length" | "transfer-encoding"
                )
            {
                return Err(at("that header cannot carry the token"));
            }
        }
        check_reason("a listener", &spec.reason).map_err(|why| at(&why))?;
    }
    Ok(())
}

fn check_relay_channels(specs: &[RelayChannelSpec]) -> Result<(), String> {
    if specs.len() > MAX_RELAY_CHANNELS {
        return Err(format!(
            "[[fyx_ui.relay_channels]] has {} entries, at most {MAX_RELAY_CHANNELS}",
            specs.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for spec in specs {
        let at = |why: &str| format!("relay channel {:?}: {why}", spec.name);
        if !is_name(&spec.name) {
            return Err(at("the name must be [a-z0-9-]{1,32}"));
        }
        if !seen.insert(&spec.name) {
            return Err(at("declared twice"));
        }
        if !(1..=MAX_RELAY_TIMEOUT_MS).contains(&spec.timeout_ms) {
            return Err(at(&format!(
                "timeout_ms must be 1 to {MAX_RELAY_TIMEOUT_MS}"
            )));
        }
        check_reason("a relay channel", &spec.reason).map_err(|why| at(&why))?;
    }
    Ok(())
}

const DISCOVERY_PLACEHOLDERS: &[&str] = &["port", "token", "pid", "worktrees", "uid", "tmp"];

/// The placeholders of `text`, or an error for an unclosed or unknown one.
fn placeholders<'a>(text: &'a str, known: &[&str]) -> Result<Vec<&'a str>, String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("{text:?}: an unclosed ${{"))?;
        let name = &after[..end];
        if !known.contains(&name) {
            return Err(format!("{text:?}: unknown placeholder ${{{name}}}"));
        }
        found.push(name);
        rest = &after[end + 1..];
    }
    Ok(found)
}

/// The static checks of §B.8; the host checks the resolved path again (no symlinks, `0700`).
fn check_discovery_file(
    spec: &DiscoveryFileSpec,
    listener_exists: impl Fn(&str) -> bool,
) -> Result<(), String> {
    let at = |why: &str| format!("discovery file {:?}: {why}", spec.name);
    if !listener_exists(&spec.listener) {
        return Err(at(&format!("no listener named {:?}", spec.listener)));
    }
    let has_parent = |path: &str| {
        path.split(['/', '\\'])
            .any(|component| component == ".." || component == ".")
    };
    // `dir`: under an account profile dir or the temp dir, then plain components.
    let rest = if let Some(rest) = spec.dir.strip_prefix("${tmp}") {
        rest
    } else if let Some(after) = spec.dir.strip_prefix("${account.") {
        let (tool, rest) = after
            .split_once('}')
            .ok_or_else(|| at("an unclosed ${account."))?;
        if !is_name(tool) {
            return Err(at("the account tool must be [a-z0-9-]{1,32}"));
        }
        rest
    } else {
        return Err(at("dir must start with ${account.<tool>} or ${tmp}"));
    };
    if !(rest.is_empty() || rest.starts_with('/'))
        || has_parent(rest)
        || rest.contains(['$', '\\', ':', '\0'])
    {
        return Err(at("dir must continue with plain /-separated folder names"));
    }
    if let Some(default_dir) = &spec.default_dir {
        let under_home = default_dir
            .strip_prefix("~/")
            .is_some_and(|rest| !rest.is_empty() && !has_parent(rest));
        if !under_home || default_dir.contains(['$', '\\', ':', '\0']) {
            return Err(at(
                "default_dir must be a folder under ~/ without .. or ${…}",
            ));
        }
    }
    // The name: never a tool's settings file.
    let names = placeholders(&spec.name, &["port", "pid"]).map_err(|why| at(&why))?;
    let valid_name = !names.is_empty()
        && (spec.name.ends_with(".lock") || spec.name.ends_with(".json"))
        && !spec.name.contains(['/', '\\', ':', '\0'])
        && !spec.name.starts_with('.');
    if !valid_name {
        return Err(at(
            "the name must contain ${port} or ${pid}, end in .lock or .json, and name no folder",
        ));
    }
    if spec.content.is_empty() || spec.content.len() > 32 {
        return Err(at("content must have 1 to 32 keys"));
    }
    for (key, value) in &spec.content {
        if key.is_empty() || key.len() > 64 || value.len() > 1024 {
            return Err(at("a content key or value is too long"));
        }
        placeholders(value, DISCOVERY_PLACEHOLDERS).map_err(|why| at(&why))?;
    }
    check_reason("a discovery file", &spec.reason).map_err(|why| at(&why))?;
    Ok(())
}

/// `[a-z0-9-]{1,32}`.
fn is_name(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener(name: &str) -> ListenerSpec {
        ListenerSpec {
            name: name.into(),
            protocol: ListenerProtocol::McpWebsocket,
            auth: ListenerAuth {
                header: Some("x-claude-code-ide-authorization".into()),
                ..ListenerAuth::default()
            },
            token_in_env: false,
            reason: "lets Claude Code connect".into(),
        }
    }

    fn lock(dir: &str, name: &str) -> DiscoveryFileSpec {
        DiscoveryFileSpec {
            listener: "ide".into(),
            dir: dir.into(),
            default_dir: Some("~/.claude/ide".into()),
            name: name.into(),
            content: [
                ("pid".to_string(), "${pid}".to_string()),
                ("authToken".to_string(), "${token}".to_string()),
                ("workspaceFolders".to_string(), "${worktrees}".to_string()),
            ]
            .into_iter()
            .collect(),
            reason: "Claude Code finds Fyx through it".into(),
        }
    }

    fn check(listeners: &[ListenerSpec], files: &[DiscoveryFileSpec]) -> Result<(), String> {
        IntegrationTables {
            listeners,
            discovery_files: files,
            relay_channels: &[],
        }
        .check()
    }

    #[test]
    fn listeners() {
        assert!(check(&[listener("ide")], &[]).is_ok());
        let mut two_ways = listener("ide");
        two_ways.auth.bearer = true;
        let mut no_way = listener("ide");
        no_way.auth.header = None;
        let mut origin = listener("ide");
        origin.auth.header = Some("Origin".into());
        let mut no_reason = listener("ide");
        no_reason.reason.clear();
        for bad in [
            vec![two_ways],
            vec![no_way],
            vec![origin],
            vec![no_reason],
            vec![listener("IDE")],
            vec![listener("a"), listener("a")],
            (0..=MAX_LISTENERS)
                .map(|i| listener(&format!("l{i}")))
                .collect(),
        ] {
            assert!(check(&bad, &[]).is_err(), "{bad:?}");
        }
    }

    /// Plan 5.12 "done when": a template naming `settings.json` is refused; with it, `..`, a
    /// foreign root and unknown placeholders.
    #[test]
    fn discovery_files() {
        let ide = [listener("ide")];
        assert!(check(&ide, &[lock("${account.claude}/ide", "${port}.lock")]).is_ok());
        assert!(
            check(
                &ide,
                &[lock(
                    "${tmp}/gemini-ide",
                    "gemini-ide-server-${pid}-${port}.json"
                )]
            )
            .is_ok()
        );
        for bad in [
            lock("${account.claude}", "settings.json"),
            lock("${account.claude}/ide", "${port}.toml"),
            lock("${account.claude}/ide", "../${port}.lock"),
            lock("${account.claude}/../x", "${port}.lock"),
            lock("~/.claude/ide", "${port}.lock"),
            lock("/etc", "${port}.lock"),
            lock("${home}/x", "${port}.lock"),
            lock("${account.claude}/ide", "${token}.lock"),
            lock("${tmp}x", "${port}.lock"),
        ] {
            assert!(check(&ide, std::slice::from_ref(&bad)).is_err(), "{bad:?}");
        }
        let mut default_outside = lock("${account.claude}/ide", "${port}.lock");
        default_outside.default_dir = Some("/tmp/x".into());
        assert!(check(&ide, &[default_outside]).is_err());
        let mut unknown = lock("${account.claude}/ide", "${port}.lock");
        unknown.content.insert("x".into(), "${env.HOME}".into());
        assert!(check(&ide, &[unknown]).is_err());
        assert!(
            check(&[], &[lock("${tmp}", "${port}.lock")]).is_err(),
            "no listener"
        );
    }

    #[test]
    fn relay_channels() {
        let channel = |name: &str, timeout_ms| RelayChannelSpec {
            name: name.into(),
            timeout_ms,
            on_failure: OnFailure::default(),
            reason: "the pre-tool hook".into(),
        };
        let tables = |channels: &[RelayChannelSpec]| {
            IntegrationTables {
                relay_channels: channels,
                ..IntegrationTables::default()
            }
            .check()
        };
        assert!(tables(&[channel("pre-tool", 30_000)]).is_ok());
        assert!(tables(&[channel("pre-tool", 0)]).is_err());
        assert!(tables(&[channel("pre-tool", MAX_RELAY_TIMEOUT_MS + 1)]).is_err());
        assert!(tables(&[channel("a", 1), channel("a", 1)]).is_err());
        assert_eq!(OnFailure::default().exit, 0);
    }
}
