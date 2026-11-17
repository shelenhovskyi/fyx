//! Host-owned review of agent-proposed edits.
//!
//! Agent writes reach [`AgentReview`] through the gate in `acp_thread::fyx_review`; permission requests
//! that carry a diff are observed on every [`AcpThread`]. Only the host resolves a review: extensions may
//! observe [`AgentReviewEvent`]s and read reviews, but there is no path for them to decide whether an
//! edit lands.

mod proposed_edit;

use std::rc::Rc;

use acp_thread::{
    AcpThread, AcpThreadEvent, PermissionRequestId, SelectedPermissionOutcome, ToolCallContent,
    fyx_review::{AgentWrite, AgentWriteGate, AgentWriteOutcome, set_agent_write_gate},
};
use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task, WeakEntity};

pub use proposed_edit::{Hunk, HunkDecision, ProposedEdit, Resolution};

pub fn init(cx: &mut App) {
    let review = cx.new(|_| AgentReview::default());
    cx.set_global(GlobalAgentReview(review.clone()));
    set_agent_write_gate(Rc::new(WriteGate(review.downgrade())), cx);
    cx.observe_new(|_: &mut AcpThread, _, cx: &mut Context<AcpThread>| {
        cx.subscribe_self(on_thread_event).detach();
    })
    .detach();
}

struct GlobalAgentReview(Entity<AgentReview>);

