//! Review comments sent to the agent as a turn of their own (plan 7.2).
//!
//! A comment on a rejected hunk explains the rejection, so it travels with the decision (the tool
//! result). A comment on an accepted hunk asks for follow-up work: it stays open until the user
//! sends the session's open comments to the agent as one prompt, with file and line references, or
//! until the user's next prompt in the session, in front of which it goes. Nothing is lost when the
//! user never presses "Send".

use std::{
    collections::HashMap,
    fmt::Write as _,
    ops::Range,
    path::{Path, PathBuf},
};

use acp_thread::ThreadStatus;
use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Result, anyhow};
use gpui::{App, Context, SharedString, WeakEntity};
use project::Project;

use crate::{
    AgentReview, AgentReviewEvent, HunkDecision, ProposedEdit, ReviewSource,
    review_pane::{display_path, escape_for_display},
};

/// Longest excerpt of the commented change quoted to the agent, in characters.
const EXCERPT_CHARS: usize = 80;

/// One comment on one hunk of an agent's change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewComment {
    pub path: PathBuf,
    /// Rows of the file after the change that the hunk covers (0-based, end exclusive), or `None`
    /// when the review showed a fragment of the file (a permission preview), whose rows are not the
    /// file's.
    pub rows: Option<Range<u32>>,
    pub decision: HunkDecision,
    /// The first changed line, so the agent finds the place even without rows.
    pub excerpt: String,
    pub text: String,
}

impl ReviewComment {
    fn location(&self, path: &str) -> String {
        match &self.rows {
            Some(rows) if rows.is_empty() => {
                format!("{path}:{} (lines removed)", rows.start.max(1))
            }
            Some(rows) if rows.len() == 1 => format!("{path}:{}", rows.start + 1),
            Some(rows) => format!("{path}:{}-{}", rows.start + 1, rows.end),
            None => path.to_string(),
        }
    }
}

/// Who delivers a session's comments to the agent.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CommentRecipient {
    /// The session's thread in the agent panel: the comments are its next prompt.
    Agent,
    /// The extension whose proposals were reviewed (a terminal agent); it forwards the batch to its
    /// tool (`fyx:ui` 0.4 `review-comment`, see `docs/fyx/wit-0.4/design.md` §B.10).
    Extension(SharedString),
}

/// The open comments of one session for one recipient, sent together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentBatch {
    pub session_id: acp_v2::SessionId,
    pub recipient: CommentRecipient,
    pub comments: Vec<ReviewComment>,
    /// The comments as the agent reads them, with paths relative to the project.
    pub prompt: String,
}

/// What became of a session's comments, for the review pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommentsStatus {
    /// Waiting for the user to send them.
    pub open: usize,
    /// Sent while the agent was busy: they go out when its turn stops.
    pub queued: usize,
    /// Delivered to the agent's thread or handed to the extension.
    pub sent: usize,
}

#[derive(Default)]
pub(crate) struct Comments {
    /// Comments of decided reviews, not sent yet.
    open: HashMap<acp_v2::SessionId, Vec<(CommentRecipient, ReviewComment)>>,
    /// The prompt to send when the session's turn stops.
    queued: HashMap<acp_v2::SessionId, CommentBatch>,
    /// Batches for extensions, until their bridge takes them.
    for_extensions: Vec<CommentBatch>,
    sent: HashMap<acp_v2::SessionId, usize>,
    /// The project of each session's thread, for paths relative to it.
    projects: HashMap<acp_v2::SessionId, WeakEntity<Project>>,
}

impl Comments {
    pub(crate) fn thread_opened(
        &mut self,
        session_id: acp_v2::SessionId,
        project: WeakEntity<Project>,
    ) {
        self.projects.insert(session_id, project);
    }

    pub(crate) fn session_closed(&mut self, session_id: &acp_v2::SessionId) {
        self.open.remove(session_id);
        self.queued.remove(session_id);
        self.sent.remove(session_id);
        self.projects.remove(session_id);
    }
}

fn recipient_for(source: &ReviewSource) -> CommentRecipient {
    match source {
        ReviewSource::Extension { name, .. } => CommentRecipient::Extension(name.clone()),
        ReviewSource::Permission(_) | ReviewSource::Write => CommentRecipient::Agent,
    }
}

