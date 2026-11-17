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
