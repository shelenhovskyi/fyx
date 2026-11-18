//! fyx: settings content of Fyx-owned crates. A file of its own, so rebases never conflict on it.

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

/// Fyx's own settings (`fyx` in settings).
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct FyxSettingsContent {
    /// UI extensions (`crates/ui_ext_host`).
    pub extensions: Option<FyxExtensionsSettingsContent>,
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
}
