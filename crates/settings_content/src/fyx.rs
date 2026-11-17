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
    /// (`fs/write_text_file`), and the edit and write tools of Zed's built-in agent. Not reviewed:
    /// shell and terminal commands (files they change are listed as changed without review), the
    /// built-in agent's delete, move and copy tools, and edits whose diff Fyx cannot read (a
    /// notification says so). Only user settings can turn this off; `.fyx/review.toml` in a
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
/// user settings: project settings have no `fyx_security` key.
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
}
