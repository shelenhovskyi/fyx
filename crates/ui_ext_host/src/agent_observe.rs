//! The agent reviews as `agent:observe` sees them (W2c's [`AgentReviews`], plan 4.8, design.md
//! 0.3 §11), over W5's read-only `agent_review::observe`.
//!
//! One observation of `AgentReview` per app, on the main thread, keeps a mirror of the open
//! reviews and fans every change out to the extensions that subscribed. A subscription takes the
//! mirror and joins the fan-out under one lock, so no change falls between the two. Files become
//! the extension's `file-ref`s (its own worktree ids), or `none` outside every worktree; review,
//! session and turn ids stay the host's, which `ui_ext_wasm` maps per extension. Review mode off
//! means no reviews and no events, only `review-mode(false)`; turning it on sends a fresh
//! snapshot. Nothing here can decide a review.
//!
//! A file an extension may not read (`private_files`, `file_scan_exclusions`, `fyx_guard`'s read
//! rules) reaches it as `none` with no hunks, like a file outside the worktrees (integration-5
//! S1, the same paths `workspace:edit` refuses). That is decided on the main thread, as each
//! change arrives.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
};

use agent_review::observe::{
    ObservedEvent, ObservedHunk, ObservedReview, ObservedSource, ObservedTurnFile,
};
use futures::{
    FutureExt as _, StreamExt as _, channel::mpsc, future::BoxFuture, stream::BoxStream,
};
use gpui::{App, Global};
use ui_ext_wasm::observe::{
    AgentEvent, AgentReviews, HunkDecision, HunkInfo, Resolution, ReviewInfo, ReviewSource,
    RuleKind, TurnFile,
};

use crate::worktrees::OpenWorktrees;

/// What the fan-out sends: an observed change, or the reviews after review mode came on.
#[derive(Clone)]
enum FeedEvent {
    Observed(ObservedEvent),
    Snapshot(Vec<ObservedReview>),
}

#[derive(Default)]
struct Feed {
    review_mode: bool,
    /// The open reviews, oldest first, whatever the mode.
    reviews: Vec<ObservedReview>,
    subscribers: Vec<mpsc::UnboundedSender<FeedEvent>>,
}

impl Feed {
    fn apply(&mut self, event: ObservedEvent) {
        let mut sent = vec![FeedEvent::Observed(event.clone())];
        match event {
            ObservedEvent::Opened(review)
            | ObservedEvent::Updated(review)
            | ObservedEvent::Resolved(review) => {
                match self.reviews.iter_mut().find(|open| open.id == review.id) {
                    Some(open) => *open = review,
                    None => self.reviews.push(review),
                }
            }
            ObservedEvent::Closed(id) => self.reviews.retain(|review| review.id != id),
            ObservedEvent::ReviewMode(on) => {
                let was = std::mem::replace(&mut self.review_mode, on);
                if on && !was {
                    sent.push(FeedEvent::Snapshot(self.reviews.clone()));
                }
            }
            ObservedEvent::TurnEnded(..) | ObservedEvent::UnreviewedChanges(..) => {}
        }
        let review_mode = self.review_mode;
        self.subscribers.retain(|subscriber| {
            sent.iter()
                .filter(|event| {
                    review_mode
                        || matches!(event, FeedEvent::Observed(ObservedEvent::ReviewMode(_)))
                })
                .all(|event| subscriber.unbounded_send(event.clone()).is_ok())
        });
    }
}

/// A private file's review keeps its ids and decision state but loses its path and hunks.
fn redact_review(mut review: ObservedReview, cx: &App) -> ObservedReview {
    if crate::workspace_edit::is_private_path(&review.abs_path, cx) {
        review.abs_path = PathBuf::new();
        review.hunks.clear();
    }
    review
}

