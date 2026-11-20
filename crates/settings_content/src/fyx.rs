//! fyx: settings content of Fyx-owned crates. A file of its own, so rebases never conflict on it.

use std::path::{Component, Path, PathBuf};

use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

/// Host review of agent edits (`crates/agent_review`).
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct AgentReviewSettingsContent {
    /// Hold agent edits for hunk-by-hunk review before they land. Covered: edits ACP agents propose
    /// with a diff (v1 permission requests, single-file v2 patches), writes through Fyx
    /// (`fs/write_text_file`), and the edit, write, delete, move and copy tools of Zed's built-in
    /// agent. Not reviewed: shell and terminal commands (files they change are listed as changed
    /// without review), and edits whose diff Fyx cannot read (a notification says so). Only user settings can turn this off; `.fyx/review.toml` in a
    /// project can only make review stricter. The `FYX_AGENT_REVIEW` environment variable
    /// overrides it: `0` turns review off, any other value turns it on.
    ///
    /// Default: false
    pub enabled: Option<bool>,
    /// Paths whose agent edits land without review while review mode is on, as glob patterns
    /// relative to a worktree root (a pattern without `/` matches the file name at any depth).
    /// Only your user settings can set them: a project's `.fyx/review.toml` can only make review
    /// stricter, and its `deny` and `always_ask` patterns win over these.
    ///
    /// Default: []
    pub auto_accept: Option<Vec<String>>,
}

/// Security modes for agents (`crates/fyx_guard`, `docs/fyx/security-modes.md`). Read only from
/// user settings: project settings have no `fyx_security` key. While Fyx runs, a change that lowers
/// protection asks for confirmation.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxSecuritySettingsContent {
    /// How agents are guarded.
    ///
    /// Default: off
    pub mode: Option<FyxSecurityMode>,
    /// Environment variables an agent terminal keeps even though they look like secrets. Names or
    /// `*` patterns, matched ignoring ASCII case.
    ///
    /// Default: []
    pub allow_env: Option<Vec<String>>,
}

#[derive(
    Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, MergeFrom,
)]
#[serde(rename_all = "snake_case")]
pub enum FyxSecurityMode {
    /// No protection; behaves like Zed.
    #[default]
    Off,
    /// Log what shield would block, block nothing.
    Audit,
    /// Private files, secret environment variables and agent configs are out of agents' reach.
    Shield,
    /// A value this build does not know. Treated as `shield`, so a typo never turns protection off.
    #[serde(other)]
    #[schemars(skip)]
    Unrecognized,
}

impl crate::UserSettingsContent {
    /// The user's settings object for UI extension `extension_id` (`fyx_extensions.<id>`), as
    /// written: the extension's declarations check it (`ui_ext::settings`). User level only
    /// (design wit-0.4 §D.2): project settings files cannot set it.
    pub fn fyx_extension_settings(&self, extension_id: &str) -> Option<&serde_json::Value> {
        self.content.fyx_extensions.as_ref()?.get(extension_id)
    }
}

/// Fyx's own settings (`fyx` in settings).
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxSettingsContent {
    /// UI extensions (`crates/ui_ext_host`).
    pub extensions: Option<FyxExtensionsSettingsContent>,
    /// Terminal integrations (`crates/terminal_host`).
    pub terminal: Option<FyxTerminalSettingsContent>,
    /// Account profiles for CLI tools and agents (`crates/terminal_host`, design §C.6).
    pub accounts: Option<FyxAccountsSettingsContent>,
    /// Fyx's own updates (`crates/fyx_update_host`, `docs/fyx/platform/self-update.md`).
    pub updates: Option<FyxUpdatesSettingsContent>,
    /// Local history of files and agent turns (`crates/fyx_history_host`,
    /// `docs/fyx/platform/local-history.md`).
    pub history: Option<FyxHistorySettingsContent>,
}