impl Global for GlobalAgentReview {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReviewId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewSource {
    /// A diff shown with an ACP permission request, before the agent writes anything.
    Permission(PermissionRequestId),
    /// An agent's `fs/write_text_file`, held until the review is submitted.
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionVerdict {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentReviewEvent {
    Opened(ReviewId),
    Updated(ReviewId),
    Resolved(ReviewId),
    Closed(ReviewId),
}

type PermissionResponder = Box<dyn FnOnce(PermissionVerdict, &mut App)>;

enum Responder {
    Permission(PermissionResponder),
    Write(oneshot::Sender<AgentWriteOutcome>),
}

pub struct Review {
    id: ReviewId,
    session_id: acp_v2::SessionId,
    source: ReviewSource,
    edit: ProposedEdit,
    responder: Option<Responder>,
}

impl Review {
    pub fn id(&self) -> ReviewId {
        self.id
    }

    pub fn session_id(&self) -> &acp_v2::SessionId {
        &self.session_id
    }

    pub fn source(&self) -> ReviewSource {
        self.source
    }

    pub fn edit(&self) -> &ProposedEdit {
        &self.edit
    }

    pub fn is_submitted(&self) -> bool {
        self.responder.is_none()
    }
}

/// Open reviews of agent edits. Review mode is off by default: while it is off, agent writes and
/// permission requests behave as upstream.
#[derive(Default)]
pub struct AgentReview {
    enabled: bool,
    reviews: Vec<Review>,
    next_id: u64,
}

impl EventEmitter<AgentReviewEvent> for AgentReview {}

impl AgentReview {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalAgentReview>()
            .map(|global| global.0.clone())
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Turning review mode off leaves open reviews in place; writes already held still wait for a
    /// decision, so nothing lands without one.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.enabled = enabled;
        cx.notify();
    }

    pub fn reviews(&self) -> &[Review] {
        &self.reviews
    }

    pub fn review(&self, id: ReviewId) -> Option<&Review> {
        self.reviews.iter().find(|review| review.id == id)
    }

    pub fn set_decision(
        &mut self,
        id: ReviewId,
        hunk_index: usize,
        decision: HunkDecision,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.open_review_mut(id)?
            .edit
            .set_decision(hunk_index, decision)?;
        cx.emit(AgentReviewEvent::Updated(id));
        cx.notify();
        Ok(())
    }

    pub fn decide_all(
        &mut self,
        id: ReviewId,
        decision: HunkDecision,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.open_review_mut(id)?.edit.decide_all(decision);
        cx.emit(AgentReviewEvent::Updated(id));
        cx.notify();
        Ok(())
    }

    pub fn set_comment(
        &mut self,
        id: ReviewId,
        hunk_index: usize,
        comment: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.open_review_mut(id)?
            .edit
            .set_comment(hunk_index, comment)?;
        cx.emit(AgentReviewEvent::Updated(id));
        cx.notify();
        Ok(())
    }

    /// Delivers the decisions. Every hunk must be decided first.
    pub fn submit(&mut self, id: ReviewId, cx: &mut Context<Self>) -> Result<()> {
        let review = self.open_review_mut(id)?;
        if let Resolution::Pending { undecided } = review.edit.resolution() {
            return Err(anyhow!("{undecided} hunks are still undecided"));
        }
        let responder = review
            .responder
            .take()
            .context("review was already submitted")?;
        match responder {
            Responder::Write(sender) => {
                let outcome = write_outcome(&review.edit)?;
                if sender.send(outcome).is_err() {
                    log::debug!("agent write finished before its review was submitted");
                }
                self.reviews.retain(|review| review.id != id);
                cx.emit(AgentReviewEvent::Resolved(id));
                cx.emit(AgentReviewEvent::Closed(id));
            }
            Responder::Permission(respond) => {
                let verdict = permission_verdict(review.edit.resolution());
                // Answering the permission makes the thread emit an event this entity handles, so it
                // must not run while this entity is being updated.
                cx.defer(move |cx| respond(verdict, cx));
                cx.emit(AgentReviewEvent::Resolved(id));
                // An allowed preview stays until the agent's write adopts its decisions; after a
                // denial no write follows.
                if verdict == PermissionVerdict::Deny {
                    self.reviews.retain(|review| review.id != id);
                    cx.emit(AgentReviewEvent::Closed(id));
                }
            }
        }
        cx.notify();
        Ok(())
    }

    /// Closes a review without a decision. A held write then fails without touching the file.
    pub fn close(&mut self, id: ReviewId, cx: &mut Context<Self>) {
        let count = self.reviews.len();
        self.reviews.retain(|review| review.id != id);
        if self.reviews.len() != count {
            cx.emit(AgentReviewEvent::Closed(id));
            cx.notify();
        }
    }

    fn open_review_mut(&mut self, id: ReviewId) -> Result<&mut Review> {
        let review = self
            .reviews
            .iter_mut()
            .find(|review| review.id == id)
            .with_context(|| format!("no review {id:?}"))?;
        anyhow::ensure!(
            review.responder.is_some(),
            "review {id:?} was already submitted"
        );
        Ok(review)
    }

    fn insert(
        &mut self,
        session_id: acp_v2::SessionId,
        source: ReviewSource,
        edit: ProposedEdit,
        responder: Responder,
        cx: &mut Context<Self>,
    ) -> ReviewId {
        let id = ReviewId(self.next_id);
        self.next_id += 1;
        self.reviews.push(Review {
            id,
            session_id,
            source,
            edit,
            responder: Some(responder),
        });
        cx.emit(AgentReviewEvent::Opened(id));
        cx.notify();
        id
    }

    pub fn open_permission_preview(
        &mut self,
        session_id: acp_v2::SessionId,
        request_id: PermissionRequestId,
        edit: ProposedEdit,
        respond: PermissionResponder,
        cx: &mut Context<Self>,
    ) -> Option<ReviewId> {
        if !self.enabled || edit.hunks().is_empty() {
            return None;
        }
        // A newer preview of the same file supersedes allowed ones whose write never came.
        let superseded = self
            .reviews
            .iter()
            .filter(|review| {
                matches!(review.source, ReviewSource::Permission(_))
                    && review.is_submitted()
                    && review.session_id == session_id
                    && review.edit.abs_path() == edit.abs_path()
            })
            .map(|review| review.id)
            .collect::<Vec<_>>();
        for id in superseded {
            self.close(id, cx);
        }
        Some(self.insert(
            session_id,
            ReviewSource::Permission(request_id),
            edit,
            Responder::Permission(respond),
            cx,
        ))
    }

    /// Drops an undecided preview once its permission request was answered elsewhere.
    fn permission_answered(&mut self, request_id: PermissionRequestId, cx: &mut Context<Self>) {
        let undecided = self
            .reviews
            .iter()
            .filter(|review| {
                review.source == ReviewSource::Permission(request_id) && review.responder.is_some()
            })
            .map(|review| review.id)
            .collect::<Vec<_>>();
        for id in undecided {
            self.close(id, cx);
        }
    }

    pub fn review_write(
        &mut self,
        write: AgentWrite,
        cx: &mut Context<Self>,
    ) -> Task<Result<AgentWriteOutcome>> {
        if !self.enabled {
            return Task::ready(Ok(AgentWriteOutcome {
                content: Some(write.new_text),
                feedback: None,
            }));
        }
        let mut edit = ProposedEdit::new(write.abs_path, write.old_text, write.new_text);
        let previews = self
            .reviews
            .iter()
            .filter(|review| {
                matches!(review.source, ReviewSource::Permission(_))
                    && review.is_submitted()
                    && review.session_id == write.session_id
                    && review.edit.abs_path() == edit.abs_path()
            })
            .map(|review| review.id)
            .collect::<Vec<_>>();
        for id in previews.iter().rev() {
            if let Some(preview) = self.review(*id) {
                edit.adopt_decisions(&preview.edit);
            }
            self.close(*id, cx);
        }

        if !edit.resolution().is_pending() {
            return Task::ready(write_outcome(&edit));
        }
        let (sender, receiver) = oneshot::channel();
        self.insert(
            write.session_id,
            ReviewSource::Write,
            edit,
            Responder::Write(sender),
            cx,
        );
        cx.spawn(async move |_, _| {
            receiver
                .await
                .map_err(|_| anyhow!("The review of this edit was closed; the file is unchanged."))
        })
    }
}

fn permission_verdict(resolution: Resolution) -> PermissionVerdict {
    match resolution {
        Resolution::RejectedAll => PermissionVerdict::Deny,
        _ => PermissionVerdict::Allow,
    }
}

fn write_outcome(edit: &ProposedEdit) -> Result<AgentWriteOutcome> {
    let content = match edit.resolution() {
        Resolution::RejectedAll => None,
        _ => Some(edit.apply_accepted()?),
    };
    Ok(AgentWriteOutcome {
        content,
        feedback: edit.feedback(),
    })
}

struct WriteGate(WeakEntity<AgentReview>);

impl AgentWriteGate for WriteGate {
    fn review_write(&self, write: AgentWrite, cx: &mut App) -> Task<Result<AgentWriteOutcome>> {
        match self.0.upgrade() {
            Some(review) => review.update(cx, |review, cx| review.review_write(write, cx)),
            None => Task::ready(Err(anyhow!(
                "Agent review is unavailable; the file is unchanged."
            ))),
        }
    }
}

fn on_thread_event(thread: &mut AcpThread, event: &AcpThreadEvent, cx: &mut Context<AcpThread>) {
    let Some(review) = AgentReview::global(cx) else {
        return;
    };
    match event {
        AcpThreadEvent::ToolAuthorizationRequested(request_id) => {
            if !review.read(cx).is_enabled() {
                return;
            }
            let Some((edit, respond)) = permission_preview(thread, *request_id, cx) else {
                return;
            };
            let session_id = thread.session_id().clone();
            review.update(cx, |review, cx| {
                review.open_permission_preview(session_id, *request_id, edit, respond, cx);
            });
        }
        AcpThreadEvent::ToolAuthorizationReceived(request_id) => {
            review.update(cx, |review, cx| review.permission_answered(*request_id, cx));
        }
        _ => {}
    }
}

/// The diff of a pending edit permission and the host's way to answer it. Only legacy (v1) diffs carry
/// whole old and new texts; v2 patch diffs are reviewed when the agent writes.
fn permission_preview(
    thread: &AcpThread,
    request_id: PermissionRequestId,
    cx: &Context<AcpThread>,
) -> Option<(ProposedEdit, PermissionResponder)> {
    let request = thread.permission_request(request_id)?;
    let options = request.legacy_options()?;
    let allow_option = options.allow_once_option_id()?;
    let deny_option = options.deny_once_option_id()?;
    let (_, tool_call) = thread.tool_call(request.legacy_tool_call_id()?)?;
    let diff = tool_call
        .content()
        .iter()
        .find_map(|content| match content {
            ToolCallContent::LegacyDiff { source, .. } => Some(source),
            _ => None,
        })?;
    let edit = ProposedEdit::new(
        diff.path.clone(),
        diff.old_text.clone().unwrap_or_default(),
        diff.new_text.clone(),
    );
    let thread = cx.weak_entity();
    let respond: PermissionResponder = Box::new(move |verdict, cx| {
        let (option_id, option_kind) = match verdict {
            PermissionVerdict::Allow => (allow_option, acp_v2::PermissionOptionKind::AllowOnce),
            PermissionVerdict::Deny => (deny_option, acp_v2::PermissionOptionKind::RejectOnce),
        };
        let outcome = SelectedPermissionOutcome::new(option_id, option_kind);
        if let Err(error) = thread.update(cx, |thread, cx| {
            thread.authorize_permission_request(request_id, outcome, cx)
        }) {
            log::debug!("thread closed before its edit review was submitted: {error}");
        }
    });
    Some((edit, respond))
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::Path, path::PathBuf};