fn redact_event(event: ObservedEvent, cx: &App) -> ObservedEvent {
    let private = |path: &PathBuf| crate::workspace_edit::is_private_path(path, cx);
    match event {
        ObservedEvent::Opened(review) => ObservedEvent::Opened(redact_review(review, cx)),
        ObservedEvent::Updated(review) => ObservedEvent::Updated(redact_review(review, cx)),
        ObservedEvent::Resolved(review) => ObservedEvent::Resolved(redact_review(review, cx)),
        ObservedEvent::TurnEnded(turn, files) => ObservedEvent::TurnEnded(
            turn,
            files
                .into_iter()
                .map(|mut file| {
                    if private(&file.abs_path) {
                        file.abs_path = PathBuf::new();
                    }
                    file
                })
                .collect(),
        ),
        ObservedEvent::UnreviewedChanges(turn, paths) => ObservedEvent::UnreviewedChanges(
            turn,
            paths.into_iter().filter(|path| !private(path)).collect(),
        ),
        event => event,
    }
}

#[derive(Clone, Default)]
struct AgentFeed(Arc<Mutex<Feed>>);

impl Global for AgentFeed {}

impl AgentFeed {
    fn lock(&self) -> std::sync::MutexGuard<'_, Feed> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The app's feed, observing `AgentReview` from the first call on; `None` without agent
    /// review (outside Fyx, some tests).
    fn global(cx: &mut App) -> Option<Self> {
        if let Some(feed) = cx.try_global::<Self>() {
            return Some(feed.clone());
        }
        let mut observation = agent_review::observe::observe(cx)?;
        let feed = Self::default();
        {
            let mut state = feed.lock();
            state.review_mode = observation.review_mode;
            state.reviews = std::mem::take(&mut observation.snapshot)
                .into_iter()
                .map(|review| redact_review(review, cx))
                .collect();
        }
        cx.spawn({
            let feed = feed.clone();
            async move |cx| {
                while let Some(event) = observation.events.next().await {
                    let event = cx.update(|cx| redact_event(event, cx));
                    feed.lock().apply(event);
                }
            }
        })
        .detach();
        cx.set_global(feed.clone());
        Some(feed)
    }
}

/// The [`AgentReviews`] of one load, or `None` without agent review. `key` is what
/// [`OpenWorktrees::for_extension`] was given, so its file refs match the extension's worktrees.
pub(crate) fn for_extension(key: &str, cx: &mut App) -> Option<Arc<dyn AgentReviews>> {
    let feed = AgentFeed::global(cx)?;
    Some(Arc::new(ExtensionReviews {
        feed,
        files: Files {
            key: key.into(),
            open: OpenWorktrees::global(cx),
        },
    }))
}

struct ExtensionReviews {
    feed: AgentFeed,
    files: Files,
}

impl AgentReviews for ExtensionReviews {
    /// Needs no main-thread trip: the feed's mirror and fan-out live under one lock, and the
    /// observation itself started on the main thread when the extension loaded.
    fn subscribe(&self) -> BoxFuture<'static, (Vec<ReviewInfo>, BoxStream<'static, AgentEvent>)> {
        let (sender, events) = mpsc::unbounded();
        let (review_mode, reviews) = {
            let mut feed = self.feed.lock();
            feed.subscribers.push(sender);
            let reviews = if feed.review_mode {
                feed.reviews.clone()
            } else {
                Vec::new()
            };
            (feed.review_mode, reviews)
        };
        let snapshot = reviews
            .iter()
            .map(|review| self.files.review(review))
            .collect();
        let files = self.files.clone();
        let first = futures::stream::once(async move { AgentEvent::ReviewMode(review_mode) });
        let rest = events.map(move |event| files.event(event));
        futures::future::ready((snapshot, first.chain(rest).boxed())).boxed()
    }
}

/// Maps observed data to the extension's view of it.
#[derive(Clone)]
struct Files {
    key: Arc<str>,
    open: OpenWorktrees,
}

