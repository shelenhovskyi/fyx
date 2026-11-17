//! Host-owned review of agent-proposed edits.
//!
//! Agent writes reach [`AgentReview`] through the gate in `acp_thread::fyx_review`; permission requests
//! that carry a diff are observed on every [`AcpThread`]. Only the host resolves a review: extensions may
//! observe [`AgentReviewEvent`]s and read reviews, but there is no path for them to decide whether an
//! edit lands.

mod proposed_edit;
mod review_pane;
// dev-only: never compiled into release builds unless the `review-script` feature asks for it, because a
// script can approve agent edits the user never saw.
#[cfg(any(debug_assertions, feature = "review-script"))]
mod script_driver;

use std::{ops::Range, rc::Rc};

use acp_thread::{
    AcpThread, AcpThreadEvent, PermissionRequestId, SelectedPermissionOutcome, ToolCallContent,
    fyx_review::{AgentWrite, AgentWriteGate, AgentWriteOutcome, set_agent_write_gate},
};
use action_log::ActionLog;
use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Context as _, Result, anyhow};
use fs::Fs;
use futures::channel::oneshot;
use gpui::Action as _;
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, EventEmitter, Global, Task, WeakEntity,
};
use language::BufferEditSource;
use project::Project;
use settings::{RegisterSetting, Settings, SettingsStore};
use ui::{ContextMenu, IconPosition};
use workspace::{MultiWorkspace, Toast, notifications::NotificationId};

pub use proposed_edit::{Hunk, HunkDecision, ProposedEdit, Resolution};
pub use review_pane::{
    AcceptAllHunks, AcceptHunk, CommentHunk, NextHunk, OpenReviews, PreviousHunk, RejectAllHunks,
    RejectHunk, ReviewPane, SubmitReview, ToggleReviewMode,
};

/// Overrides the `agent_review.enabled` setting: `0` turns review off, any other value turns it on.
const REVIEW_MODE_VAR: &str = "FYX_AGENT_REVIEW";

/// `agent_review` in settings (schema in `settings_content::AgentReviewSettingsContent`, default in
/// `assets/settings/fyx.json`).
#[derive(Clone, Debug, RegisterSetting)]
pub struct AgentReviewSettings {
    pub enabled: bool,
}

impl Settings for AgentReviewSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            enabled: content
                .agent_review
                .as_ref()
                .and_then(|review| review.enabled)
                .unwrap_or(false),
        }
    }
}

fn review_mode_override() -> Option<bool> {
    std::env::var_os(REVIEW_MODE_VAR).map(|value| value != "0")
}

/// Whether agent edits are reviewed: the environment override, else the setting.
pub fn review_mode(cx: &App) -> bool {
    review_mode_override().unwrap_or_else(|| {
        cx.has_global::<SettingsStore>() && AgentReviewSettings::get_global(cx).enabled
    })
}

/// Writes `agent_review.enabled` to the user's settings; review mode follows the settings change.
pub fn set_review_mode_setting(enabled: bool, cx: &App) {
    if review_mode_override().is_some() {
        log::warn!("fyx: {REVIEW_MODE_VAR} is set and overrides the agent_review.enabled setting");
    }
    settings::update_settings_file(<dyn Fs>::global(cx), cx, move |settings, _| {
        settings.agent_review.get_or_insert_default().enabled = Some(enabled);
    });
}

/// The agent panel's options-menu toggle for review mode.
pub fn review_mode_menu_entry(menu: ContextMenu, cx: &App) -> ContextMenu {
    let enabled = review_mode(cx);
    menu.toggleable_entry_disabled_when(
        "Review Agent Edits",
        enabled,
        review_mode_override().is_some(),
        IconPosition::Start,
        Some(ToggleReviewMode.boxed_clone()),
        move |_, cx| set_review_mode_setting(!enabled, cx),
    )
}

