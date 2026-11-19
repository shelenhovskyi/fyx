//! fyx:ui 0.4 proposed edits through agent review (`review:propose`, design wit-0.4 §B.10, plan
//! 5.13): the model the runtime and the review bridge share. The bridge, in `ui_ext_host` over
//! `agent_review`'s public API, opens the reviews; accepting or rejecting stays the user's.

use std::sync::Arc;

use crate::process::FileRef;

pub use crate::integration::CAPABILITY_REVIEW_PROPOSE;

/// Open proposals per extension (§B.10).
pub const MAX_OPEN_PROPOSALS: usize = 64;
/// The largest proposed file, as new content or as a diff (§B.10).
pub const MAX_PROPOSAL_BYTES: usize = 8 << 20;
/// `title` and `session`, each.
pub const MAX_PROPOSAL_LABEL_BYTES: usize = 256;
/// Paths one `end-turn` may name.
pub const MAX_TURN_PATHS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proposed {
    NewContent(String),
    UnifiedDiff(String),
}

/// `review.proposal`, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub path: FileRef,
    pub change: Proposed,
    pub title: String,
    /// The extension's key for a tool session; the review panel names it.
    pub session: String,
    /// The host applies the accepted hunks to the buffer as one undo step and never saves.
    pub apply: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Accepted,
    Partial,
    Rejected,
}

/// `review.review-outcome`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewOutcome {
    pub decision: Decision,
    /// The text after the user's choices and edits.
    pub final_content: String,
}

/// Where the bridge sends a proposal's outcome, once, unless it was withdrawn: `Err` is the
/// reason the review ended without a decision (the file went away, the diff did not apply).
pub type ReviewSink = Arc<dyn Fn(u64, Result<ReviewOutcome, String>) + Send + Sync>;

/// The review side of `review` (`ui_ext_host` over `agent_review`). Ids are the runtime's, unique
/// in the process (a reloaded instance never reuses one); the runtime has checked the capability, the path (a worktree file, not
/// protected), the sizes and the per-extension limit.
pub trait ReviewHost: Send + Sync + 'static {
    /// Opens the review. `Err` is shown as `unavailable` (a private file, a diff that does not
    /// apply); nothing reaches `sink` then.
    fn propose(
        &self,
        extension_id: &str,
        id: u64,
        proposal: Proposal,
        sink: ReviewSink,
    ) -> Result<(), String>;
    /// Closes an undecided review without applying anything and returns its current text, the
    /// user's edits included. Its sink hears nothing more.
    fn withdraw(&self, extension_id: &str, id: u64) -> Result<String, String>;
    /// `agent_review`'s `begin_turn` for the tool session.
    fn begin_turn(&self, extension_id: &str, session: &str);
    /// `end_turn` and `note_changed_paths`: files changed in the turn without a proposal show as
    /// "changed without review".
    fn end_turn(&self, extension_id: &str, session: &str, changed: Vec<FileRef>);
    /// The instance is gone or `review:propose` was turned off: these open reviews (all of that
    /// instance's) close, nothing applied, and no outcome is sent. A reloaded instance's reviews
    /// are not among them.
    fn close(&self, extension_id: &str, ids: &[u64]);
}