impl Files {
    fn event(&self, event: FeedEvent) -> AgentEvent {
        match event {
            FeedEvent::Snapshot(reviews) => AgentEvent::Snapshot {
                reviews: reviews.iter().map(|review| self.review(review)).collect(),
                older_dropped: false,
            },
            FeedEvent::Observed(event) => match event {
                ObservedEvent::Opened(review) => AgentEvent::Opened(self.review(&review)),
                ObservedEvent::Updated(review) => AgentEvent::Updated(self.review(&review)),
                ObservedEvent::Resolved(review) => AgentEvent::Resolved(self.review(&review)),
                ObservedEvent::Closed(id) => AgentEvent::Closed(id),
                ObservedEvent::TurnEnded(turn, files) => AgentEvent::TurnEnded(
                    turn,
                    files.iter().map(|file| self.turn_file(file)).collect(),
                ),
                ObservedEvent::UnreviewedChanges(turn, paths) => AgentEvent::UnreviewedChanges(
                    turn,
                    paths
                        .iter()
                        .filter_map(|path| self.open.file_ref(&self.key, path))
                        .collect(),
                ),
                ObservedEvent::ReviewMode(on) => AgentEvent::ReviewMode(on),
            },
        }
    }

    fn review(&self, review: &ObservedReview) -> ReviewInfo {
        let file = self.open.file_ref(&self.key, &review.abs_path);
        ReviewInfo {
            review: review.id,
            session: review.session,
            turn: review.turn,
            source: match review.source {
                ObservedSource::Permission => ReviewSource::Permission,
                ObservedSource::Write => ReviewSource::Write,
            },
            rule: review.rule.map(rule_kind),
            resolution: resolution(review.resolution),
            submitted: review.submitted,
            // A delete, move or copy's one hunk describes the operation, not text of the file.
            hunks: if review.operation || file.is_none() {
                Vec::new()
            } else {
                review.hunks.iter().map(hunk).collect()
            },
            truncated: false,
            file,
        }
    }

    fn turn_file(&self, file: &ObservedTurnFile) -> TurnFile {
        TurnFile {
            file: self.open.file_ref(&self.key, &file.abs_path),
            resolution: resolution(file.resolution),
            rule: file.rule.map(rule_kind),
        }
    }
}

fn hunk(hunk: &ObservedHunk) -> HunkInfo {
    HunkInfo {
        old_start: hunk.old_start,
        old_count: hunk.old_count,
        new_start: hunk.new_start,
        new_count: hunk.new_count,
        old_text: hunk.old_text.clone(),
        new_text: hunk.new_text.clone(),
        decision: match hunk.decision {
            agent_review::HunkDecision::Pending => HunkDecision::Pending,
            agent_review::HunkDecision::Accepted => HunkDecision::Accepted,
            agent_review::HunkDecision::Rejected => HunkDecision::Rejected,
        },
        comment: hunk.comment.clone(),
    }
}