/// `fyx.history`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxHistorySettingsContent {
    /// Keep a snapshot of a file on every save and of every file an agent turn changed, on this
    /// machine only, so a file or a whole agent turn can be restored ("fyx: local history").
    ///
    /// Default: true
    pub enabled: Option<bool>,
    /// The most disk space the history of one worktree may use, in megabytes; the oldest
    /// snapshots go first.
    ///
    /// Default: 256
    pub max_total_mb: Option<u64>,
    /// Snapshots older than this many days are deleted.
    ///
    /// Default: 30
    pub max_age_days: Option<u64>,
    /// Files larger than this many megabytes are not kept (their changes are still listed).
    ///
    /// Default: 8
    pub max_file_mb: Option<u64>,
}

/// `fyx.updates`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxUpdatesSettingsContent {
    /// Check once a day whether a new Fyx release is out, from the releases of
    /// `shelenhovskyi/fyx` on GitHub (two small requests, no identifiers). A new version is only
    /// offered; nothing is downloaded or installed until you press "Install". "fyx: check for
    /// updates" checks once, whatever this says.
    ///
    /// Default: false
    pub check: Option<bool>,
}

/// `fyx.terminal`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxTerminalSettingsContent {
    /// Pop up a notification when a program in a terminal (Claude Code, for example, through
    /// OSC 7501) starts waiting for you or finishes while the Fyx window is in the background.
    /// The terminal's tab and the Agent Sessions panel show the status either way.
    ///
    /// Default: true
    pub notifications: Option<bool>,
    /// Load Fyx's shell integration into zsh, bash, fish and PowerShell terminals: command blocks
    /// for "share the last command", prompt and exit-code marks. Fyx's scripts source your own
    /// startup files first and never change them. Applies to terminals opened afterwards.
    ///
    /// Default: true
    pub shell_integration: Option<bool>,
    /// A soft daily cap on agents' cost in US dollars, from the cost they report (plan 7.8).
    /// Reaching it stops the running agent turns with a notice; "Continue today" lifts it until
    /// the next day. 0 means no cap.
    ///
    /// Default: 0
    pub daily_cost_cap: Option<f32>,
}

/// `fyx.extensions`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxExtensionsSettingsContent {
    /// Folders of UI extensions in development, loaded on every start and reloaded when their
    /// component is rebuilt. "fyx_ui: install dev UI extension" adds to this list. Only your user
    /// settings count, and each folder still needs one "Enable" before it loads. The
    /// `FYX_UI_DEV_EXTENSIONS` environment variable adds more, without asking.
    ///
    /// Default: []
    pub dev_paths: Option<Vec<String>>,
    /// Show the Extension Inspector panel (live tree, events, calls and timings of UI
    /// extensions) in release builds too; dev builds always have it. New windows pick it up.
    ///
    /// Default: false
    pub developer_mode: Option<bool>,
    /// Status bar items of UI extensions not to show, as `"<extension id>/<surface id>"`. Right-click
    /// an item and pick "Hide Button" to add one; "fyx_ui: show hidden status items" empties it.
    ///
    /// Default: []
    pub hidden_status_items: Option<Vec<String>>,
    /// The Fyx extension registry: an `https://` base URL, `http://localhost:<port>/` for a
    /// local test server, or `file:///<dir>` for a registry directory. Empty turns the registry
    /// off. Nothing is fetched until you open the Fyx tab or an update check is due.
    ///
    /// Default: ""
    pub registry_url: Option<String>,
    /// Zed's extension registry (themes, languages, language servers): an `http(s)://` base URL.
    /// Only the Extensions page, installs and update checks of installed Zed extensions use it;
    /// sign-in, telemetry and the rest of Zed's cloud stay off with `server_url`. Zed's terms
    /// apply to the registry. Empty builds the address from `server_url`, which in Fyx points
    /// nowhere, so the registry is off.
    ///
    /// Default: "https://api.zed.dev"
    pub zed_registry_url: Option<String>,
    /// What UI extensions may reach over the network.
    pub network: Option<FyxExtensionsNetworkSettingsContent>,
}