/// The comments of `edit`'s hunks that `keep` selects, not sent yet.
fn hunk_comments(
    edit: &ProposedEdit,
    source: &ReviewSource,
    keep: impl Fn(HunkDecision) -> bool,
) -> Vec<ReviewComment> {
    let fragment = matches!(source, ReviewSource::Permission(_));
    edit.hunks()
        .iter()
        .filter(|hunk| !hunk.comment_sent && keep(hunk.decision))
        .filter_map(|hunk| {
            let text = hunk.comment.clone()?;
            let changed = if hunk.new_bytes.is_empty() {
                &edit.old_text()[hunk.old_bytes.clone()]
            } else {
                &edit.new_text()[hunk.new_bytes.clone()]
            };
            Some(ReviewComment {
                path: edit.abs_path().to_path_buf(),
                rows: (!fragment).then(|| hunk.new_rows.clone()),
                decision: hunk.decision,
                excerpt: excerpt(changed),
                text,
            })
        })
        .collect()
}

fn excerpt(changed: &str) -> String {
    let line = changed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut excerpt = line.chars().take(EXCERPT_CHARS).collect::<String>();
    if line.chars().count() > EXCERPT_CHARS {
        excerpt.push('…');
    }
    escape_for_display(&excerpt)
}

/// The comments as one prompt for the agent.
pub fn comments_prompt(comments: &[ReviewComment], display: &dyn Fn(&Path) -> String) -> String {
    let mut prompt = String::from(
        "[Fyx review: comments from the user's review of your changes]\nThe user reviewed your \
         changes and left these comments. Address each one. Accepted changes are already in the \
         files; read them before editing.",
    );
    for (index, comment) in comments.iter().enumerate() {
        let state = match comment.decision {
            HunkDecision::Accepted => "accepted change",
            HunkDecision::Rejected => "rejected change, not written",
            HunkDecision::Pending => "change still under review",
        };
        write!(
            prompt,
            "\n\n{}. {} ({state}: `{}`)\n{}",
            index + 1,
            comment.location(&display(&comment.path)),
            comment.excerpt,
            comment.text
        )
        .ok();
    }
    prompt.push_str("\n[End of the review comments]");
    prompt
}

impl AgentReview {
    /// Keeps the comments on accepted hunks of a decided review as the session's open comments.
    /// Comments on rejected hunks went with the decision.
    pub(crate) fn keep_comments(
        &mut self,
        session_id: &acp_v2::SessionId,
        source: &ReviewSource,
        edit: &ProposedEdit,
        cx: &mut Context<Self>,
    ) {
        let comments = hunk_comments(edit, source, |decision| decision == HunkDecision::Accepted);
        if comments.is_empty() {
            return;
        }
        let recipient = recipient_for(source);
        let open = self.comments.open.entry(session_id.clone()).or_default();
        for comment in comments {
            // A preview's comments reach its write too; the write's rows are the file's.
            open.retain(|(_, kept)| !(kept.path == comment.path && kept.text == comment.text));
            open.push((recipient.clone(), comment));
        }
        cx.emit(AgentReviewEvent::CommentsChanged);
        cx.notify();
    }

    /// The session's comments not sent yet: of decided reviews, and of reviews still open.
    pub fn open_comments(&self, session_id: &acp_v2::SessionId) -> Vec<ReviewComment> {
        let mut comments = self
            .comments
            .open
            .get(session_id)
            .into_iter()
            .flatten()
            .map(|(_, comment)| comment.clone())
            .collect::<Vec<_>>();
        for review in self
            .reviews
            .iter()
            .filter(|review| &review.session_id == session_id)
        {
            comments.extend(hunk_comments(&review.edit, &review.source, |_| true));
        }
        comments
    }

    pub fn comments_status(&self, session_id: &acp_v2::SessionId) -> CommentsStatus {
        CommentsStatus {
            open: self.open_comments(session_id).len(),
            queued: self
                .comments
                .queued
                .get(session_id)
                .map_or(0, |batch| batch.comments.len()),
            sent: self.comments.sent.get(session_id).copied().unwrap_or(0),
        }
    }

