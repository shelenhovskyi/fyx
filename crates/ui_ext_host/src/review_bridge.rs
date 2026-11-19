//! The review side of the 0.4 `review` import (W2c's [`ReviewHost`], design wit-0.4 §B.10, plan
//! 5.13): an extension's proposed edit opens a review in `agent_review`, and the user's decision
//! goes back to the extension. Accepting stays the user's.
//!
//! A proposal goes through `AgentReview::review_write` as a write of the session
//! `fyx-ext:<extension>:<session>`, so the review rules, the turn summary and "changed without
//! review" apply as to an agent's write. The target is checked as a `workspace:edit` target is
//! (trust, links, exclusions, `private_files`, `fyx_guard`, size); a diff is applied strictly to
//! the file's current text ([`crate::unified_diff`]). Review mode off refuses the proposal:
//! `review_write` would let it through unseen.
//!
//! The outcome is the content the review gives back: none is `rejected`, the proposal itself is
//! `accepted`, anything else `partial`. With `apply`, the accepted text replaces the buffer's as
//! one undo step, never saved; without it nothing is touched. Withdrawing, a revocation and the
//! instance's end drop the review's task, which closes the review without applying anything.
//!
//! Each load has its own host object; ids are unique in the process, and `close` names the
//! reviews to close, so an old instance's close after a reload never touches the new one's.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use acp_thread::fyx_review::AgentWrite;
use agent_review::AgentReview;
use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{Either, select},
};
use gpui::{App, AppContext as _, AsyncApp, Entity};
use language::Buffer;
use ui_ext::{
    process::FileRef,
    review::{Decision, Proposal, Proposed, ReviewHost, ReviewOutcome, ReviewSink},
};

use crate::main_thread::MainThread;

/// How long an extension thread waits for a review to open (the file may have to load).
const PROPOSE_TIMEOUT: Duration = Duration::from_secs(20);

/// An open review: the proposed text (what `withdraw` returns), and the signal that closes it.
struct Open {
    proposed: String,
    close: oneshot::Sender<()>,
}

type OpenReviews = Arc<Mutex<HashMap<u64, Open>>>;

fn lock(open: &OpenReviews) -> std::sync::MutexGuard<'_, HashMap<u64, Open>> {
    open.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The agent review session of an extension's tool session.
pub fn session_key(extension_id: &str, session: &str) -> String {
    format!("fyx-ext:{extension_id}:{session}")
}

/// The [`ReviewHost`] of one load. `key` is what [`crate::worktrees::OpenWorktrees::for_extension`]
/// was given.
pub fn for_extension(key: &str, cx: &mut App) -> Arc<dyn ReviewHost> {
    Arc::new(ExtensionReviews {
        key: key.into(),
        main: MainThread::global(cx),
        open: OpenReviews::default(),
    })
}

struct ExtensionReviews {
    key: Arc<str>,
    main: MainThread,
    open: OpenReviews,
}

impl ReviewHost for ExtensionReviews {
    fn propose(
        &self,
        extension_id: &str,
        id: u64,
        proposal: Proposal,
        sink: ReviewSink,
    ) -> Result<(), String> {
        let key = self.key.clone();
        let extension_id = extension_id.to_string();
        let open = self.open.clone();
        self.main
            .call(PROPOSE_TIMEOUT, move |cx, reply| {
                cx.spawn(async move |cx| {
                    let opened =
                        open_review(&key, &extension_id, id, proposal, sink, &open, cx).await;
                    let ok = opened.is_ok();
                    if !reply.send(opened) && ok {
                        // The extension stopped waiting and was told `unavailable`.
                        if let Some(review) = lock(&open).remove(&id) {
                            review.close.send(()).ok();
                        }
                    }
                })
                .detach();
            })
            .unwrap_or_else(|| Err("the review did not open in time".into()))
    }

    fn withdraw(&self, _extension_id: &str, id: u64) -> Result<String, String> {
        let review = lock(&self.open)
            .remove(&id)
            .ok_or("no open review with that id")?;
        review.close.send(()).ok();
        Ok(review.proposed)
    }

    fn begin_turn(&self, extension_id: &str, session: &str) {
        let session = session_key(extension_id, session);
        self.main.post(move |cx| {
            if let Some(review) = AgentReview::global(cx) {
                review.update(cx, |review, _| {
                    review.begin_turn(&session.into());
                });
            }
        });
    }

    fn end_turn(&self, extension_id: &str, session: &str, changed: Vec<FileRef>) {
        let session = session_key(extension_id, session);
        let key = self.key.clone();
        self.main.post(move |cx| {
            let Some(review) = AgentReview::global(cx) else {
                return;
            };
            let paths = changed
                .iter()
                .filter_map(|file| crate::context_host::locate(&key, file, cx).ok())
                .map(|(_, abs_path)| abs_path)
                .collect::<Vec<_>>();
            review.update(cx, |review, cx| {
                let session = session.into();
                review.note_changed_paths(&session, paths, cx);
                review.end_turn(&session, cx);
            });
        });
    }

