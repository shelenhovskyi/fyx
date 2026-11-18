//! Host side of `agent:observe` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §11, plan 4.8):
//! a read-only feed of the agent reviews. Nothing in the world takes a review id, so an
//! extension can watch reviews but never accept, reject or comment on one.
//!
//! `start` checks the manifest and the broker and returns; after the call, outside any budget,
//! the broker is asked (`ask` prompts) and the host's [`AgentReviews`] is subscribed. The
//! extension gets a `snapshot` first, then every change, with its own opaque ids and large
//! reviews cut ([`ui_ext::observe`]). While the guest does not keep up, a newer `updated` of a
//! review replaces the one still queued; past [`MAX_QUEUED_AGENT_EVENTS`] the queue is dropped
//! and a fresh snapshot is sent instead. `stop`, a revocation, a trap and a reload end it.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};

use futures::{
    StreamExt as _,
    future::{AbortHandle, Abortable, BoxFuture},
    stream::BoxStream,
};
pub use ui_ext::observe::{
    AgentEvent, CAPABILITY_OBSERVE, HunkDecision, HunkInfo, MAX_HUNK_TEXT_BYTES,
    MAX_SNAPSHOT_REVIEWS, OpaqueIds, Resolution, ReviewInfo, ReviewSource, RuleKind, TurnFile,
    cut_review, cut_snapshot,
};
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::{
    network::{IoEvent, Running},
    permissions::{CapabilityCheck, consent},
};

/// Agent events held for a guest that does not keep up, besides the coalesced `updated` ones.
pub const MAX_QUEUED_AGENT_EVENTS: usize = 1024;

/// The agent review model as extensions may see it, implemented by the host over
/// `agent_review`. Ids are the host's own; [`OpaqueIds`] maps them per extension.
pub trait AgentReviews: Send + Sync + 'static {
    /// The reviews open now, oldest first, and every change after them, in order, with no
    /// change lost between the two. The stream may begin with `review-mode`; a `snapshot` in it
    /// replaces everything before. Dropping the stream unsubscribes.
    ///
    /// Called from tokio, never on the main thread: an implementation over gpui state sends the
    /// request to the main thread and resolves when the snapshot is taken there.
    fn subscribe(&self) -> BoxFuture<'static, (Vec<ReviewInfo>, BoxStream<'static, AgentEvent>)>;
}

/// What the running call asked for; the last of `start` and `stop` wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObserveOp {
    Start,
    Stop,
}

/// The `agent-observe` state of one instance, in its store.
pub(crate) struct ObserveState {
    /// Whether the host gave an [`AgentReviews`]; without one, `start` is `unavailable`.
    available: bool,
    op: Option<ObserveOp>,
}

impl ObserveState {
    pub(crate) fn new(available: bool) -> Self {
        Self {
            available,
            op: None,
        }
    }

    /// `agent-observe.start`, after the caller checked the manifest and the broker. Starting
    /// while observing changes nothing.
    pub(crate) fn start(&mut self) -> Result<(), HostError> {
        if !self.available {
            return Err(HostError {
                code: HostErrorCode::Unavailable,
                message: "agent reviews are not available here".into(),
            });
        }
        self.op = Some(ObserveOp::Start);
        Ok(())
    }

    pub(crate) fn stop(&mut self) {
        self.op = Some(ObserveOp::Stop);
    }

    /// After a call: what to do if it succeeded; a failed call drops it.
    pub(crate) fn finish_call(&mut self, succeeded: bool) -> Option<ObserveOp> {
        let op = self.op.take();
        if succeeded { op } else { None }
    }
}

struct Observing {
    abort: AbortHandle,
    /// False once the feed ended by itself (the host's stream closed, consent refused).
    live: Arc<AtomicBool>,
}

/// The feed of one instance. Dropping it stops the feed.
pub(crate) struct ObserveHandle {
    extension_id: Arc<str>,
    broker: Option<Arc<dyn CapabilityCheck>>,
    reviews: Option<Arc<dyn AgentReviews>>,
    running: Arc<Running>,
    observing: Mutex<Option<Observing>>,
}

impl ObserveHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        broker: Option<Arc<dyn CapabilityCheck>>,
        reviews: Option<Arc<dyn AgentReviews>>,
        running: Arc<Running>,
    ) -> Self {
        Self {
            extension_id,
            broker,
            reviews,
            running,
            observing: Mutex::new(None),
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.reviews.is_some()
    }

    /// Applies what the call asked for, on the current tokio runtime.
    pub(crate) fn apply(&self, op: Option<ObserveOp>) {
        match op {
            Some(ObserveOp::Start) => self.start(),
            Some(ObserveOp::Stop) => self.stop(),
            None => {}
        }
    }

    fn start(&self) {
        let Some(reviews) = self.reviews.clone() else {
            return;
        };
        let mut observing = self
            .observing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if observing
            .as_ref()
            .is_some_and(|observing| observing.live.load(Ordering::SeqCst))
        {
            return;
        }
        self.running
            .diagnostics
            .io_note(CAPABILITY_OBSERVE, "start".into(), "observing".into());
        let (abort, registration) = AbortHandle::new_pair();
        let live = Arc::new(AtomicBool::new(true));
        *observing = Some(Observing {
            abort,
            live: live.clone(),
        });
        let broker = self.broker.clone();
        let extension_id = self.extension_id.clone();
        let running = self.running.clone();
        tokio::spawn(async move {
            let feed = async move {
                if consent(broker.as_ref(), &extension_id, CAPABILITY_OBSERVE).await {
                    feed(reviews.as_ref(), &running).await;
                }
            };
            Abortable::new(feed, registration).await.ok();
            live.store(false, Ordering::SeqCst);
        });
    }

    /// `stop`, a revocation of `agent:observe`, a trap: the feed ends and nothing more of it is
    /// delivered.
    pub(crate) fn stop(&self) {
        if let Some(observing) = self
            .observing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            observing.abort.abort();
            self.running
                .diagnostics
                .io_note(CAPABILITY_OBSERVE, "stop".into(), "stopped".into());
        }
    }
}