fn rule_kind(kind: agent_review::RuleKind) -> RuleKind {
    match kind {
        agent_review::RuleKind::AutoAccept => RuleKind::AutoAccept,
        agent_review::RuleKind::AlwaysAsk => RuleKind::AlwaysAsk,
        agent_review::RuleKind::Deny => RuleKind::Deny,
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn resolution(resolution: agent_review::Resolution) -> Resolution {
    match resolution {
        agent_review::Resolution::Unchanged => Resolution::Unchanged,
        agent_review::Resolution::Pending { undecided } => Resolution::Pending(count(undecided)),
        agent_review::Resolution::AcceptedAll => Resolution::AcceptedAll,
        agent_review::Resolution::RejectedAll => Resolution::RejectedAll,
        agent_review::Resolution::Partial { accepted, rejected } => {
            Resolution::Partial(count(accepted), count(rejected))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::EntityId;

    use super::*;
    use crate::worktrees::OpenWorktree;

    fn review(id: u64, path: &str) -> ObservedReview {
        ObservedReview {
            id,
            session: 1,
            turn: 1,
            abs_path: PathBuf::from(path),
            source: ObservedSource::Write,
            rule: None,
            resolution: agent_review::Resolution::Pending { undecided: 1 },
            submitted: false,
            operation: false,
            hunks: vec![ObservedHunk {
                old_start: 0,
                old_count: 1,
                new_start: 0,
                new_count: 1,
                old_text: "a\n".into(),
                new_text: "b\n".into(),
                decision: agent_review::HunkDecision::Pending,
                comment: None,
            }],
        }
    }

    fn reviews(feed: AgentFeed, open: OpenWorktrees) -> ExtensionReviews {
        ExtensionReviews {
            feed,
            files: Files {
                key: "ext".into(),
                open,
            },
        }
    }

    #[test]
    fn test_snapshot_then_changes_with_file_refs() {
        let open = OpenWorktrees::default();
        open.set(
            EntityId::from(1u64),
            vec![OpenWorktree {
                key: (1, 1),
                root: PathBuf::from("/work"),
                trusted: true,
            }],
        );
        let feed = AgentFeed::default();
        {
            let mut state = feed.lock();
            state.review_mode = true;
            state.reviews = vec![review(1, "/work/a.rs")];
        }
        let reviews = reviews(feed.clone(), open);
        let (snapshot, mut events) = futures::executor::block_on(reviews.subscribe());
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].file.as_ref().unwrap().path, "a.rs");
        assert_eq!(snapshot[0].hunks.len(), 1);
        assert_eq!(snapshot[0].resolution, Resolution::Pending(1));

        feed.lock()
            .apply(ObservedEvent::Opened(review(2, "/etc/hosts")));
        feed.lock().apply(ObservedEvent::Closed(1));
        let mut operation = review(3, "/work/b.rs");
        operation.operation = true;
        feed.lock().apply(ObservedEvent::Opened(operation));
        drop(reviews);

        let received: Vec<AgentEvent> = futures::executor::block_on(async {
            let mut collected = Vec::new();
            for _ in 0..4 {
                collected.push(events.next().await.unwrap());
            }
            collected
        });
        assert_eq!(received[0], AgentEvent::ReviewMode(true));
        let AgentEvent::Opened(outside) = &received[1] else {
            panic!("{:?}", received[1]);
        };
        assert_eq!(outside.file, None, "outside every worktree");
        assert!(outside.hunks.is_empty(), "no hunks outside the worktrees");
        assert_eq!(received[2], AgentEvent::Closed(1));
        let AgentEvent::Opened(operation) = &received[3] else {
            panic!("{:?}", received[3]);
        };
        assert!(operation.hunks.is_empty(), "an operation has no text hunks");
        assert_eq!(
            feed.lock()
                .reviews
                .iter()
                .map(|review| review.id)
                .collect::<Vec<_>>(),
            [2, 3]
        );
    }

    #[test]
    fn test_review_mode_off_sends_only_the_mode() {
        let feed = AgentFeed::default();
        feed.lock().reviews = vec![review(1, "/work/a.rs")];
        let reviews = reviews(feed.clone(), OpenWorktrees::default());
        let (snapshot, mut events) = futures::executor::block_on(reviews.subscribe());
        assert!(snapshot.is_empty(), "review mode off: no reviews");

        feed.lock()
            .apply(ObservedEvent::Opened(review(2, "/work/b.rs")));
        feed.lock().apply(ObservedEvent::ReviewMode(true));
        feed.lock().apply(ObservedEvent::ReviewMode(false));
        feed.lock().apply(ObservedEvent::Closed(2));

        let received: Vec<AgentEvent> = futures::executor::block_on(async {
            let mut collected = Vec::new();
            for _ in 0..4 {
                collected.push(events.next().await.unwrap());
            }
            collected
        });
        assert_eq!(received[0], AgentEvent::ReviewMode(false));
        assert_eq!(received[1], AgentEvent::ReviewMode(true));
        let AgentEvent::Snapshot {
            reviews: snapshot, ..
        } = &received[2]
        else {
            panic!("{:?}", received[2]);
        };
        assert_eq!(
            snapshot
                .iter()
                .map(|review| review.review)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(received[3], AgentEvent::ReviewMode(false));
        assert!(
            futures::FutureExt::now_or_never(events.next()).is_none(),
            "the close while off is not sent"
        );
    }
}