pub fn init(cx: &mut App) {
    let review = cx.new(|cx| AgentReview {
        enabled: review_mode(cx),
        ..AgentReview::default()
    });
    cx.set_global(GlobalAgentReview(review.clone()));
    cx.observe_global::<SettingsStore>({
        let review = review.downgrade();
        move |cx| {
            let enabled = review_mode(cx);
            if let Some(review) = review.upgrade()
                && review.read(cx).is_enabled() != enabled
            {
                review.update(cx, |review, cx| review.set_enabled(enabled, cx));
            }
        }
    })
    .detach();
    set_agent_write_gate(Rc::new(WriteGate(review.downgrade())), cx);
    cx.observe_new(|_: &mut AcpThread, _, cx: &mut Context<AcpThread>| {
        cx.subscribe_self(on_thread_event).detach();
        cx.on_release(|thread, cx| {
            if let Some(review) = AgentReview::global(cx) {
                let session_id = thread.session_id().clone();
                review.update(cx, |review, cx| review.close_session(&session_id, cx));
            }
        })
        .detach();
    })
    .detach();
    review_pane::init(cx);
    #[cfg(any(debug_assertions, feature = "review-script"))]
    script_driver::init(cx); // dev-only, off unless FYX_AGENT_REVIEW_SCRIPT is set
}

/// Whether this build contains the dev-only review script driver.
pub const SCRIPT_DRIVER_COMPILED: bool = cfg!(any(debug_assertions, feature = "review-script"));

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
    /// Some hunks were rejected. The host writes the accepted hunks itself and denies the tool call,
    /// so an agent that writes to disk on its own cannot land a hunk the user rejected.
    ApplyAcceptedAndDeny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentReviewEvent {
    Opened(ReviewId),
    Updated(ReviewId),
    Resolved(ReviewId),
    Closed(ReviewId),
}

type PermissionResponder = Box<dyn FnOnce(PermissionVerdict, ProposedEdit, &mut App)>;

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
                let edit = review.edit.clone();
                // Answering the permission makes the thread emit an event this entity handles, so it
                // must not run while this entity is being updated.
                cx.defer(move |cx| respond(verdict, edit, cx));
                cx.emit(AgentReviewEvent::Resolved(id));
                // An allowed preview stays until the agent's write adopts its decisions; after a
                // denial no write follows.
                if verdict != PermissionVerdict::Allow {
                    self.reviews.retain(|review| review.id != id);
                    cx.emit(AgentReviewEvent::Closed(id));
                }
            }
        }
        cx.notify();
        Ok(())
    }

    /// Closes every review of a session, e.g. when its thread is released. Held writes fail.
    pub fn close_session(&mut self, session_id: &acp_v2::SessionId, cx: &mut Context<Self>) {
        let ids = self
            .reviews
            .iter()
            .filter(|review| &review.session_id == session_id)
            .map(|review| review.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.close(id, cx);
        }
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
        Resolution::Unchanged | Resolution::AcceptedAll => PermissionVerdict::Allow,
        Resolution::RejectedAll => PermissionVerdict::Deny,
        Resolution::Partial { .. } | Resolution::Pending { .. } => {
            PermissionVerdict::ApplyAcceptedAndDeny
        }
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
    let respond: PermissionResponder = Box::new(move |verdict, edit, cx| {
        let (option_id, option_kind) = match verdict {
            PermissionVerdict::Allow => (allow_option, acp_v2::PermissionOptionKind::AllowOnce),
            PermissionVerdict::Deny | PermissionVerdict::ApplyAcceptedAndDeny => {
                (deny_option, acp_v2::PermissionOptionKind::RejectOnce)
            }
        };
        let outcome = SelectedPermissionOutcome::new(option_id, option_kind);
        cx.spawn(async move |cx| {
            // The accepted hunks land before the agent hears the denial and reads the file again.
            if verdict == PermissionVerdict::ApplyAcceptedAndDeny {
                let thread_parts = thread.read_with(cx, |thread, _| {
                    (thread.project().clone(), thread.action_log().clone())
                });
                match thread_parts {
                    Ok((project, action_log)) => {
                        if let Err(error) =
                            write_accepted_hunks(project.clone(), action_log, &edit, cx).await
                        {
                            let message = format!(
                                "Agent review: the accepted hunks of {} could not be written:                                  {error:#}. The agent's edit was rejected.",
                                edit.abs_path().display()
                            );
                            log::error!("{message}");
                            cx.update(|cx| report_host_write_failure(&project, message, cx));
                        }
                    }
                    Err(error) => {
                        log::debug!("thread closed before its review was applied: {error}")
                    }
                }
            }
            if let Err(error) = thread.update(cx, |thread, cx| {
                thread.authorize_permission_request(request_id, outcome, cx)
            }) {
                log::debug!("thread closed before its edit review was submitted: {error}");
            }
        })
        .detach();
    });
    Some((edit, respond))
}