/// `fyx.extensions.network`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxExtensionsNetworkSettingsContent {
    /// Host names that UI extensions may reach on your private network (10/8, 172.16/12,
    /// 192.168/16, 100.64/10, fc00::/7), when the extension also declares them.
    /// `*.corp.example` matches subdomains. Loopback and link-local addresses are never allowed.
    ///
    /// Default: []
    pub private_hosts: Option<Vec<String>>,
}

/// `fyx.accounts`: several accounts of one tool, each in its own config dir. Fyx only puts the
/// tool's config-dir variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, ...) into a terminal or an agent;
/// it never reads, copies or moves what is in the dir. Sign in with the tool's own command in a
/// terminal of that account. Read only from your user settings: project settings have no `fyx` key.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxAccountsSettingsContent {
    /// Your accounts, e.g.
    /// `{ "name": "Work", "tool": "claude", "dir": "~/.claude-work", "color": "red" }`.
    ///
    /// Default: []
    pub profiles: Option<Vec<FyxAccountProfileContent>>,
    /// Tools that can have accounts, by the name `profiles` use, with the variable that selects
    /// their config dir. Presets for claude, codex, gemini, copilot and opencode.
    pub tools: Option<HashMap<String, FyxAccountToolContent>>,
    /// The account each project uses, by project folder and then tool:
    /// `{ "~/work/api": { "claude": "Work" } }`. Terminals opened inside the folder, and agents of
    /// that tool in a project with this folder, get the account. Set from the terminal panel's "+"
    /// menu. Kept here, never in the repository, so a repository cannot pick your account.
    ///
    /// Default: {}
    pub projects: Option<HashMap<String, HashMap<String, String>>>,
}

/// One account in `fyx.accounts.profiles`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxAccountProfileContent {
    /// Shown on terminal tabs and in the sessions list, e.g. "Work".
    pub name: Option<String>,
    /// The tool, a key of `fyx.accounts.tools`, e.g. "claude".
    pub tool: Option<String>,
    /// The tool's config dir for this account; `~` is your home folder.
    pub dir: Option<String>,
    /// The badge colour.
    ///
    /// Default: blue
    pub color: Option<FyxAccountColor>,
}

/// A tool in `fyx.accounts.tools`.
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxAccountToolContent {
    /// The environment variable that selects the tool's config dir, e.g. `CLAUDE_CONFIG_DIR`.
    pub env: Option<String>,
    /// The tool's name in menus, e.g. "Claude Code".
    pub label: Option<String>,
    /// Ids of agent servers (`agent_servers`) that run this tool and get the project's account.
    ///
    /// Default: []
    pub agents: Option<Vec<String>>,
}

/// The colour of an account's badge, from the theme's terminal colours.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, MergeFrom,
)]
#[serde(rename_all = "snake_case")]
pub enum FyxAccountColor {
    #[default]
    Blue,
    Green,
    Yellow,
    Red,
    Magenta,
    Cyan,
    /// A value this build does not know; shown as blue.
    #[serde(other)]
    #[schemars(skip)]
    Unrecognized,
}

/// An account profile with what a terminal or an agent needs: the variable and the dir.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FyxAccount {
    pub name: String,
    pub tool: String,
    /// The variable that selects the tool's config dir.
    pub env: String,
    /// The dir, `~` expanded. Opaque: Fyx never looks inside.
    pub dir: PathBuf,
    pub color: FyxAccountColor,
}

impl FyxAccount {
    /// The `(name, value)` to put into an environment.
    pub fn env_entry(&self) -> (String, String) {
        (self.env.clone(), self.dir.to_string_lossy().into_owned())
    }
}

impl FyxAccountsSettingsContent {
    /// Every complete profile, in settings order. Profiles of unknown tools, with an empty name or
    /// dir, or whose tool's variable is not a plain name (or is one of Fyx's own) are skipped.
    pub fn accounts(&self) -> Vec<FyxAccount> {
        self.profiles
            .iter()
            .flatten()
            .filter_map(|profile| {
                let name = profile.name.as_deref()?.trim();
                let tool = profile.tool.as_deref()?.trim();
                let dir = profile.dir.as_deref()?.trim();
                if name.is_empty() || dir.is_empty() {
                    return None;
                }
                let env = self.tool_env(tool)?;
                Some(FyxAccount {
                    name: name.to_owned(),
                    tool: tool.to_owned(),
                    env: env.to_owned(),
                    dir: expand_home(dir),
                    color: profile.color.unwrap_or_default(),
                })
            })
            .collect()
    }