    fn close(&self, _extension_id: &str, ids: &[u64]) {
        let closed = {
            let mut open = lock(&self.open);
            ids.iter()
                .filter_map(|id| open.remove(id))
                .collect::<Vec<_>>()
        };
        for review in closed {
            review.close.send(()).ok();
        }
    }
}

/// Resolves, loads and opens the review; its outcome goes to `sink` once, unless it was
/// withdrawn or closed first.
async fn open_review(
    key: &str,
    extension_id: &str,
    id: u64,
    proposal: Proposal,
    sink: ReviewSink,
    open: &OpenReviews,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let review = cx
        .update(|cx| AgentReview::global(cx))
        .ok_or("agent review is not available")?;
    if !review.read_with(cx, |review, _| review.is_enabled()) {
        return Err(
            "review mode is off: turn on agent review to see this extension's proposals".into(),
        );
    }
    let create = matches!(proposal.change, Proposed::NewContent(_));
    let file = crate::workspace_edit::resolve_file(key, proposal.path.clone(), create, cx)
        .await
        .map_err(|error| crate::workspace_edit::describe_error(&error))?;
    let buffer = file
        .project
        .update(cx, |project, cx| {
            project.open_buffer(file.project_path.clone(), cx)
        })
        .await
        .map_err(|error| format!("{} could not be opened: {error:#}", proposal.path.path))?;
    let old_text = buffer.read_with(cx, |buffer, _| buffer.text());
    let new_text = match proposal.change {
        Proposed::NewContent(text) => text,
        Proposed::UnifiedDiff(diff) => crate::unified_diff::apply(&old_text, &diff)
            .map_err(|why| format!("the diff does not apply: {why}"))?,
    };
    let write = AgentWrite {
        session_id: session_key(extension_id, &proposal.session).into(),
        abs_path: file.abs_path,
        buffer: buffer.clone(),
        old_text: old_text.clone(),
        new_text: new_text.clone(),
    };
    let decided = review.update(cx, |review, cx| review.review_write(write, cx));
    let (close, closed) = oneshot::channel();
    lock(open).insert(
        id,
        Open {
            proposed: new_text.clone(),
            close,
        },
    );
    let open = open.clone();
    let apply = proposal.apply;
    cx.spawn(async move |cx| {
        let result = match select(decided, closed.map(|_| ())).await {
            Either::Left((result, _)) => result,
            // Withdrawn or closed: dropping the task closes the review, nothing applied.
            Either::Right(_) => return,
        };
        if lock(&open).remove(&id).is_none() {
            return;
        }
        let outcome = match result {
            Err(error) => Err(format!("{error:#}")),
            Ok(written) => {
                let final_content = written.content.clone().unwrap_or_else(|| old_text.clone());
                let decision = match &written.content {
                    None => Decision::Rejected,
                    Some(content) if *content == new_text => Decision::Accepted,
                    Some(_) => Decision::Partial,
                };
                let applied = match (&written.content, apply) {
                    (Some(content), true) if *content != old_text => {
                        cx.update(|cx| apply_to_buffer(&buffer, &old_text, content, cx))
                    }
                    _ => Ok(()),
                };
                applied.map(|()| ReviewOutcome {
                    decision,
                    final_content,
                })
            }
        };
        cx.background_spawn(async move { sink(id, outcome) })
            .detach();
    })
    .detach();
    Ok(())
}

/// The byte ranges `old` and `new` differ in, trimmed of their common start and end at character
/// boundaries.
pub(crate) fn changed_range(
    old: &str,
    new: &str,
) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = old[prefix..]
        .bytes()
        .rev()
        .zip(new[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}

/// The accepted text into the buffer, as one undo step of its own; not saved. Refused when the
/// buffer changed during the review.
fn apply_to_buffer(
    buffer: &Entity<Buffer>,
    old_text: &str,
    new_text: &str,
    cx: &mut App,
) -> Result<(), String> {
    if buffer.read(cx).text() != old_text {
        return Err("the file changed during the review; nothing was applied".into());
    }
    let (old_range, new_range) = changed_range(old_text, new_text);
    buffer.update(cx, |buffer, cx| {
        buffer.finalize_last_transaction();
        buffer.start_transaction();
        buffer.edit([(old_range, &new_text[new_range])], None, cx);
        buffer.end_transaction(cx);
        buffer.finalize_last_transaction();
    });
    crate::workspace_edit::keep_buffer_until_clean(buffer, cx);
    Ok(())
}

#[cfg(test)]
#[path = "review_bridge_tests.rs"]
mod tests;