struct HostWriteFailed;

/// Shows a failed host write in the workspaces of `project`. ACP has no way to attach a message to a
/// permission answer, so the user is the one who has to learn that approved hunks did not land.
fn report_host_write_failure(project: &Entity<Project>, message: String, cx: &mut App) {
    for window in cx.windows() {
        let Some(multi_workspace) = window.downcast::<MultiWorkspace>() else {
            continue;
        };
        let result = multi_workspace.update(cx, |multi_workspace, _, cx| {
            let workspace = multi_workspace.workspace().clone();
            if workspace.read(cx).project() == project {
                workspace.update(cx, |workspace, cx| {
                    workspace.show_toast(
                        Toast::new(NotificationId::unique::<HostWriteFailed>(), message.clone()),
                        cx,
                    )
                });
            }
        });
        if let Err(error) = result {
            log::debug!("could not show the review failure: {error}");
        }
    }
}

/// Writes a preview's accepted hunks into the file, through the action log like an agent write, so
/// they show up in the agent diff. The preview's old text is a fragment that must occur exactly once.
async fn write_accepted_hunks(
    project: Entity<Project>,
    action_log: Entity<ActionLog>,
    edit: &ProposedEdit,
    cx: &mut AsyncApp,
) -> Result<()> {
    let replacement = edit.apply_accepted()?.replace("\r\n", "\n");
    let fragment = edit.old_text().replace("\r\n", "\n");
    let open = project.update(cx, |project, cx| {
        let path = project
            .project_path_for_absolute_path(edit.abs_path(), cx)
            .context("the file is outside the project")?;
        anyhow::Ok(project.open_buffer(path, cx))
    })?;
    let buffer = open.await?;
    cx.update(|cx| {
        let range = locate_fragment(&buffer.read(cx).text(), &fragment)?;
        action_log.update(cx, |action_log, cx| {
            action_log.buffer_read(buffer.clone(), cx)
        });
        buffer.update(cx, |buffer, cx| {
            buffer.start_transaction();
            buffer.edit([(range, replacement)], None, cx);
            buffer.end_transaction_with_source(BufferEditSource::Agent, cx);
        });
        action_log.update(cx, |action_log, cx| {
            action_log.buffer_edited(buffer.clone(), cx)
        });
        anyhow::Ok(())
    })?;
    project
        .update(cx, |project, cx| project.save_buffer(buffer, cx))
        .await
}

