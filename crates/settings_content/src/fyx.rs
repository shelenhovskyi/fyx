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