    /// The profile named `name` of `tool`.
    pub fn account(&self, tool: &str, name: &str) -> Option<FyxAccount> {
        self.accounts()
            .into_iter()
            .find(|account| account.tool == tool && account.name == name)
    }

    /// The tool's menu name, or its key.
    pub fn tool_label(&self, tool: &str) -> String {
        self.tools
            .as_ref()
            .and_then(|tools| tools.get(tool))
            .and_then(|tool| tool.label.clone())
            .unwrap_or_else(|| tool.to_owned())
    }

    /// The tool whose `agents` list `agent_id`.
    pub fn tool_for_agent(&self, agent_id: &str) -> Option<&str> {
        let mut tools = self
            .tools
            .iter()
            .flatten()
            .filter(|(_, tool)| tool.agents.iter().flatten().any(|id| id == agent_id))
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        tools.sort_unstable();
        tools.first().copied()
    }

    /// The folder of `fyx.accounts.projects` that holds `path` (the deepest one), as written in
    /// settings, and its choices by tool.
    pub fn project_for(&self, path: &Path) -> Option<(&str, &HashMap<String, String>)> {
        self.projects
            .iter()
            .flatten()
            .filter_map(|(folder, choices)| {
                let root = expand_home(folder);
                path_within(path, &root).then(|| (root.components().count(), folder, choices))
            })
            .max_by_key(|(depth, _, _)| *depth)
            .map(|(_, folder, choices)| (folder.as_str(), choices))
    }

    /// The accounts chosen for the project that holds `path`, one per tool, sorted by tool. A
    /// choice naming no profile is skipped.
    pub fn project_accounts(&self, path: &Path) -> Vec<FyxAccount> {
        let Some((_, choices)) = self.project_for(path) else {
            return Vec::new();
        };
        let mut accounts = choices
            .iter()
            .filter_map(|(tool, name)| self.account(tool, name))
            .collect::<Vec<_>>();
        accounts.sort_by(|a, b| a.tool.cmp(&b.tool));
        accounts
    }

    /// The account of `tool` chosen for the first of `roots` that has one.
    pub fn project_account<'a>(
        &self,
        tool: &str,
        roots: impl IntoIterator<Item = &'a Path>,
    ) -> Option<FyxAccount> {
        roots.into_iter().find_map(|root| {
            self.project_accounts(root)
                .into_iter()
                .find(|account| account.tool == tool)
        })
    }

    fn tool_env(&self, tool: &str) -> Option<&str> {
        let env = self.tools.as_ref()?.get(tool)?.env.as_deref()?.trim();
        is_account_env_name(env).then_some(env)
    }
}

/// A plain variable name that is not one of Fyx's or Zed's own.
fn is_account_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let starts_well = bytes
        .next()
        .is_some_and(|first| first == b'_' || first.is_ascii_alphabetic());
    let upper = name.to_ascii_uppercase();
    starts_well
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
        && !upper.starts_with("FYX_")
        && !upper.starts_with("ZED_")
}

/// `~` and `~/...` (or `~\...`) relative to the home folder.
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => util::paths::home_dir().clone(),
        Some(rest) if rest.starts_with(['/', '\\']) => {
            util::paths::home_dir().join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(path),
    }
}

/// Whether `path` is `root` or inside it, by components (ASCII case ignored on Windows).
fn path_within(path: &Path, root: &Path) -> bool {
    let normal = |path: &Path| {
        path.components()
            .filter(|component| !matches!(component, Component::CurDir))
            .map(|component| {
                let text = component.as_os_str().to_string_lossy();
                if cfg!(windows) {
                    text.replace('/', "\\").to_ascii_lowercase()
                } else {
                    text.into_owned()
                }
            })
            .collect::<Vec<_>>()
    };
    let root = normal(root);
    !root.is_empty() && normal(path).starts_with(&root)
}