    /// Sends the session's open comments to the agent as one prompt: now when its thread is idle,
    /// when its turn stops when it is busy. Comments on extensions' proposals go to the extension
    /// ([`Self::take_extension_comments`]). Returns how many comments went.
    pub fn send_comments(
        &mut self,
        session_id: &acp_v2::SessionId,
        cx: &mut Context<Self>,
    ) -> Result<usize> {
        let mut by_recipient: Vec<(CommentRecipient, Vec<ReviewComment>)> = Vec::new();
        let mut add = |recipient: CommentRecipient, comment: ReviewComment| match by_recipient
            .iter_mut()
            .find(|(kept, _)| *kept == recipient)
        {
            Some((_, comments)) => comments.push(comment),
            None => by_recipient.push((recipient, vec![comment])),
        };
        let decided = self
            .comments
            .open
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        for (recipient, comment) in decided {
            add(recipient, comment);
        }
        for review in self
            .reviews
            .iter()
            .filter(|review| &review.session_id == session_id)
        {
            for comment in hunk_comments(&review.edit, &review.source, |_| true) {
                add(recipient_for(&review.source), comment);
            }
        }
        if by_recipient.is_empty() {
            return Err(anyhow!("There are no comments to send"));
        }
        let thread = self.thread_for_session(session_id);
        if thread.is_none()
            && by_recipient
                .iter()
                .any(|(recipient, _)| *recipient == CommentRecipient::Agent)
        {
            return Err(anyhow!(
                "The agent's thread is closed; the comments stay until it opens again"
            ));
        }

        self.comments.open.remove(session_id);
        let mut updated = Vec::new();
        for review in self
            .reviews
            .iter_mut()
            .filter(|review| &review.session_id == session_id)
        {
            if review.edit.mark_comments_sent() > 0 {
                updated.push(review.id);
            }
        }
        for id in updated {
            cx.emit(AgentReviewEvent::Updated(id));
        }

        let display = self.display_for(session_id);
        let mut count = 0;
        for (recipient, comments) in by_recipient {
            count += comments.len();
            match recipient {
                CommentRecipient::Agent => {
                    let busy = thread
                        .as_ref()
                        .is_some_and(|thread| thread.read(cx).status() == ThreadStatus::Generating);
                    if busy {
                        let batch = self
                            .comments
                            .queued
                            .entry(session_id.clone())
                            .or_insert_with(|| CommentBatch {
                                session_id: session_id.clone(),
                                recipient: CommentRecipient::Agent,
                                comments: Vec::new(),
                                prompt: String::new(),
                            });
                        batch.comments.extend(comments);
                        batch.prompt = comments_prompt(&batch.comments, &|path| display(path, cx));
                    } else if let Some(thread) = thread.clone() {
                        let prompt = comments_prompt(&comments, &|path| display(path, cx));
                        *self.comments.sent.entry(session_id.clone()).or_default() +=
                            comments.len();
                        // Outside this update: sending takes the session's notes from this entity.
                        cx.spawn(async move |_, cx| {
                            let submission = thread.update(cx, |thread, cx| {
                                thread.send(
                                    vec![acp_v2::ContentBlock::Text(acp_v2::TextContent::new(
                                        prompt,
                                    ))],
                                    cx,
                                )
                            });
                            if let Err(error) = submission.await {
                                log::error!(
                                    "fyx: agent review: sending review comments failed: {error:#}"
                                );
                            }
                        })
                        .detach();
                    }
                }
                CommentRecipient::Extension(name) => {
                    let prompt = comments_prompt(&comments, &|path| display(path, cx));
                    *self.comments.sent.entry(session_id.clone()).or_default() += comments.len();
                    self.comments.for_extensions.push(CommentBatch {
                        session_id: session_id.clone(),
                        recipient: CommentRecipient::Extension(name),
                        comments,
                        prompt,
                    });
                }
            }
        }
        cx.emit(AgentReviewEvent::CommentsChanged);
        cx.notify();
        Ok(count)
    }

