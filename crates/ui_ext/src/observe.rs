//! The pure part of `agent:observe` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §11, plan 4.8):
//! the read-only view of agent reviews, its limits, the per-extension opaque ids and the cut of
//! large reviews. Nothing here, and nothing in the world, can decide a review.

use std::collections::HashMap;

use crate::process::FileRef;

pub const CAPABILITY_OBSERVE: &str = "agent:observe";
/// Bytes of hunk text (old plus new) per review.
pub const MAX_HUNK_TEXT_BYTES: usize = 256 * 1024;
/// Reviews in a snapshot; the oldest are dropped first.
pub const MAX_SNAPSHOT_REVIEWS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HunkDecision {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkInfo {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub old_text: String,
    pub new_text: String,
    pub decision: HunkDecision,
    pub comment: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    Unchanged,
    Pending(u32),
    AcceptedAll,
    RejectedAll,
    /// Accepted and rejected hunk counts.
    Partial(u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReviewSource {
    Permission,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleKind {
    AutoAccept,
    AlwaysAsk,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewInfo {
    pub review: u64,
    pub session: u64,
    pub turn: u64,
    /// `None` when the file is outside every worktree; then there are no hunks either.
    pub file: Option<FileRef>,
    pub source: ReviewSource,
    pub rule: Option<RuleKind>,
    pub resolution: Resolution,
    pub submitted: bool,
    pub hunks: Vec<HunkInfo>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnFile {
    pub file: Option<FileRef>,
    pub resolution: Resolution,
    pub rule: Option<RuleKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    Snapshot(Vec<ReviewInfo>),
    Opened(ReviewInfo),
    Updated(ReviewInfo),
    Resolved(ReviewInfo),
    Closed(u64),
    TurnEnded(u64, Vec<TurnFile>),
    UnreviewedChanges(u64, Vec<FileRef>),
    ReviewMode(bool),
}

impl AgentEvent {
    /// The review an `updated` event is about, for coalescing under backpressure.
    pub fn updated_review(&self) -> Option<u64> {
        match self {
            AgentEvent::Updated(info) => Some(info.review),
            _ => None,
        }
    }
}

/// Cuts a review to [`MAX_HUNK_TEXT_BYTES`] of hunk text, keeping whole hunks, and drops every
/// hunk of a review whose file is outside the worktrees (design.md §11).
pub fn cut_review(mut info: ReviewInfo) -> ReviewInfo {
    if info.file.is_none() {
        info.truncated = info.truncated || !info.hunks.is_empty();
        info.hunks.clear();
        return info;
    }
    let mut total = 0;
    let mut keep = 0;
    for hunk in &info.hunks {
        let size = hunk.old_text.len() + hunk.new_text.len();
        if total + size > MAX_HUNK_TEXT_BYTES {
            break;
        }
        total += size;
        keep += 1;
    }
    if keep < info.hunks.len() {
        info.hunks.truncate(keep);
        info.truncated = true;
    }
    info
}

/// A snapshot as the extension gets it: at most [`MAX_SNAPSHOT_REVIEWS`], the oldest dropped
/// first (reviews are given oldest first), with `truncated` set on the last one kept.
pub fn cut_snapshot(mut reviews: Vec<ReviewInfo>) -> Vec<ReviewInfo> {
    let mut dropped = false;
    if reviews.len() > MAX_SNAPSHOT_REVIEWS {
        reviews.drain(..reviews.len() - MAX_SNAPSHOT_REVIEWS);
        dropped = true;
    }
    let mut reviews: Vec<ReviewInfo> = reviews.into_iter().map(cut_review).collect();
    if dropped && let Some(last) = reviews.last_mut() {
        last.truncated = true;
    }
    reviews
}

/// Maps the host's review, session and turn ids to opaque ones of one extension (design.md §0.5):
/// stable while it lives, never shared with another extension's map.
#[derive(Debug, Default)]
pub struct OpaqueIds {
    reviews: HashMap<u64, u64>,
    sessions: HashMap<u64, u64>,
    turns: HashMap<u64, u64>,
    next: u64,
}

impl OpaqueIds {
    fn map(table: &mut HashMap<u64, u64>, next: &mut u64, id: u64) -> u64 {
        *table.entry(id).or_insert_with(|| {
            *next += 1;
            *next
        })
    }

    pub fn review(&mut self, id: u64) -> u64 {
        Self::map(&mut self.reviews, &mut self.next, id)
    }

    pub fn session(&mut self, id: u64) -> u64 {
        Self::map(&mut self.sessions, &mut self.next, id)
    }

    pub fn turn(&mut self, id: u64) -> u64 {
        Self::map(&mut self.turns, &mut self.next, id)
    }

    fn info(&mut self, mut info: ReviewInfo) -> ReviewInfo {
        info.review = self.review(info.review);
        info.session = self.session(info.session);
        info.turn = self.turn(info.turn);
        info
    }

    /// The event with this extension's ids, and its reviews cut.
    pub fn event(&mut self, event: AgentEvent) -> AgentEvent {
        match event {
            AgentEvent::Snapshot(reviews) => AgentEvent::Snapshot(
                cut_snapshot(reviews)
                    .into_iter()
                    .map(|info| self.info(info))
                    .collect(),
            ),
            AgentEvent::Opened(info) => AgentEvent::Opened(self.info(cut_review(info))),
            AgentEvent::Updated(info) => AgentEvent::Updated(self.info(cut_review(info))),
            AgentEvent::Resolved(info) => AgentEvent::Resolved(self.info(cut_review(info))),
            AgentEvent::Closed(review) => AgentEvent::Closed(self.review(review)),
            AgentEvent::TurnEnded(turn, files) => AgentEvent::TurnEnded(self.turn(turn), files),
            AgentEvent::UnreviewedChanges(turn, files) => {
                AgentEvent::UnreviewedChanges(self.turn(turn), files)
            }
            AgentEvent::ReviewMode(on) => AgentEvent::ReviewMode(on),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(bytes: usize) -> HunkInfo {
        HunkInfo {
            old_start: 0,
            old_count: 1,
            new_start: 0,
            new_count: 1,
            old_text: String::new(),
            new_text: "a".repeat(bytes),
            decision: HunkDecision::Pending,
            comment: None,
        }
    }

    fn review(id: u64, file: bool, hunks: Vec<HunkInfo>) -> ReviewInfo {
        ReviewInfo {
            review: id,
            session: 900,
            turn: 901,
            file: file.then(|| FileRef {
                worktree: Some(1),
                path: "src/main.rs".into(),
            }),
            source: ReviewSource::Write,
            rule: None,
            resolution: Resolution::Pending(1),
            submitted: false,
            hunks,
            truncated: false,
        }
    }

    #[test]
    fn large_reviews_are_cut_at_whole_hunks() {
        let cut = cut_review(review(1, true, vec![hunk(200 * 1024), hunk(200 * 1024)]));
        assert_eq!(cut.hunks.len(), 1);
        assert!(cut.truncated);
        let whole = cut_review(review(1, true, vec![hunk(1024); 4]));
        assert_eq!(whole.hunks.len(), 4);
        assert!(!whole.truncated);
        // A 1 MiB hunk alone is cut entirely.
        assert!(
            cut_review(review(1, true, vec![hunk(1 << 20)]))
                .hunks
                .is_empty()
        );
    }

    #[test]
    fn files_outside_the_worktrees_have_no_hunks() {
        let outside = cut_review(review(1, false, vec![hunk(10)]));
        assert!(outside.hunks.is_empty());
        assert!(outside.file.is_none());
    }

    #[test]
    fn snapshots_keep_the_newest_thousand() {
        let reviews: Vec<ReviewInfo> = (0..1005).map(|id| review(id, true, Vec::new())).collect();
        let cut = cut_snapshot(reviews);
        assert_eq!(cut.len(), MAX_SNAPSHOT_REVIEWS);
        assert_eq!(cut[0].review, 5);
        assert!(cut.last().unwrap().truncated);
    }

    #[test]
    fn ids_are_opaque_per_extension_and_stable() {
        let mut first = OpaqueIds::default();
        let mut second = OpaqueIds::default();
        second.review(12345);
        let a = first.event(AgentEvent::Opened(review(777, true, Vec::new())));
        let b = second.event(AgentEvent::Opened(review(777, true, Vec::new())));
        let (AgentEvent::Opened(a), AgentEvent::Opened(b)) = (a, b) else {
            panic!("opened");
        };
        assert_ne!(a.review, 777);
        assert_ne!(a.review, b.review, "another extension sees another id");
        assert_eq!(
            first.event(AgentEvent::Closed(777)),
            AgentEvent::Closed(a.review)
        );
    }
}