#[cfg(test)]
mod account_tests {
    #[test]
    fn extension_settings_are_kept_as_written() {
        let user: crate::UserSettingsContent = serde_json::from_str(
            r#"{ "fyx_extensions": { "acme.todo": { "limit": 5, "mode": "x", "extra": [1] }, "acme.bad": 3 } }"#,
        )
        .expect("user settings");
        assert_eq!(
            user.fyx_extension_settings("acme.todo"),
            Some(&serde_json::json!({ "limit": 5, "mode": "x", "extra": [1] }))
        );
        assert_eq!(
            user.fyx_extension_settings("acme.bad"),
            Some(&serde_json::json!(3)),
            "not an object: the extension's checks report it"
        );
        assert_eq!(user.fyx_extension_settings("acme.none"), None);
    }

    use super::*;

    fn settings(json: &str) -> FyxAccountsSettingsContent {
        serde_json::from_str(json).unwrap()
    }

    const TWO: &str = r#"{
        "tools": { "claude": { "env": "CLAUDE_CONFIG_DIR", "agents": ["claude-acp"] },
                   "bad": { "env": "FYX_TERMINAL_TOKEN" } },
        "profiles": [
            { "name": "Personal", "tool": "claude", "dir": "/home/a/.claude" },
            { "name": "Work", "tool": "claude", "dir": "/home/a/.claude-work", "color": "red" },
            { "name": "Odd", "tool": "bad", "dir": "/x" },
            { "name": "", "tool": "claude", "dir": "/y" },
            { "name": "Lost", "tool": "nope", "dir": "/z" }
        ],
        "projects": { "/src/work": { "claude": "Work" }, "/src/work/side": { "claude": "Personal" },
                      "/src/gone": { "claude": "Nobody" } }
    }"#;

    #[test]
    fn complete_profiles_of_known_tools_only() {
        let accounts = settings(TWO).accounts();
        let names = accounts.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Personal", "Work"]);
        assert_eq!(accounts[1].color, FyxAccountColor::Red);
        assert_eq!(accounts[0].color, FyxAccountColor::Blue);
        assert_eq!(
            accounts[1].env_entry(),
            (
                "CLAUDE_CONFIG_DIR".to_owned(),
                PathBuf::from("/home/a/.claude-work")
                    .to_string_lossy()
                    .into_owned()
            )
        );
    }

    #[test]
    fn deepest_project_folder_wins() {
        let settings = settings(TWO);
        let at = |path: &str| {
            settings
                .project_accounts(Path::new(path))
                .into_iter()
                .map(|a| a.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(at("/src/work"), ["Work"]);
        assert_eq!(at("/src/work/api/src"), ["Work"]);
        assert_eq!(at("/src/work/side/x"), ["Personal"]);
        assert!(at("/src/workshop").is_empty(), "by components, not text");
        assert!(at("/src/gone").is_empty(), "a choice naming no profile");
        assert!(at("/elsewhere").is_empty());
    }

    #[test]
    fn agents_map_to_tools() {
        let settings = settings(TWO);
        assert_eq!(settings.tool_for_agent("claude-acp"), Some("claude"));
        assert_eq!(settings.tool_for_agent("codex-acp"), None);
        let account = settings
            .project_account("claude", [Path::new("/other"), Path::new("/src/work/a")])
            .unwrap();
        assert_eq!(account.name, "Work");
    }

    #[test]
    fn env_names_are_plain_and_not_fyx() {
        assert!(is_account_env_name("CLAUDE_CONFIG_DIR"));
        assert!(is_account_env_name("_X1"));
        for bad in ["", "1X", "A-B", "A B", "fyx_x", "ZED_X"] {
            assert!(!is_account_env_name(bad), "{bad}");
        }
    }

    #[test]
    fn home_is_expanded() {
        assert_eq!(expand_home("~"), *util::paths::home_dir());
        assert_eq!(
            expand_home("~/.claude-work"),
            util::paths::home_dir().join(".claude-work")
        );
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
