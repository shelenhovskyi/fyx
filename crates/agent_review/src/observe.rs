//! The read-only view of agent reviews for `agent:observe` (plan 4.8, docs/fyx/wit-0.3/design.md §11):
//! a snapshot of the reviews and, with no gap after it, their changes, as plain data. Nothing here
//! can decide a review; `ui_ext_host` wraps it for extensions.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc};

use agent_client_protocol::v2::schema as acp_v2;
use futures::channel::mpsc;
use gpui::{App, Entity, Subscription};

use crate::{
    AgentReview, AgentReviewEvent, HunkDecision, Resolution, Review, ReviewSource, RuleKind,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedHunk {
    /// Rows, 0-based, of the old and the new text.
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub old_text: String,
    pub new_text: String,
    pub decision: HunkDecision,
    pub comment: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedSource {
    Permission,
    Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedReview {
    /// Stable for the app's run, never reused.
    pub id: u64,
    /// Numbered per [`Observation`], in the order sessions are first seen.
    pub session: u64,
    pub turn: u64,
    pub abs_path: PathBuf,
    pub source: ObservedSource,
    /// The kind of rule that decided or asked for the review; its pattern and file are left out.
    pub rule: Option<RuleKind>,
    pub resolution: Resolution,
    pub submitted: bool,
    /// A delete, move or copy of Zed's built-in agent: its one hunk describes the operation (a deleted
    /// text file's hunk is its content).
    pub operation: bool,
    pub hunks: Vec<ObservedHunk>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedTurnFile {
    pub abs_path: PathBuf,
    pub resolution: Resolution,
    pub rule: Option<RuleKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservedEvent {
    Opened(ObservedReview),
    Updated(ObservedReview),
    Resolved(ObservedReview),
    Closed(u64),
    /// The turn's decided files, in the order they were decided.
    TurnEnded(u64, Vec<ObservedTurnFile>),
    /// Every file of the turn that changed on disk without review so far.
    UnreviewedChanges(u64, Vec<PathBuf>),
    ReviewMode(bool),
}

/// What [`observe`] returns: the reviews now (oldest first) and every change after them. An event may
/// repeat what the snapshot already shows; none is missed. Dropping it ends the subscription.
pub struct Observation {
    pub review_mode: bool,
    pub snapshot: Vec<ObservedReview>,
    pub events: mpsc::UnboundedReceiver<ObservedEvent>,
    _subscription: Subscription,
}

#[derive(Default)]
struct SessionNumbers(HashMap<acp_v2::SessionId, u64>);

impl SessionNumbers {
    fn number(&mut self, session: &acp_v2::SessionId) -> u64 {
        let next = self.0.len() as u64 + 1;
        *self.0.entry(session.clone()).or_insert(next)
    }
}

/// Starts observing the agent reviews, or `None` without agent review (outside Fyx, some tests).
pub fn observe(cx: &mut App) -> Option<Observation> {
    let review = AgentReview::global(cx)?;
    let sessions = Rc::new(RefCell::new(SessionNumbers::default()));
    let (review_mode, snapshot) = {
        let store = review.read(cx);
        let mut sessions = sessions.borrow_mut();
        let snapshot = store
            .reviews()
            .iter()
            .map(|review| observed(review, &mut sessions))
            .collect();
        (store.is_enabled(), snapshot)
    };
    let (sender, events) = mpsc::unbounded();
    // Subscribed in the same synchronous step as the snapshot was read, so no change falls between.
    let subscription = cx.subscribe(&review, move |review: Entity<AgentReview>, event, cx| {
        let store = review.read(cx);
        let mut sessions = sessions.borrow_mut();
        let find = |id| store.review(id).or_else(|| store.departed_review(id));
        let observed_event = match *event {
            AgentReviewEvent::Opened(id) => {
                find(id).map(|review| ObservedEvent::Opened(observed(review, &mut sessions)))
            }
            AgentReviewEvent::Updated(id) => {
                find(id).map(|review| ObservedEvent::Updated(observed(review, &mut sessions)))
            }
            AgentReviewEvent::Resolved(id) => {
                find(id).map(|review| ObservedEvent::Resolved(observed(review, &mut sessions)))
            }
            AgentReviewEvent::Closed(id) => Some(ObservedEvent::Closed(id.as_u64())),
            AgentReviewEvent::TurnEnded(turn) => Some(ObservedEvent::TurnEnded(
                turn.as_u64(),
                store
                    .turn_history(turn)
                    .iter()
                    .map(|entry| ObservedTurnFile {
                        abs_path: entry.path.clone(),
                        resolution: entry.resolution,
                        rule: entry.rule.as_ref().map(|rule| rule.kind),
                    })
                    .collect(),
            )),
            AgentReviewEvent::UnreviewedChanges(turn) => Some(ObservedEvent::UnreviewedChanges(
                turn.as_u64(),
                store.unreviewed_changes(turn).cloned().collect(),
            )),
            AgentReviewEvent::ModeChanged(enabled) => Some(ObservedEvent::ReviewMode(enabled)),
            // Shown to the user; observers see the decision in the turn's files.
            AgentReviewEvent::DecidedByRule { .. } => None,
        };
        if let Some(observed_event) = observed_event {
            sender.unbounded_send(observed_event).ok();
        }
    });
    Some(Observation {
        review_mode,
        snapshot,
        events,
        _subscription: subscription,
    })
}

fn observed(review: &Review, sessions: &mut SessionNumbers) -> ObservedReview {
    let edit = review.edit();
    let hunks = (0..edit.hunks().len())
        .filter_map(|index| {
            let hunk = &edit.hunks()[index];
            let (old_text, new_text) = edit.hunk_texts(index)?;
            Some(ObservedHunk {
                old_start: hunk.old_rows.start,
                old_count: hunk.old_rows.end - hunk.old_rows.start,
                new_start: hunk.new_rows.start,
                new_count: hunk.new_rows.end - hunk.new_rows.start,
                old_text: old_text.to_string(),
                new_text: new_text.to_string(),
                decision: hunk.decision,
                comment: hunk.comment.clone(),
            })
        })
        .collect();
    ObservedReview {
        id: review.id().as_u64(),
        session: sessions.number(review.session_id()),
        turn: review.turn().as_u64(),
        abs_path: edit.abs_path().to_path_buf(),
        source: match review.source() {
            ReviewSource::Permission(_) => ObservedSource::Permission,
            // A proposal holds a write the same way; the snapshot has no own kind for it yet.
            ReviewSource::Write | ReviewSource::Extension { .. } => ObservedSource::Write,
        },
        rule: review.rule().map(|rule| rule.kind),
        resolution: edit.resolution(),
        submitted: review.is_submitted(),
        operation: review.operation().is_some(),
        hunks,
    }
}

#[cfg(test)]
mod tests {
    use acp_thread::fyx_review::{AgentWrite, AgentWriteGate as _};
    use gpui::{AppContext as _, TestAppContext};
    use language::Buffer;

    use super::*;

    const OLD: &str = "one\ntwo\nthree\nfour\nfive\n";
    const NEW: &str = "ONE\ntwo\nthree\nfour\nFIVE\n";

    fn write(path: &str, cx: &mut TestAppContext) -> AgentWrite {
        AgentWrite {
            session_id: acp_v2::SessionId::new("session"),
            abs_path: PathBuf::from(path),
            buffer: cx.new(|cx| Buffer::local(OLD, cx)),
            old_text: OLD.into(),
            new_text: NEW.into(),
        }
    }

    #[gpui::test]
    async fn a_snapshot_then_every_change(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(None, cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let gate = crate::WriteGate(review.downgrade());
        let agent_write = write("/p/a.txt", cx);
        let first = cx.update(|cx| gate.review_write(agent_write, cx));
        cx.run_until_parked();

        let mut observation = cx.update(|cx| observe(cx)).unwrap();
        assert!(observation.review_mode);
        let [snapshot] = observation.snapshot.as_slice() else {
            panic!("one review: {:?}", observation.snapshot);
        };
        assert_eq!(snapshot.abs_path, PathBuf::from("/p/a.txt"));
        assert_eq!(snapshot.source, ObservedSource::Write);
        assert_eq!(snapshot.resolution, Resolution::Pending { undecided: 2 });
        assert_eq!(snapshot.session, 1);
        let hunk = &snapshot.hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_count,
                hunk.new_start,
                hunk.new_count
            ),
            (0, 1, 0, 1)
        );
        assert_eq!(
            (hunk.old_text.as_str(), hunk.new_text.as_str()),
            ("one\n", "ONE\n")
        );

        // Accepting the turn resolves the held write in one update; the events still carry the
        // final decisions.
        let id = crate::ReviewId(snapshot.id);
        let turn = review.read_with(cx, |review, _| review.review(id).unwrap().turn());
        review.update(cx, |review, cx| {
            review
                .decide_turn(turn, HunkDecision::Accepted, cx)
                .unwrap();
            review.end_turn(&acp_v2::SessionId::new("session"), cx);
            review.set_enabled(false, cx);
        });
        cx.run_until_parked();
        assert_eq!(first.await.unwrap().content.as_deref(), Some(NEW));

        let mut events = Vec::new();
        while let Ok(event) = observation.events.try_recv() {
            events.push(event);
        }
        let ObservedEvent::Resolved(resolved) = &events[0] else {
            panic!("{events:?}");
        };
        assert_eq!(resolved.resolution, Resolution::AcceptedAll);
        assert!(resolved.submitted);
        assert_eq!(events[1], ObservedEvent::Closed(snapshot.id));
        let ObservedEvent::TurnEnded(ended, files) = &events[2] else {
            panic!("{events:?}");
        };
        assert_eq!(*ended, turn.as_u64());
        assert_eq!(files[0].resolution, Resolution::AcceptedAll);
        assert_eq!(events[3], ObservedEvent::ReviewMode(false));
    }
}