fn locate_fragment(text: &str, fragment: &str) -> Result<Range<usize>> {
    anyhow::ensure!(
        !fragment.is_empty(),
        "the proposed edit has no old text to replace"
    );
    let mut matches = text.match_indices(fragment).map(|(start, _)| start);
    let start = matches
        .next()
        .context("the file no longer contains the text the agent proposed to change")?;
    anyhow::ensure!(
        matches.next().is_none(),
        "the text the agent proposed to change occurs more than once"
    );
    Ok(start..start + fragment.len())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::Path, path::PathBuf};

    use super::*;
    use gpui::TestAppContext;
    use language::Buffer;
    use gpui::UpdateGlobal as _;

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
            move |verdict, _, _| verdicts.borrow_mut().push(verdict)
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
    async fn partly_accepted_preview_is_denied_and_applied_by_the_host(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let answers = Rc::new(RefCell::new(Vec::new()));
        let respond: PermissionResponder = Box::new({
            let answers = answers.clone();
            move |verdict, edit, _| answers.borrow_mut().push((verdict, edit))
        });
        let preview = ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into());
        let id = review
            .update(cx, |review, cx| {
                review.open_permission_preview(session(), new_request_id(), preview, respond, cx)
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
            assert!(review.reviews().is_empty(), "no write follows a denial");
        });
        cx.run_until_parked();
        let answers = answers.borrow();
        let [(verdict, edit)] = answers.as_slice() else {
            panic!("expected one answer, got {}", answers.len());
        };
        assert_eq!(*verdict, PermissionVerdict::ApplyAcceptedAndDeny);
        assert_eq!(
            edit.apply_accepted().unwrap(),
            "one\ntwo\nthree\nfour\nFIVE\n"
        );
    }

    #[gpui::test]
    async fn previews_close_when_answered_elsewhere_and_sessions_close(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let preview = ProposedEdit::new(PathBuf::from("/p/b.txt"), OLD.into(), NEW.into());
        let request_id = new_request_id();
        review.update(cx, |review, cx| {
            review.open_permission_preview(
                session(),
                request_id,
                preview,
                Box::new(|_, _, _| {}),
                cx,
            );
            review.permission_answered(request_id, cx);
            assert!(review.reviews().is_empty());
        });

        let write = agent_write("/p/a.txt", cx);
        let held = gate_write(&review, write, cx);
        let mut other = agent_write("/p/a.txt", cx);
        other.session_id = acp_v2::SessionId::new("other");
        let other_held = gate_write(&review, other, cx);
        review.update(cx, |review, cx| {
            review.close_session(&session(), cx);
            assert_eq!(review.reviews().len(), 1);
            assert_eq!(
                review.reviews()[0].session_id(),
                &acp_v2::SessionId::new("other")
            );
        });
        assert!(held.await.is_err());
        drop(other_held);
    }

    #[test]
    fn locates_a_fragment_only_when_it_is_unique() {
        assert_eq!(locate_fragment("a\nb\nc\n", "b\n").unwrap(), 2..4);
        assert!(locate_fragment("a\nb\nb\n", "b\n").is_err());
        assert!(locate_fragment("a\n", "z").is_err());
        assert!(locate_fragment("a\n", "").is_err());
    }

    #[gpui::test]
    async fn review_mode_follows_the_setting(cx: &mut TestAppContext) {
        if review_mode_override().is_some() {
            // FYX_AGENT_REVIEW in the test environment overrides the setting.
            return;
        }
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        assert!(
            !review.read_with(cx, |review, _| review.is_enabled()),
            "off by default"
        );

        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.agent_review.get_or_insert_default().enabled = Some(true);
                });
            });
        });
        cx.run_until_parked();
        assert!(review.read_with(cx, |review, _| review.is_enabled()));

        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.agent_review.get_or_insert_default().enabled = Some(false);
                });
            });
        });
        cx.run_until_parked();
        assert!(!review.read_with(cx, |review, _| review.is_enabled()));
    }

    #[test]
    fn script_driver_is_only_in_dev_builds() {
        // Release builds compile the driver only when the `review-script` feature asks for it.
        assert_eq!(
            SCRIPT_DRIVER_COMPILED,
            cfg!(debug_assertions) || cfg!(feature = "review-script")
        );
        #[cfg(not(any(debug_assertions, feature = "review-script")))]
        assert!(!SCRIPT_DRIVER_COMPILED);
    }

    #[gpui::test]
    async fn failed_host_write_is_shown_in_the_workspace(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({ "a.txt": "x
" }),
        )
        .await;
        let project = Project::test(fs, [util::path!("/p").as_ref()], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());

        // Outside any window update, as the permission responder runs.
        cx.cx
            .update(|cx| report_host_write_failure(&project, "could not be written".into(), cx));
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |workspace, _| {
            workspace.has_notification(&NotificationId::unique::<HostWriteFailed>())
        }));
    }

    #[gpui::test]
    async fn host_writes_accepted_hunks_of_a_fragment(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({ "a.txt": "header\none\ntwo\nthree\nfooter\n" }),
        )
        .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let action_log = cx.new(|_| ActionLog::new(project.clone()));
        let mut edit = ProposedEdit::new(
            PathBuf::from(util::path!("/p/a.txt")),
            "one\ntwo\nthree".into(),
            "ONE\ntwo\nTHREE".into(),
        );
        assert_eq!(edit.hunks().len(), 2);
        edit.set_decision(0, HunkDecision::Accepted).unwrap();
        edit.set_decision(1, HunkDecision::Rejected).unwrap();
        let mut async_cx = cx.to_async();
        write_accepted_hunks(project.clone(), action_log.clone(), &edit, &mut async_cx)
            .await
            .unwrap();
        cx.run_until_parked();
        let saved = fs.load(Path::new(util::path!("/p/a.txt"))).await.unwrap();
        assert_eq!(saved, "header\nONE\ntwo\nthree\nfooter\n");
        assert_eq!(
            action_log.read_with(cx, |action_log, cx| action_log.changed_buffers(cx).count()),
            1,
            "the host's write shows up in the agent diff"
        );
    }
}