impl Drop for ObserveHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The review an event is about, by the extension's id.
fn review_of(event: &AgentEvent) -> Option<u64> {
    match event {
        AgentEvent::Opened(info) | AgentEvent::Updated(info) | AgentEvent::Resolved(info) => {
            Some(info.review)
        }
        AgentEvent::Closed(review) => Some(*review),
        _ => None,
    }
}

/// Queues `event`; a newer `updated` replaces a queued one of the same review when nothing else
/// about that review came between them. False when the queue is full.
pub(crate) fn enqueue(queue: &mut VecDeque<AgentEvent>, event: AgentEvent) -> bool {
    if let Some(review) = event.updated_review()
        && let Some(queued) = queue
            .iter_mut()
            .rev()
            .find(|queued| review_of(queued) == Some(review))
        && queued.updated_review() == Some(review)
    {
        *queued = event;
        return true;
    }
    if queue.len() >= MAX_QUEUED_AGENT_EVENTS {
        return false;
    }
    queue.push_back(event);
    true
}

/// Subscribes and forwards until the host's stream ends or the task is aborted.
async fn feed(reviews: &dyn AgentReviews, running: &Running) {
    let Some(mut sender) = running.sender() else {
        return;
    };
    let mut ids = OpaqueIds::default();
    let mut queue = VecDeque::new();
    let (snapshot, mut stream) = reviews.subscribe().await;
    queue.push_back(ids.event(AgentEvent::Snapshot(snapshot)));
    let mut ended = false;
    enum Step {
        Ready(bool),
        Event(Option<AgentEvent>),
    }
    loop {
        if ended && queue.is_empty() {
            return;
        }
        let step = futures::future::poll_fn(|cx| {
            if !queue.is_empty()
                && let Poll::Ready(ready) = sender.poll_ready(cx)
            {
                return Poll::Ready(Step::Ready(ready.is_ok()));
            }
            if !ended && let Poll::Ready(event) = stream.poll_next_unpin(cx) {
                return Poll::Ready(Step::Event(event));
            }
            Poll::Pending
        })
        .await;
        match step {
            Step::Ready(false) => return,
            Step::Ready(true) => {
                if let Some(event) = queue.pop_front()
                    && sender.start_send(IoEvent::Agent(event)).is_err()
                {
                    return;
                }
            }
            Step::Event(None) => ended = true,
            Step::Event(Some(event)) => {
                if matches!(event, AgentEvent::Snapshot(_)) {
                    queue.clear();
                }
                if !enqueue(&mut queue, ids.event(event)) {
                    // Too far behind: start over from a fresh snapshot.
                    drop(stream);
                    let (snapshot, fresh) = reviews.subscribe().await;
                    stream = fresh;
                    queue.clear();
                    queue.push_back(ids.event(AgentEvent::Snapshot(snapshot)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(review: u64, turn: u64) -> ReviewInfo {
        ReviewInfo {
            review,
            session: 1,
            turn,
            file: None,
            source: ReviewSource::Write,
            rule: None,
            resolution: Resolution::Pending(1),
            submitted: false,
            hunks: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn updates_coalesce_only_when_nothing_came_between() {
        let mut queue = VecDeque::new();
        assert!(enqueue(&mut queue, AgentEvent::Opened(info(1, 1))));
        assert!(enqueue(&mut queue, AgentEvent::Updated(info(1, 1))));
        assert!(enqueue(&mut queue, AgentEvent::Updated(info(2, 1))));
        assert!(enqueue(&mut queue, AgentEvent::Updated(info(1, 2))));
        assert_eq!(
            queue,
            [
                AgentEvent::Opened(info(1, 1)),
                AgentEvent::Updated(info(1, 2)),
                AgentEvent::Updated(info(2, 1)),
            ]
        );
        assert!(enqueue(&mut queue, AgentEvent::Resolved(info(1, 2))));
        assert!(enqueue(&mut queue, AgentEvent::Updated(info(1, 3))));
        assert_eq!(
            queue.len(),
            5,
            "an update after resolved is not folded into it"
        );
    }

    #[test]
    fn a_full_queue_refuses() {
        let mut queue = VecDeque::new();
        for review in 0..MAX_QUEUED_AGENT_EVENTS as u64 {
            assert!(enqueue(&mut queue, AgentEvent::Closed(review)));
        }
        assert!(!enqueue(&mut queue, AgentEvent::Closed(u64::MAX)));
        // An update of a queued review still folds in.
        let mut queue: VecDeque<_> = (0..MAX_QUEUED_AGENT_EVENTS as u64)
            .map(|review| AgentEvent::Updated(info(review, 1)))
            .collect();
        assert!(enqueue(&mut queue, AgentEvent::Updated(info(7, 2))));
    }
}
