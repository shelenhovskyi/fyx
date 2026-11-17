//! fyx: settings content of Fyx-owned crates. A file of its own, so rebases never conflict on it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

/// Host review of agent edits (`crates/agent_review`).
#[with_fallible_options]
#[derive(Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema, MergeFrom, Debug)]
pub struct AgentReviewSettingsContent {
    /// Hold every agent edit for hunk-by-hunk review before it lands. The `FYX_AGENT_REVIEW`
    /// environment variable overrides it: `0` turns review off, any other value turns it on.
    ///
    /// Default: false
    pub enabled: Option<bool>,
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
}