    use super::*;
    use gpui::TestAppContext;
    use language::Buffer;

    const OLD: &str = "one\ntwo\nthree\nfour\nfive\n";
    const NEW: &str = "ONE\ntwo\nthree\nfour\nFIVE\n";

    fn session() -> acp_v2::SessionId {
        acp_v2::SessionId::new("session")
    }

    fn agent_write(path: &str, cx: &mut TestAppContext) -> AgentWrite {
        AgentWrite {
            session_id: session(),
            abs_path: PathBuf::from(path),
            buffer: cx.new(|cx| Buffer::local(OLD, cx)),
            old_text: OLD.into(),
            new_text: NEW.into(),
        }
    }

    fn new_request_id() -> PermissionRequestId {
        serde_json::from_value(serde_json::Value::String(uuid::Uuid::new_v4().to_string())).unwrap()
    }

    fn setup(cx: &mut TestAppContext) -> (Entity<AgentReview>, Rc<RefCell<Vec<AgentReviewEvent>>>) {
        let review = cx.new(|_| AgentReview::default());
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = events.clone();
            cx.subscribe(&review, move |_, event, _| events.borrow_mut().push(*event))
                .detach();
        });
        (review, events)
    }

    fn gate_write(
        review: &Entity<AgentReview>,
        write: AgentWrite,
        cx: &mut TestAppContext,
    ) -> Task<Result<AgentWriteOutcome>> {
        let gate = WriteGate(review.downgrade());
        cx.update(|cx| gate.review_write(write, cx))
    }

    fn only_review_id(review: &Entity<AgentReview>, cx: &mut TestAppContext) -> ReviewId {
        review.read_with(cx, |review, _| {
            assert_eq!(review.reviews().len(), 1);
            review.reviews()[0].id()
        })
    }

    #[gpui::test]
    async fn disabled_review_passes_writes_through(cx: &mut TestAppContext) {
        let (review, events) = setup(cx);
        let write = agent_write("/p/a.txt", cx);
        let outcome = gate_write(&review, write, cx).await.unwrap();
        assert_eq!(outcome.content.as_deref(), Some(NEW));
        assert_eq!(outcome.feedback, None);
        assert!(events.borrow().is_empty());
    }

    #[gpui::test]
    async fn accept_one_hunk_and_reject_another(cx: &mut TestAppContext) {
        let (review, events) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let write = agent_write("/p/a.txt", cx);
        let task = gate_write(&review, write, cx);
        let id = only_review_id(&review, cx);
        review.read_with(cx, |review, _| {
            let review = review.review(id).unwrap();
            assert_eq!(review.source(), ReviewSource::Write);
            assert_eq!(review.edit().hunks().len(), 2);
            assert_eq!(review.edit().abs_path(), Path::new("/p/a.txt"));
        });

        review.update(cx, |review, cx| {
            review
                .set_decision(id, 0, HunkDecision::Accepted, cx)
                .unwrap();
            assert!(review.submit(id, cx).is_err(), "hunk 2 is undecided");
            review
                .set_decision(id, 1, HunkDecision::Rejected, cx)
                .unwrap();
            review
                .set_comment(id, 1, Some("leave five alone".into()), cx)
                .unwrap();
            review.submit(id, cx).unwrap();
            assert!(review.reviews().is_empty());
            assert!(
                review
                    .set_decision(id, 0, HunkDecision::Rejected, cx)
                    .is_err()
            );
        });

        let outcome = task.await.unwrap();
        assert_eq!(
            outcome.content.as_deref(),
            Some("ONE\ntwo\nthree\nfour\nfive\n")
        );
        let feedback = outcome.feedback.unwrap();
        assert!(feedback.contains("applied 1 of 2 hunks"), "{feedback}");
        assert!(feedback.contains("leave five alone"), "{feedback}");
        assert_eq!(
            events.borrow().first().copied(),
            Some(AgentReviewEvent::Opened(id))
        );
        assert_eq!(
            events.borrow()[events.borrow().len() - 2..],
            [AgentReviewEvent::Resolved(id), AgentReviewEvent::Closed(id)]
        );
    }

    #[gpui::test]
    async fn rejecting_everything_writes_nothing(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let write = agent_write("/p/a.txt", cx);
        let task = gate_write(&review, write, cx);
        let id = only_review_id(&review, cx);
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Rejected, cx).unwrap();
            review.submit(id, cx).unwrap();
        });
        let outcome = task.await.unwrap();
        assert_eq!(outcome.content, None);
        assert!(outcome.feedback.unwrap().contains("the file is unchanged"));
    }

    #[gpui::test]
    async fn closing_a_review_fails_the_write(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let write = agent_write("/p/a.txt", cx);
        let task = gate_write(&review, write, cx);
        let id = only_review_id(&review, cx);
        review.update(cx, |review, cx| review.close(id, cx));
        assert!(task.await.is_err());

        let write = agent_write("/p/a.txt", cx);
        let task = gate_write(&review, write, cx);
        drop(review);
        // Dropped entities are released when effects are flushed.
        cx.update(|_| {});
        cx.run_until_parked();
        assert!(task.await.is_err(), "dropping the store must fail closed");
    }

    #[gpui::test]
    async fn permission_preview_answers_the_request_and_carries_decisions(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let verdicts = Rc::new(RefCell::new(Vec::new()));
        let respond: PermissionResponder = Box::new({
            let verdicts = verdicts.clone();
            move |verdict, _| verdicts.borrow_mut().push(verdict)
        });
        // The agent proposes a fragment of the file, as Claude's Edit tool does.
        let preview = ProposedEdit::new(
            PathBuf::from("/p/a.txt"),
            "one\ntwo".into(),
            "ONE\ntwo".into(),
        );
        let request_id = new_request_id();
        let id = review
            .update(cx, |review, cx| {
                review.open_permission_preview(session(), request_id, preview, respond, cx)
            })
            .unwrap();
        review.update(cx, |review, cx| {
            review
                .set_decision(id, 0, HunkDecision::Accepted, cx)
                .unwrap();
            review.submit(id, cx).unwrap();
        });
        cx.run_until_parked();
        assert_eq!(*verdicts.borrow(), [PermissionVerdict::Allow]);

        // The write contains the previewed hunk plus one the agent did not show.
        let write = agent_write("/p/a.txt", cx);
        let task = gate_write(&review, write, cx);
        let write_id = only_review_id(&review, cx);
        review.update(cx, |review, cx| {
            let edit = review.review(write_id).unwrap().edit();
            assert_eq!(edit.hunks()[0].decision, HunkDecision::Accepted);
            assert_eq!(edit.hunks()[1].decision, HunkDecision::Pending);
            review
                .set_decision(write_id, 1, HunkDecision::Accepted, cx)
                .unwrap();
            review.submit(write_id, cx).unwrap();
        });
        let outcome = task.await.unwrap();
        assert_eq!(outcome.content.as_deref(), Some(NEW));
        assert_eq!(outcome.feedback, None);
    }

    #[gpui::test]
    async fn fully_decided_preview_resolves_the_write_without_a_review(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let verdicts = Rc::new(RefCell::new(Vec::new()));
        let respond: PermissionResponder = Box::new({
            let verdicts = verdicts.clone();
            move |verdict, _| verdicts.borrow_mut().push(verdict)
        });
        let preview = ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into());
        let request_id = new_request_id();
        let id = review
            .update(cx, |review, cx| {
                review.open_permission_preview(session(), request_id, preview, respond, cx)
            })
            .unwrap();
        review.update(cx, |review, cx| {
            review
                .set_decision(id, 0, HunkDecision::Rejected, cx)
                .unwrap();
            review
                .set_decision(id, 1, HunkDecision::Accepted, cx)
                .unwrap();
            review.submit(id, cx).unwrap();
        });
        cx.run_until_parked();
        assert_eq!(*verdicts.borrow(), [PermissionVerdict::Allow]);

        let write = agent_write("/p/a.txt", cx);
        let outcome = gate_write(&review, write, cx).await.unwrap();
        assert_eq!(
            outcome.content.as_deref(),
            Some("one\ntwo\nthree\nfour\nFIVE\n")
        );
        assert!(review.read_with(cx, |review, _| review.reviews().is_empty()));

        // An undecided preview disappears once the permission is answered in the thread view.
        let respond: PermissionResponder = Box::new(|_, _| {});
        let preview = ProposedEdit::new(PathBuf::from("/p/b.txt"), OLD.into(), NEW.into());
        let request_id = new_request_id();
        review.update(cx, |review, cx| {
            review.open_permission_preview(session(), request_id, preview, respond, cx);
            review.permission_answered(request_id, cx);
            assert!(review.reviews().is_empty());
        });
    }
}