    /// Takes the comment batches for extension `name`, for its tool. The host bridge takes them on
    /// [`AgentReviewEvent::CommentsChanged`].
    pub fn take_extension_comments(&mut self, name: &str) -> Vec<CommentBatch> {
        let (taken, kept) = std::mem::take(&mut self.comments.for_extensions)
            .into_iter()
            .partition(|batch| {
                matches!(&batch.recipient, CommentRecipient::Extension(recipient) if recipient.as_ref() == name)
            });
        self.comments.for_extensions = kept;
        taken
    }

    /// The prompt queued for the session's stopped turn, counted as sent.
    pub(crate) fn take_queued_comments(
        &mut self,
        session_id: &acp_v2::SessionId,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let batch = self.comments.queued.remove(session_id)?;
        *self.comments.sent.entry(session_id.clone()).or_default() += batch.comments.len();
        cx.emit(AgentReviewEvent::CommentsChanged);
        Some(batch.prompt)
    }

    /// The turn failed: queued comments are open again, for the user to send later.
    pub(crate) fn requeue_comments(
        &mut self,
        session_id: &acp_v2::SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(batch) = self.comments.queued.remove(session_id) else {
            return;
        };
        let open = self.comments.open.entry(session_id.clone()).or_default();
        open.extend(
            batch
                .comments
                .into_iter()
                .map(|comment| (CommentRecipient::Agent, comment)),
        );
        cx.emit(AgentReviewEvent::CommentsChanged);
    }

    /// The session's open comments for the agent, taken for the front of the user's next prompt.
    pub(crate) fn take_open_agent_comments(
        &mut self,
        session_id: &acp_v2::SessionId,
        cx: &App,
    ) -> Option<String> {
        let open = self.comments.open.get_mut(session_id)?;
        let (agent, extension): (Vec<_>, Vec<_>) = std::mem::take(open)
            .into_iter()
            .partition(|(recipient, _)| *recipient == CommentRecipient::Agent);
        *open = extension;
        let mut comments = agent
            .into_iter()
            .map(|(_, comment)| comment)
            .collect::<Vec<_>>();
        if let Some(batch) = self.comments.queued.remove(session_id) {
            comments.splice(0..0, batch.comments);
        }
        if comments.is_empty() {
            return None;
        }
        *self.comments.sent.entry(session_id.clone()).or_default() += comments.len();
        let display = self.display_for(session_id);
        Some(comments_prompt(&comments, &|path| display(path, cx)))
    }

    fn display_for(
        &self,
        session_id: &acp_v2::SessionId,
    ) -> impl Fn(&Path, &App) -> String + use<> {
        let project = self.comments.projects.get(session_id).cloned();
        move |path, cx| match project.as_ref().and_then(WeakEntity::upgrade) {
            Some(project) => display_path(&project, path, cx),
            None => escape_for_display(&path.display().to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(path: &str, rows: Option<Range<u32>>, text: &str) -> ReviewComment {
        ReviewComment {
            path: PathBuf::from(path),
            rows,
            decision: HunkDecision::Accepted,
            excerpt: "let x = 5;".into(),
            text: text.into(),
        }
    }

    #[test]
    fn the_prompt_names_each_file_and_line() {
        let prompt = comments_prompt(
            &[
                comment("a.rs", Some(4..5), "rename x"),
                comment("a.rs", Some(9..12), "add a test"),
                comment("b.rs", None, "too clever"),
                comment("c.rs", Some(3..3), "why remove this?"),
            ],
            &|path| path.display().to_string(),
        );
        assert!(
            prompt.contains("1. a.rs:5 (accepted change: `let x = 5;`)\nrename x"),
            "{prompt}"
        );
        assert!(prompt.contains("2. a.rs:10-12 "), "{prompt}");
        assert!(prompt.contains("3. b.rs (accepted"), "{prompt}");
        assert!(prompt.contains("4. c.rs:3 (lines removed)"), "{prompt}");
    }

    #[test]
    fn excerpts_are_one_short_line() {
        assert_eq!(excerpt("\n   fn main() {\n}\n"), "fn main() {");
        let long = "x".repeat(200);
        assert_eq!(excerpt(&long).chars().count(), EXCERPT_CHARS + 1);
        assert_eq!(excerpt("a\u{202E}b"), "a\\u{202e}b");
    }
}
