//! Host-owned review of agent-proposed edits.
//!
//! Agent writes reach [`AgentReview`] through the gate in `acp_thread::fyx_review`; permission requests
//! that carry a diff are observed on every [`AcpThread`]. Only the host resolves a review: extensions may
//! observe [`AgentReviewEvent`]s and read reviews, but there is no path for them to decide whether an
//! edit lands.

mod proposed_edit;
mod review_pane;
mod review_rules;
// dev-only: never compiled into release builds unless the `review-script` feature asks for it, because a
// script can approve agent edits the user never saw.
#[cfg(any(debug_assertions, feature = "review-script"))]
mod script_driver;

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
};

use acp_thread::{
    AcpThread, AcpThreadEvent, PermissionRequestId, SelectedPermissionOutcome, ThreadStatus,
    ToolCallContent,
    fyx_review::{AgentWrite, AgentWriteGate, AgentWriteOutcome, set_agent_write_gate},
};
use action_log::ActionLog;
use agent_client_protocol::v2::schema as acp_v2;
use anyhow::{Context as _, Result, anyhow};
use fs::Fs;
use futures::channel::oneshot;
use gpui::Action as _;
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, EntityId, EventEmitter, Global, Task,
    WeakEntity, Window,
};
use language::BufferEditSource;
use project::{PathChange, Project};
use settings::{RegisterSetting, Settings, SettingsStore};
use ui::{ContextMenu, IconPosition};
use workspace::{MultiWorkspace, Toast, notifications::NotificationId};

pub use proposed_edit::{Hunk, HunkDecision, ProposedEdit, Resolution};
pub use review_pane::{
    AcceptAllHunks, AcceptHunk, AcceptTurn, CommentHunk, NextHunk, OpenReviews, PreviousHunk,
    RejectAllHunks, RejectHunk, RejectTurn, ReviewPane, SubmitReview, ToggleReviewMode,
    permission_card_notice,
};
use review_rules::UserAutoAccept;
pub use review_rules::{RULES_FILE, ReviewRules, RuleKind, RuleMatch};

/// What review mode covers, for the status bar and the docs.
pub const COVERAGE: &str = "Agent review covers edits that ACP agents propose with a diff (v1 \
    permission requests and single-file v2 patches), writes through Fyx (fs/write_text_file), and the \
    edit and write tools of Zed's built-in agent. Not reviewed: shell and terminal commands (files \
    they change during a turn are listed as changed without review), the built-in agent's delete, \
    move and copy tools (they ask permission as in Zed), and edits whose diff Fyx cannot read (a \
    notification says so).";

/// The short form, under every review pane.
pub const NOT_REVIEWED: &str = "Not reviewed: terminal commands (files they change are listed above), \
    the built-in agent's delete, move and copy tools, edits Fyx cannot show as a diff.";

/// Overrides the `agent_review.enabled` setting: `0` turns review off, any other value turns it on.
const REVIEW_MODE_VAR: &str = "FYX_AGENT_REVIEW";

/// `agent_review` in settings (schema in `settings_content::AgentReviewSettingsContent`, default in
/// `assets/settings/fyx.json`).
#[derive(Clone, Debug, RegisterSetting)]
pub struct AgentReviewSettings {
    pub enabled: bool,
    /// Patterns whose edits land without review (in review mode). Only user settings set them.
    pub auto_accept: Vec<String>,
}

impl Settings for AgentReviewSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let review = content.agent_review.as_ref();
        Self {
            enabled: review.and_then(|review| review.enabled).unwrap_or(false),
            auto_accept: review
                .and_then(|review| review.auto_accept.clone())
                .unwrap_or_default(),
        }
    }
}

/// The user's `agent_review.auto_accept` patterns.
fn user_auto_accept(cx: &App) -> UserAutoAccept {
    if !cx.has_global::<SettingsStore>() {
        return UserAutoAccept::default();
    }
    UserAutoAccept::new(&AgentReviewSettings::get_global(cx).auto_accept)
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
    review.update(cx, |review, cx| {
        review.user_auto_accept = user_auto_accept(cx)
    });
    cx.observe_global::<SettingsStore>({
        let review = review.downgrade();
        move |cx| {
            let enabled = review_mode(cx);
            let auto_accept = user_auto_accept(cx);
            if let Some(review) = review.upgrade() {
                review.update(cx, |review, cx| {
                    review.user_auto_accept = auto_accept;
                    if review.is_enabled() != enabled {
                        review.set_enabled(enabled, cx);
                    }
                });
            }
        }
    })
    .detach();
    set_agent_write_gate(Rc::new(WriteGate(review.downgrade())), cx);
    cx.observe_new(|thread: &mut AcpThread, _, cx: &mut Context<AcpThread>| {
        cx.subscribe_self(on_thread_event).detach();
        if let Some(review) = AgentReview::global(cx) {
            let session_id = thread.session_id().clone();
            let thread_id = cx.entity_id();
            review.update(cx, |review, _| review.thread_opened(session_id, thread_id));
        }
        let project = thread.project().clone();
        reload_rules(&project, cx);
        cx.subscribe(&project, on_project_event).detach();
        let thread_id = cx.entity_id();
        cx.on_release(move |thread, cx| {
            if let Some(review) = AgentReview::global(cx) {
                let session_id = thread.session_id().clone();
                review.update(cx, |review, cx| {
                    // A reload opens a new thread for the session before the old one goes; the
                    // session's reviews belong to the new one then.
                    if review.thread_released(&session_id, thread_id) {
                        review.close_session(&session_id, cx);
                    }
                });
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

/// One agent turn of one session: every edit the agent proposes until its turn stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TurnId(u64);

/// A file already decided in a turn, for the turn's summary.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnEntry {
    pub path: PathBuf,
    pub resolution: Resolution,
    /// The project rule that decided the file, if one did.
    pub rule: Option<RuleMatch>,
}

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
    /// The agent's turn stopped; no more edits will join it.
    TurnEnded(TurnId),
    /// Files changed on disk during the turn without going through review (see
    /// [`AgentReview::unreviewed_changes`]).
    UnreviewedChanges(TurnId),
}

/// Answers a permission request. For [`PermissionVerdict::ApplyAcceptedAndDeny`] it first writes the
/// accepted hunks; if that fails it answers nothing and returns the error, so the review can stay open
/// and be submitted again.
type PermissionResponder =
    Rc<dyn Fn(PermissionVerdict, ProposedEdit, &mut App) -> Task<Result<()>>>;

enum Responder {
    Permission(PermissionResponder),
    Write(oneshot::Sender<AgentWriteOutcome>),
}

pub struct Review {
    id: ReviewId,
    turn: TurnId,
    rule: Option<RuleMatch>,
    session_id: acp_v2::SessionId,
    source: ReviewSource,
    edit: ProposedEdit,
    responder: Option<Responder>,
    /// A submitted preview whose host write is in flight.
    applying: bool,
    /// Why the last submit could not be carried out; the review stays open.
    error: Option<String>,
}

impl Review {
    pub fn id(&self) -> ReviewId {
        self.id
    }

    pub fn session_id(&self) -> &acp_v2::SessionId {
        &self.session_id
    }

    pub fn turn(&self) -> TurnId {
        self.turn
    }

    /// The project rule that applies to the file (`always_ask` is the only kind that opens a review).
    pub fn rule(&self) -> Option<&RuleMatch> {
        self.rule.as_ref()
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

    /// Why the last submit failed (the review is still open), e.g. the host could not write the
    /// accepted hunks.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// Open reviews of agent edits. Review mode is off by default: while it is off, agent writes and
/// permission requests behave as upstream.
#[derive(Default)]
pub struct AgentReview {
    enabled: bool,
    reviews: Vec<Review>,
    next_id: u64,
    turns: HashMap<acp_v2::SessionId, TurnId>,
    next_turn: u64,
    /// "Accept all in turn" / "reject all in turn": also decides edits that arrive later in the turn.
    turn_decisions: HashMap<TurnId, HunkDecision>,
    turn_history: HashMap<TurnId, Vec<TurnEntry>>,
    /// Files that changed on disk during a turn but never went through its review.
    turn_unreviewed: HashMap<TurnId, BTreeSet<PathBuf>>,
    /// Ended turns whose summaries are kept for open panes, oldest first.
    ended_turns: VecDeque<TurnId>,
    /// `.fyx/review.toml` of each worktree (empty rules for a worktree without the file).
    rules: Vec<ReviewRules>,
    /// The user's `agent_review.auto_accept` patterns: only user settings may loosen review.
    user_auto_accept: UserAutoAccept,
    /// Rules files that changed while sessions ran, waiting for the user to apply them (`None`: the
    /// file was removed).
    pending_rules: HashMap<PathBuf, Option<ReviewRules>>,
    /// Open review panes per turn, across workspaces.
    panes: HashMap<TurnId, usize>,
    /// Live threads per session.
    session_threads: HashMap<acp_v2::SessionId, HashSet<EntityId>>,
}

/// How many ended turns keep their summary (decided files, unreviewed changes).
const ENDED_TURNS_KEPT: usize = 16;

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

    /// Reviews of `turn` still waiting for a decision, oldest first.
    pub fn open_reviews_in_turn(&self, turn: TurnId) -> impl Iterator<Item = &Review> {
        self.reviews
            .iter()
            .filter(move |review| review.turn == turn && !review.is_submitted())
    }

    pub fn turn_history(&self, turn: TurnId) -> &[TurnEntry] {
        self.turn_history.get(&turn).map_or(&[], Vec::as_slice)
    }

    pub fn turn_decision(&self, turn: TurnId) -> Option<HunkDecision> {
        self.turn_decisions.get(&turn).copied()
    }

    /// The session's current turn, started by its first reviewed edit.
    fn turn_for(&mut self, session_id: &acp_v2::SessionId) -> TurnId {
        let next_turn = &mut self.next_turn;
        *self.turns.entry(session_id.clone()).or_insert_with(|| {
            let turn = TurnId(*next_turn);
            *next_turn += 1;
            turn
        })
    }

    /// Starts the session's turn if none is running (the agent began generating).
    pub fn begin_turn(&mut self, session_id: &acp_v2::SessionId) -> TurnId {
        self.turn_for(session_id)
    }

    /// Ends the session's turn: later edits start a new one. The turn's summary stays for its pane.
    pub fn end_turn(&mut self, session_id: &acp_v2::SessionId, cx: &mut Context<Self>) {
        if let Some(turn) = self.turns.remove(session_id) {
            self.turn_decisions.remove(&turn);
            self.ended_turns.push_back(turn);
            while self.ended_turns.len() > ENDED_TURNS_KEPT {
                if let Some(old) = self.ended_turns.pop_front() {
                    self.turn_history.remove(&old);
                    self.turn_unreviewed.remove(&old);
                }
            }
            cx.emit(AgentReviewEvent::TurnEnded(turn));
            cx.notify();
        }
    }

    /// Files that changed on disk during `turn` without going through its review: the agent wrote
    /// them another way (a shell command, its own file tools). Observed only; nothing is blocked.
    pub fn unreviewed_changes(&self, turn: TurnId) -> impl Iterator<Item = &PathBuf> {
        self.turn_unreviewed.get(&turn).into_iter().flatten()
    }

    /// Records files that changed on disk while the session's turn runs. Files the turn reviewed are
    /// skipped: their writes are the reviewed ones.
    pub fn note_changed_paths(
        &mut self,
        session_id: &acp_v2::SessionId,
        paths: impl IntoIterator<Item = PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        let Some(turn) = self.turns.get(session_id).copied() else {
            return;
        };
        let reviewed = self
            .turn_history(turn)
            .iter()
            .map(|entry| entry.path.clone())
            .chain(
                self.reviews
                    .iter()
                    .filter(|review| review.turn == turn)
                    .map(|review| review.edit.abs_path().to_path_buf()),
            )
            .collect::<HashSet<_>>();
        let unreviewed = self.turn_unreviewed.entry(turn).or_default();
        let mut added = false;
        for path in paths {
            if !reviewed.contains(&path) {
                added |= unreviewed.insert(path);
            }
        }
        if added {
            log::info!(
                "fyx: agent review: {} {} changed without review in {turn:?}: {:?}",
                unreviewed.len(),
                if unreviewed.len() == 1 {
                    "file"
                } else {
                    "files"
                },
                unreviewed
            );
            cx.emit(AgentReviewEvent::UnreviewedChanges(turn));
            cx.notify();
        }
    }

    fn thread_opened(&mut self, session_id: acp_v2::SessionId, thread_id: EntityId) {
        self.session_threads
            .entry(session_id)
            .or_default()
            .insert(thread_id);
    }

    /// Returns whether it was the session's last thread.
    fn thread_released(&mut self, session_id: &acp_v2::SessionId, thread_id: EntityId) -> bool {
        let Some(threads) = self.session_threads.get_mut(session_id) else {
            return true;
        };
        threads.remove(&thread_id);
        if threads.is_empty() {
            self.session_threads.remove(session_id);
            true
        } else {
            false
        }
    }

    pub(crate) fn pane_opened(&mut self, turn: TurnId) {
        *self.panes.entry(turn).or_default() += 1;
    }

    /// When the last pane of a turn closes, its open reviews close too: a held write fails ("the
    /// file is unchanged") instead of waiting for a pane nobody can see.
    pub(crate) fn pane_closed(&mut self, turn: TurnId, cx: &mut Context<Self>) {
        let remaining = self
            .panes
            .get(&turn)
            .copied()
            .unwrap_or(0)
            .saturating_sub(1);
        if remaining > 0 {
            self.panes.insert(turn, remaining);
            return;
        }
        self.panes.remove(&turn);
        let ids = self
            .open_reviews_in_turn(turn)
            .map(|review| review.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.close(id, cx);
        }
    }

    /// Reviews waiting for a decision, across all turns.
    pub fn open_review_count(&self) -> usize {
        self.reviews
            .iter()
            .filter(|review| !review.is_submitted())
            .count()
    }

    /// Accepts or rejects every open review of `turn`, and every edit that joins the turn later.
    pub fn decide_turn(
        &mut self,
        turn: TurnId,
        decision: HunkDecision,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        anyhow::ensure!(
            decision != HunkDecision::Pending,
            "a turn is accepted or rejected, not reset"
        );
        self.turn_decisions.insert(turn, decision);
        let ids = self
            .open_reviews_in_turn(turn)
            .map(|review| review.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.open_review_mut(id)?.edit.decide_pending(decision);
            self.submit(id, cx)?;
        }
        Ok(())
    }

    fn record(&mut self, turn: TurnId, edit: &ProposedEdit, rule: Option<RuleMatch>) {
        log::info!(
            "fyx: agent review: {} {:?} in {turn:?}{}",
            edit.abs_path().display(),
            edit.resolution(),
            rule.as_ref()
                .map(|rule| format!(" by rule {}", rule.describe()))
                .unwrap_or_default()
        );
        self.turn_history.entry(turn).or_default().push(TurnEntry {
            path: edit.abs_path().to_path_buf(),
            resolution: edit.resolution(),
            rule,
        });
    }

    /// The project rule for `abs_path`, from the innermost worktree with rules.
    pub fn rule_for(&self, abs_path: &Path) -> Option<RuleMatch> {
        self.rules
            .iter()
            .filter(|rules| rules.relative_path(abs_path).is_some())
            .max_by_key(|rules| rules.root().components().count())?
            .evaluate(abs_path, &self.user_auto_accept)
    }

    /// Replaces the user's `auto_accept` patterns (tests; in the app they follow the settings).
    pub fn set_user_auto_accept(&mut self, patterns: &[String], cx: &mut Context<Self>) {
        self.user_auto_accept = UserAutoAccept::new(patterns);
        cx.notify();
    }

    /// Replaces the rules of the worktree at `root` (`None`: it has no rules file).
    pub fn set_rules(&mut self, root: PathBuf, rules: Option<ReviewRules>, cx: &mut Context<Self>) {
        self.pending_rules.remove(&root);
        self.rules.retain(|existing| existing.root() != root);
        self.rules.extend(rules);
        cx.notify();
    }

    /// Takes the rules file as read now (a new thread starts, or the file changed). A change that only
    /// adds restrictions applies at once. A change that removes any (a `deny` or `always_ask` pattern,
    /// or the whole file) waits until the user applies it: the agent under review may have written the
    /// file, through the terminal or a write the user accepted, and a new thread (a subagent the agent
    /// opens, say) must not apply it either. Returns the restrictions waiting to be removed.
    pub fn load_rules(&mut self, rules: ReviewRules, cx: &mut Context<Self>) -> Vec<String> {
        let root = rules.root().to_path_buf();
        let current = self.rules.iter().find(|existing| existing.root() == root);
        if current.is_some_and(|current| current.same_as(&rules)) {
            self.pending_rules.remove(&root);
            return Vec::new();
        }
        let loosened = current
            .map(|current| current.loosened_by(&rules))
            .unwrap_or_default();
        if loosened.is_empty() {
            self.set_rules(root, Some(rules), cx);
            return Vec::new();
        }
        log::info!(
            "fyx: agent review: the review rules of {} changed and would remove {}; they apply once \
             the user applies them",
            root.display(),
            loosened.join(", ")
        );
        self.pending_rules.insert(root, Some(rules));
        cx.notify();
        loosened
    }

    /// Whether a changed rules file of the worktree at `root` waits for the user.
    pub fn has_pending_rules(&self, root: &Path) -> bool {
        self.pending_rules.contains_key(root)
    }

    /// Applies the changed rules file of the worktree at `root`; the user confirmed it.
    pub fn apply_pending_rules(&mut self, root: &Path, cx: &mut Context<Self>) {
        if let Some(rules) = self.pending_rules.remove(root) {
            log::info!(
                "fyx: agent review: applying the changed rules of {}",
                root.display()
            );
            self.set_rules(root.to_path_buf(), rules, cx);
        }
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
        anyhow::ensure!(!review.applying, "the review is being applied");
        let edit = review.edit.clone();
        match review.responder.as_ref() {
            Some(Responder::Write(_)) => {
                let outcome = write_outcome(&edit)?;
                let Some(Responder::Write(sender)) = review.responder.take() else {
                    return Err(anyhow!("review was already submitted"));
                };
                let (turn, rule) = (review.turn, review.rule.clone());
                if sender.send(outcome).is_err() {
                    log::debug!("agent write finished before its review was submitted");
                }
                self.record(turn, &edit, rule);
                self.reviews.retain(|review| review.id != id);
                cx.emit(AgentReviewEvent::Resolved(id));
                cx.emit(AgentReviewEvent::Closed(id));
            }
            Some(Responder::Permission(respond)) => {
                let respond = respond.clone();
                let verdict = permission_verdict(edit.resolution());
                review.applying = true;
                review.error = None;
                cx.emit(AgentReviewEvent::Updated(id));
                // Answering the permission makes the thread emit an event this entity handles, so it
                // runs outside this update.
                cx.spawn(async move |this, cx| {
                    let answered = cx.update(|cx| respond(verdict, edit, cx));
                    let result = answered.await;
                    this.update(cx, |this, cx| {
                        this.finish_permission(id, verdict, result, cx)
                    })
                    .ok();
                })
                .detach();
            }
            None => return Err(anyhow!("review was already submitted")),
        }
        cx.notify();
        Ok(())
    }

    fn finish_permission(
        &mut self,
        id: ReviewId,
        verdict: PermissionVerdict,
        result: Result<()>,
        cx: &mut Context<Self>,
    ) {
        let Some(review) = self.reviews.iter_mut().find(|review| review.id == id) else {
            return;
        };
        review.applying = false;
        if let Err(error) = result {
            review.error = Some(format!("{error:#}"));
            cx.emit(AgentReviewEvent::Updated(id));
            cx.notify();
            return;
        }
        review.responder = None;
        let (turn, edit, rule) = (review.turn, review.edit.clone(), review.rule.clone());
        self.record(turn, &edit, rule);
        cx.emit(AgentReviewEvent::Resolved(id));
        // An allowed preview stays until the agent's write adopts its decisions; after a denial no
        // write follows.
        if verdict != PermissionVerdict::Allow {
            self.reviews.retain(|review| review.id != id);
            cx.emit(AgentReviewEvent::Closed(id));
        }
        cx.notify();
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
        self.end_turn(session_id, cx);
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
        anyhow::ensure!(!review.applying, "review {id:?} is being applied");
        Ok(review)
    }

    fn insert(
        &mut self,
        session_id: acp_v2::SessionId,
        source: ReviewSource,
        edit: ProposedEdit,
        responder: Responder,
        rule: Option<RuleMatch>,
        cx: &mut Context<Self>,
    ) -> ReviewId {
        let id = ReviewId(self.next_id);
        self.next_id += 1;
        let turn = self.turn_for(&session_id);
        self.reviews.push(Review {
            id,
            turn,
            rule,
            session_id,
            source,
            edit,
            responder: Some(responder),
            applying: false,
            error: None,
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
        rule: Option<RuleMatch>,
        cx: &mut Context<Self>,
    ) -> Option<ReviewId> {
        let rule_kind = rule.as_ref().map(|rule| rule.kind);
        // `deny` and `always_ask` only make review stricter, so they apply in review mode or not.
        let strict_rule = matches!(rule_kind, Some(RuleKind::Deny | RuleKind::AlwaysAsk));
        if (!self.enabled && !strict_rule) || edit.hunks().is_empty() {
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
        let turn = self.turn_for(&session_id);
        let decision = match rule_kind {
            Some(RuleKind::Deny) => Some(HunkDecision::Rejected),
            Some(RuleKind::AutoAccept) => Some(HunkDecision::Accepted),
            Some(RuleKind::AlwaysAsk) => None,
            None => self.turn_decision(turn),
        };
        if let Some(decision) = decision {
            let mut edit = edit;
            edit.decide_pending(decision);
            self.record(turn, &edit, rule);
            let verdict = permission_verdict(edit.resolution());
            cx.spawn(async move |_, cx| {
                if let Err(error) = cx.update(|cx| respond(verdict, edit, cx)).await {
                    log::error!("fyx: agent review: {error:#}");
                }
            })
            .detach();
            cx.notify();
            return None;
        }
        Some(self.insert(
            session_id,
            ReviewSource::Permission(request_id),
            edit,
            Responder::Permission(respond),
            rule,
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
        // Rules match the file the buffer really is, not the spelling the agent sent.
        let resolved_path = write
            .buffer
            .read(cx)
            .file()
            .and_then(|file| file.as_local())
            .map(|file| file.abs_path(cx));
        let rule = resolved_path
            .as_deref()
            .and_then(|path| self.rule_for(path))
            .or_else(|| self.rule_for(&write.abs_path));
        let rule_kind = rule.as_ref().map(|rule| rule.kind);
        let strict_rule = matches!(rule_kind, Some(RuleKind::Deny | RuleKind::AlwaysAsk));
        if !self.enabled && !strict_rule {
            return Task::ready(Ok(AgentWriteOutcome {
                content: Some(write.new_text),
                feedback: None,
            }));
        }
        let mut edit = ProposedEdit::new(write.abs_path, write.old_text, write.new_text);
        if let Some(rule) = rule.clone().filter(|rule| rule.kind == RuleKind::Deny) {
            edit.decide_all(HunkDecision::Rejected);
            let turn = self.turn_for(&write.session_id);
            let feedback = format!(
                "The project's review rules deny agent edits to {} ({}). The file is unchanged; do \
                 not retry this edit, ask the user instead.",
                edit.abs_path().display(),
                rule.describe()
            );
            self.record(turn, &edit, Some(rule));
            cx.notify();
            return Task::ready(Ok(AgentWriteOutcome {
                content: None,
                feedback: Some(feedback),
            }));
        }
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

        let turn = self.turn_for(&write.session_id);
        let decision = match rule_kind {
            Some(RuleKind::AutoAccept) => Some(HunkDecision::Accepted),
            // `always_ask` overrides "accept all in turn".
            Some(RuleKind::AlwaysAsk) => None,
            _ => self.turn_decision(turn),
        };
        if let Some(decision) = decision {
            edit.decide_pending(decision);
        }
        if !edit.resolution().is_pending() {
            self.record(turn, &edit, rule);
            cx.notify();
            return Task::ready(write_outcome(&edit));
        }
        let (sender, receiver) = oneshot::channel();
        self.insert(
            write.session_id,
            ReviewSource::Write,
            edit,
            Responder::Write(sender),
            rule,
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
            let session_id = thread.session_id().clone();
            let diffs = legacy_diff_count(thread, *request_id);
            if diffs > 1 {
                if review.read(cx).is_enabled() {
                    report_unreviewable_edit(
                        thread.project(),
                        &format!("one tool call edits {diffs} files"),
                        cx,
                    );
                }
                return;
            }
            if let Some((edit, respond)) = permission_preview(thread, *request_id, cx) {
                review.update(cx, |review, cx| {
                    let rule = review.rule_for(edit.abs_path());
                    review.open_permission_preview(
                        session_id,
                        *request_id,
                        edit,
                        respond,
                        rule,
                        cx,
                    );
                });
                return;
            }
            match v2_edit_request(thread, *request_id, cx) {
                V2EditRequest::NotAnEdit => {}
                V2EditRequest::Unreviewable(reason) => {
                    if review.read(cx).is_enabled() {
                        report_unreviewable_edit(thread.project(), &reason, cx);
                    }
                }
                V2EditRequest::Patch {
                    path,
                    patch,
                    created,
                    respond,
                } => {
                    let project = thread.project().clone();
                    let request_id = *request_id;
                    cx.spawn(async move |thread, cx| {
                        let preview = v2_patch_preview(&project, path, &patch, created, cx).await;
                        let still_pending = thread
                            .read_with(cx, |thread, _| {
                                thread.permission_request(request_id).is_some()
                            })
                            .unwrap_or(false);
                        match preview {
                            _ if !still_pending => {}
                            Ok(edit) => review.update(cx, |review, cx| {
                                let rule = review.rule_for(edit.abs_path());
                                review.open_permission_preview(
                                    session_id, request_id, edit, respond, rule, cx,
                                );
                            }),
                            Err(error) => cx.update(|cx| {
                                if review.read(cx).is_enabled() {
                                    report_unreviewable_edit(&project, &format!("{error:#}"), cx);
                                }
                            }),
                        }
                    })
                    .detach();
                }
            }
        }
        AcpThreadEvent::ToolAuthorizationReceived(request_id) => {
            review.update(cx, |review, cx| review.permission_answered(*request_id, cx));
        }
        AcpThreadEvent::StatusChanged if thread.status() == ThreadStatus::Generating => {
            if review.read(cx).is_enabled() {
                let session_id = thread.session_id().clone();
                review.update(cx, |review, _| {
                    review.begin_turn(&session_id);
                });
            }
        }
        AcpThreadEvent::Stopped { .. } | AcpThreadEvent::Error | AcpThreadEvent::ConnectionLost => {
            // Held writes fail ("the file is unchanged") and allowed previews expire: the agent
            // stopped waiting for them.
            let session_id = thread.session_id().clone();
            review.update(cx, |review, cx| review.close_session(&session_id, cx));
        }
        _ => {}
    }
}

/// Files that changed on disk in the thread's project, for the turn's unreviewed changes.
fn on_project_event(
    thread: &mut AcpThread,
    project: Entity<Project>,
    event: &project::Event,
    cx: &mut Context<AcpThread>,
) {
    let project::Event::WorktreeUpdatedEntries(worktree_id, changes) = event else {
        return;
    };
    if changes
        .iter()
        .any(|(path, _, _)| path.as_unix_str() == RULES_FILE)
    {
        reload_rules(&project, cx);
    }
    let Some(review) = AgentReview::global(cx) else {
        return;
    };
    if !review.read(cx).is_enabled() {
        return;
    }
    let Some(worktree) = project.read(cx).worktree_for_id(*worktree_id, cx) else {
        return;
    };
    let worktree = worktree.read(cx);
    let paths = changes
        .iter()
        .filter(|(_, _, change)| {
            matches!(
                change,
                PathChange::Added
                    | PathChange::Updated
                    | PathChange::AddedOrUpdated
                    | PathChange::Removed
            )
        })
        .filter(|(_, entry_id, _)| {
            !worktree
                .entry_for_id(*entry_id)
                .is_some_and(|entry| entry.is_dir())
        })
        .filter(|(path, _, _)| !path.components().any(|component| component == ".git"))
        .map(|(path, _, _)| worktree.absolutize(path))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return;
    }
    let session_id = thread.session_id().clone();
    review.update(cx, |review, cx| {
        review.note_changed_paths(&session_id, paths, cx)
    });
}

/// Answers a permission request through `answer`. For [`PermissionVerdict::ApplyAcceptedAndDeny`]
/// the accepted hunks are written first; if they cannot be, nothing is answered and the error goes back
/// to the review, which stays open.
fn make_responder(
    thread: WeakEntity<AcpThread>,
    answer: impl Fn(&mut AcpThread, PermissionVerdict, &mut Context<AcpThread>) + 'static,
) -> PermissionResponder {
    let answer = Rc::new(answer);
    Rc::new(move |verdict, edit, cx| {
        let thread = thread.clone();
        let answer = answer.clone();
        cx.spawn(async move |cx| {
            // The accepted hunks land before the agent hears the denial and reads the file again.
            if verdict == PermissionVerdict::ApplyAcceptedAndDeny {
                let (project, action_log) = thread.read_with(cx, |thread, _| {
                    (thread.project().clone(), thread.action_log().clone())
                })?;
                if let Err(error) =
                    write_accepted_hunks(project.clone(), action_log, &edit, cx).await
                {
                    let message = format!(
                        "Agent review: the accepted hunks of {} could not be written: {error:#}",
                        edit.abs_path().display()
                    );
                    log::error!("{message}");
                    cx.update(|cx| report_host_write_failure(&project, message, cx));
                    return Err(error);
                }
            }
            thread.update(cx, |thread, cx| answer(thread, verdict, cx))
        })
    })
}

/// How an ACP v2 permission request relates to review.
enum V2EditRequest {
    /// Not a file edit Fyx can see (no diff in the tool call).
    NotAnEdit,
    /// A file edit that cannot be shown hunk by hunk; the reason is told to the user.
    Unreviewable(String),
    /// A single-file patch, previewed once the current file text is loaded.
    Patch {
        path: PathBuf,
        patch: String,
        created: bool,
        respond: PermissionResponder,
    },
}

/// ACP v2 edits arrive as a permission request whose tool call carries a patch diff. v2 has no
/// `fs/write_text_file`, so the agent writes the file itself once allowed: the preview is the only
/// place to review it.
fn v2_edit_request(
    thread: &AcpThread,
    request_id: PermissionRequestId,
    cx: &Context<AcpThread>,
) -> V2EditRequest {
    let Some(request) = thread
        .permission_request(request_id)
        .and_then(|request| request.generic_request())
    else {
        return V2EditRequest::NotAnEdit;
    };
    let Some(acp_v2::RequestPermissionSubject::ToolCall(subject)) = &request.subject else {
        return V2EditRequest::NotAnEdit;
    };
    let Some((_, tool_call)) = thread.tool_call(&subject.tool_call.tool_call_id) else {
        return V2EditRequest::NotAnEdit;
    };
    let Some(diff) = tool_call
        .content()
        .iter()
        .find_map(|content| match content {
            ToolCallContent::DiffPatch { source, .. } => Some(source),
            _ => None,
        })
    else {
        return V2EditRequest::NotAnEdit;
    };
    let [change] = diff.changes.as_slice() else {
        return V2EditRequest::Unreviewable(format!(
            "the edit changes {} files",
            diff.changes.len()
        ));
    };
    let (path, created) = match &change.operation {
        acp_v2::DiffChangeOperation::Modify(change) => (change.path.0.clone(), false),
        acp_v2::DiffChangeOperation::Add(change) => (change.path.0.clone(), true),
        _ => {
            return V2EditRequest::Unreviewable(acp_thread::diff_change_label(change));
        }
    };
    let Some(patch) = diff.patch.as_ref().map(|patch| patch.text.clone()) else {
        return V2EditRequest::Unreviewable("the agent sent no patch text".into());
    };
    let option = |kind: acp_v2::PermissionOptionKind| {
        request
            .options
            .iter()
            .find(|option| option.kind == kind)
            .map(|option| option.option_id.clone())
    };
    let (Some(allow_option), Some(deny_option)) = (
        option(acp_v2::PermissionOptionKind::AllowOnce),
        option(acp_v2::PermissionOptionKind::RejectOnce),
    ) else {
        return V2EditRequest::Unreviewable(
            "the request offers no allow-once and reject-once".into(),
        );
    };
    let respond = make_responder(cx.weak_entity(), move |thread, verdict, cx| {
        let option_id = match verdict {
            PermissionVerdict::Allow => allow_option.clone(),
            PermissionVerdict::Deny | PermissionVerdict::ApplyAcceptedAndDeny => {
                deny_option.clone()
            }
        };
        thread.select_permission_option(request_id, option_id, cx);
    });
    V2EditRequest::Patch {
        path,
        patch,
        created,
        respond,
    }
}

/// The proposed edit of a v2 patch, against the file as it is now (not the agent's copy of it).
async fn v2_patch_preview(
    project: &Entity<Project>,
    path: PathBuf,
    patch: &str,
    created: bool,
    cx: &mut AsyncApp,
) -> Result<ProposedEdit> {
    let old_text = if created {
        String::new()
    } else {
        let open = project.update(cx, |project, cx| {
            let project_path = project
                .project_path_for_absolute_path(&path, cx)
                .context("the file is outside the project")?;
            anyhow::Ok(project.open_buffer(project_path, cx))
        })?;
        let buffer = open.await?;
        cx.update(|cx| buffer.read(cx).text())
    };
    let new_text = language::apply_diff_patch(&old_text, patch)
        .context("the patch does not apply to the file as it is now")?;
    Ok(ProposedEdit::new(path, old_text, new_text))
}

struct UnreviewableEdit;

fn report_unreviewable_edit(project: &Entity<Project>, reason: &str, cx: &mut App) {
    notify_project::<UnreviewableEdit>(
        project,
        format!(
            "Agent review: Fyx cannot review this edit hunk by hunk ({reason}). Decide it in the \
             agent panel; it is not reviewed."
        ),
        cx,
    );
}

/// How many v1 diffs the tool call of a pending permission request carries.
fn legacy_diff_count(thread: &AcpThread, request_id: PermissionRequestId) -> usize {
    thread
        .permission_request(request_id)
        .and_then(|request| request.legacy_tool_call_id())
        .and_then(|tool_call_id| thread.tool_call(tool_call_id))
        .map_or(0, |(_, tool_call)| {
            tool_call
                .content()
                .iter()
                .filter(|content| matches!(content, ToolCallContent::LegacyDiff { .. }))
                .count()
        })
}

/// The diff of a pending v1 edit permission and the host's way to answer it. v1 diffs carry whole old
/// and new texts (or fragments of them).
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
    let respond = make_responder(cx.weak_entity(), move |thread, verdict, cx| {
        let (option_id, option_kind) = match verdict {
            PermissionVerdict::Allow => (
                allow_option.clone(),
                acp_v2::PermissionOptionKind::AllowOnce,
            ),
            PermissionVerdict::Deny | PermissionVerdict::ApplyAcceptedAndDeny => (
                deny_option.clone(),
                acp_v2::PermissionOptionKind::RejectOnce,
            ),
        };
        let outcome = SelectedPermissionOutcome::new(option_id, option_kind);
        thread.authorize_permission_request(request_id, outcome, cx);
    });
    Some((edit, respond))
}

struct HostWriteFailed;
struct RulesFileInvalid;

/// Shows a failed host write in the workspaces of `project`. ACP has no way to attach a message to a
/// permission answer, so the user is the one who has to learn that approved hunks did not land.
fn report_host_write_failure(project: &Entity<Project>, message: String, cx: &mut App) {
    notify_project::<HostWriteFailed>(project, message, cx);
}

struct RulesChanged;
struct RulesIgnoredKey;

/// Reads `.fyx/review.toml` of every visible worktree of `project` (see [`AgentReview::load_rules`]).
/// A file that cannot be read or parsed keeps the previous rules and is reported: rules never
/// disappear silently.
fn reload_rules(project: &Entity<Project>, cx: &mut App) {
    let Some(review) = AgentReview::global(cx) else {
        return;
    };
    let fs = project.read(cx).fs().clone();
    let worktrees = project.read(cx).visible_worktrees(cx).collect::<Vec<_>>();
    for worktree in worktrees {
        let root = worktree.read(cx).abs_path().to_path_buf();
        let rules_path = review_rules::rules_file_path(&root);
        let review = review.downgrade();
        let project = project.downgrade();
        let fs = fs.clone();
        cx.spawn(async move |cx| {
            let notify = |kind: fn(&Entity<Project>, String, &mut App), message: String, cx: &mut AsyncApp| {
                cx.update(|cx| {
                    if let Some(project) = project.upgrade() {
                        kind(&project, message, cx);
                    }
                });
            };
            let rules = match fs.metadata(&rules_path).await {
                Ok(None) => ReviewRules::none(root.clone()),
                Ok(Some(_)) => {
                    let parsed = fs
                        .load(&rules_path)
                        .await
                        .and_then(|text| ReviewRules::parse(root.clone(), &text));
                    match parsed {
                        Ok(rules) => rules,
                        Err(error) => {
                            let message = format!(
                                "Agent review: {} could not be used ({error:#}); the previous \
                                 review rules stay in force.",
                                rules_path.display()
                            );
                            log::error!("{message}");
                            notify(notify_project::<RulesFileInvalid>, message, cx);
                            return;
                        }
                    }
                }
                Err(error) => {
                    let message = format!(
                        "Agent review: {} could not be checked ({error:#}); the previous review \
                         rules stay in force.",
                        rules_path.display()
                    );
                    log::error!("{message}");
                    notify(notify_project::<RulesFileInvalid>, message, cx);
                    return;
                }
            };
            if rules.ignored_auto_accept() {
                notify(
                    notify_project::<RulesIgnoredKey>,
                    format!(
                        "Agent review: auto_accept in {} is ignored. A project can only make review \
                         stricter; patterns that skip review belong in your settings \
                         (agent_review.auto_accept).",
                        rules_path.display()
                    ),
                    cx,
                );
            }
            let loosened = match review.update(cx, |review, cx| review.load_rules(rules, cx)) {
                Ok(loosened) => loosened,
                Err(error) => {
                    log::debug!("review closed before its rules loaded: {error}");
                    return;
                }
            };
            if loosened.is_empty() {
                return;
            }
            cx.update(|cx| {
                let Some(project) = project.upgrade() else {
                    return;
                };
                let message = format!(
                    "Agent review: {} changed and would remove these restrictions: {}. An agent may \
                     have changed it, so the new rules apply only when you apply them.",
                    rules_path.display(),
                    loosened.join(", ")
                );
                let review = review.clone();
                let apply: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |_, cx| {
                    if let Some(review) = review.upgrade() {
                        review.update(cx, |review, cx| review.apply_pending_rules(&root, cx));
                    }
                });
                notify_project_with_action::<RulesChanged>(
                    &project,
                    message,
                    Some(("Apply new review rules", apply)),
                    cx,
                );
            });
        })
        .detach();
    }
}

fn notify_project<T: 'static>(project: &Entity<Project>, message: String, cx: &mut App) {
    notify_project_with_action::<T>(project, message, None, cx);
}

fn notify_project_with_action<T: 'static>(
    project: &Entity<Project>,
    message: String,
    action: Option<(&'static str, Rc<dyn Fn(&mut Window, &mut App)>)>,
    cx: &mut App,
) {
    for window in cx.windows() {
        let Some(multi_workspace) = window.downcast::<MultiWorkspace>() else {
            continue;
        };
        let result = multi_workspace.update(cx, |multi_workspace, _, cx| {
            let workspace = multi_workspace.workspace().clone();
            if workspace.read(cx).project() == project {
                let mut toast = Toast::new(NotificationId::unique::<T>(), message.clone());
                if let Some((label, action)) = action.clone() {
                    toast = toast.on_click(label, move |window, cx| action(window, cx));
                }
                workspace.update(cx, |workspace, cx| workspace.show_toast(toast, cx));
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
    // Saving would also write the user's unsaved edits elsewhere in the file.
    let was_dirty = cx.update(|cx| buffer.read(cx).is_dirty());
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
    if was_dirty {
        log::info!(
            "fyx: agent review: {} has unsaved edits; the accepted hunks are applied but not saved",
            edit.abs_path().display()
        );
        return Ok(());
    }
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
    use gpui::UpdateGlobal as _;
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
        let respond: PermissionResponder = Rc::new({
            let verdicts = verdicts.clone();
            move |verdict, _, _| {
                verdicts.borrow_mut().push(verdict);
                Task::ready(Ok(()))
            }
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
                review.open_permission_preview(session(), request_id, preview, respond, None, cx)
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
        let respond: PermissionResponder = Rc::new({
            let answers = answers.clone();
            move |verdict, edit, _| {
                answers.borrow_mut().push((verdict, edit));
                Task::ready(Ok(()))
            }
        });
        let preview = ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into());
        let id = review
            .update(cx, |review, cx| {
                review.open_permission_preview(
                    session(),
                    new_request_id(),
                    preview,
                    respond,
                    None,
                    cx,
                )
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
        assert!(
            review.read_with(cx, |review, _| review.reviews().is_empty()),
            "no write follows a denial"
        );
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
                Rc::new(|_, _, _| Task::ready(Ok(()))),
                None,
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
    async fn turn_decision_covers_open_and_later_edits(cx: &mut TestAppContext) {
        let (review, events) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let first = gate_write(&review, agent_write("/p/a.txt", cx), cx);
        let second = gate_write(&review, agent_write("/p/b.txt", cx), cx);
        let turn = review.read_with(cx, |review, _| {
            let turns = review
                .reviews()
                .iter()
                .map(|review| review.turn())
                .collect::<Vec<_>>();
            assert_eq!(turns.len(), 2);
            assert_eq!(turns[0], turns[1], "edits of one turn share it");
            turns[0]
        });

        review.update(cx, |review, cx| {
            review
                .decide_turn(turn, HunkDecision::Accepted, cx)
                .unwrap();
            assert_eq!(review.open_reviews_in_turn(turn).count(), 0);
            assert_eq!(review.turn_history(turn).len(), 2);
        });
        assert_eq!(first.await.unwrap().content.as_deref(), Some(NEW));
        assert_eq!(second.await.unwrap().content.as_deref(), Some(NEW));

        // A later edit of the same turn is accepted without a review.
        let opened_before = events.borrow().len();
        let later = gate_write(&review, agent_write("/p/c.txt", cx), cx)
            .await
            .unwrap();
        assert_eq!(later.content.as_deref(), Some(NEW));
        assert!(
            !events.borrow()[opened_before..]
                .iter()
                .any(|event| matches!(event, AgentReviewEvent::Opened(_)))
        );
        assert_eq!(
            review.read_with(cx, |review, _| review.turn_history(turn).len()),
            3
        );

        // The next turn asks again.
        review.update(cx, |review, cx| review.end_turn(&session(), cx));
        assert!(events.borrow().contains(&AgentReviewEvent::TurnEnded(turn)));
        let next = gate_write(&review, agent_write("/p/a.txt", cx), cx);
        let next_turn = review.read_with(cx, |review, _| {
            assert_eq!(review.reviews().len(), 1, "held for review again");
            review.reviews()[0].turn()
        });
        assert_ne!(next_turn, turn);
        review.update(cx, |review, cx| {
            review
                .decide_turn(next_turn, HunkDecision::Rejected, cx)
                .unwrap()
        });
        assert_eq!(next.await.unwrap().content, None);
    }

    #[gpui::test]
    async fn unreviewed_changes_are_recorded_per_turn(cx: &mut TestAppContext) {
        let (review, events) = setup(cx);
        let path = |name: &str| PathBuf::from(format!("/p/{name}"));
        review.update(cx, |review, cx| {
            review.note_changed_paths(&session(), [path("x.txt")], cx);
            assert!(review.turns.is_empty(), "review mode off: nothing tracked");
            review.set_enabled(true, cx);
            review.note_changed_paths(&session(), [path("x.txt")], cx);
            assert!(review.turns.is_empty(), "no running turn: nothing tracked");
        });

        // a.txt goes through review, x.txt changes on disk directly.
        let held = gate_write(&review, agent_write("/p/a.txt", cx), cx);
        let turn = review.read_with(cx, |review, _| review.reviews()[0].turn());
        review.update(cx, |review, cx| {
            review.note_changed_paths(&session(), [path("a.txt"), path("x.txt")], cx);
            review.note_changed_paths(&session(), [path("x.txt")], cx);
        });
        review.read_with(cx, |review, _| {
            assert_eq!(
                review.unreviewed_changes(turn).collect::<Vec<_>>(),
                [&path("x.txt")]
            );
        });
        let unreviewed_events = events
            .borrow()
            .iter()
            .filter(|event| **event == AgentReviewEvent::UnreviewedChanges(turn))
            .count();
        assert_eq!(unreviewed_events, 1, "one event per new file");

        // The summary outlives the turn for its pane.
        let id = review.read_with(cx, |review, _| review.reviews()[0].id());
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Accepted, cx).unwrap();
            review.submit(id, cx).unwrap();
            review.end_turn(&session(), cx);
            assert_eq!(review.unreviewed_changes(turn).count(), 1);
            assert_eq!(review.turn_history(turn).len(), 1);
        });
        held.await.unwrap();
    }

    #[gpui::test]
    async fn files_written_around_review_are_noticed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({ "a.txt": "a\n", "sub": { "b.txt": "b\n" } }),
        )
        .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let thread = cx
            .update(|cx| {
                acp_thread::AgentConnection::new_session(
                    connection.clone(),
                    project.clone(),
                    workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());
        cx.run_until_parked();

        // Nothing is tracked outside a turn.
        fs.save(
            Path::new(util::path!("/p/a.txt")),
            &"changed before the turn\n".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        let turn = review.update(cx, |review, _| review.begin_turn(&session_id));
        assert_eq!(
            review.read_with(cx, |review, _| review.unreviewed_changes(turn).count()),
            0
        );

        // The agent's shell rewrites a file during its turn.
        fs.save(
            Path::new(util::path!("/p/sub/b.txt")),
            &"rewritten by sed\n".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        review.read_with(cx, |review, _| {
            assert_eq!(
                review.unreviewed_changes(turn).cloned().collect::<Vec<_>>(),
                [PathBuf::from(util::path!("/p/sub/b.txt"))]
            );
        });
        let pane_turns = workspace.read_with(cx, |workspace, cx| {
            workspace
                .items_of_type::<ReviewPane>(cx)
                .map(|pane| pane.read(cx).turn())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            pane_turns,
            [turn],
            "the turn's pane opens to show the change"
        );
        drop(thread);
    }

    fn set_test_rules(review: &Entity<AgentReview>, text: &str, cx: &mut TestAppContext) {
        let rules = ReviewRules::parse(PathBuf::from("/p"), text).unwrap();
        review.update(cx, |review, cx| {
            review.set_rules(PathBuf::from("/p"), Some(rules), cx)
        });
    }

    fn recording_responder() -> (PermissionResponder, Rc<RefCell<Vec<PermissionVerdict>>>) {
        let verdicts = Rc::new(RefCell::new(Vec::new()));
        let respond: PermissionResponder = Rc::new({
            let verdicts = verdicts.clone();
            move |verdict, _, _| {
                verdicts.borrow_mut().push(verdict);
                Task::ready(Ok(()))
            }
        });
        (respond, verdicts)
    }

    #[gpui::test]
    async fn deny_rule_rejects_with_a_reason_even_with_review_off(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        set_test_rules(&review, r#"deny = ["*.env"]"#, cx);

        let mut write = agent_write("/p/config/.env", cx);
        write.new_text = "SECRET=1\n".into();
        let outcome = gate_write(&review, write, cx).await.unwrap();
        assert_eq!(outcome.content, None);
        let feedback = outcome.feedback.unwrap();
        assert!(feedback.contains("deny = \"*.env\""), "{feedback}");
        assert!(feedback.contains("do not retry"), "{feedback}");

        let (respond, verdicts) = recording_responder();
        let preview = ProposedEdit::new(PathBuf::from("/p/.env"), "A=1\n".into(), "A=2\n".into());
        let rule = review.read_with(cx, |review, _| review.rule_for(preview.abs_path()));
        let opened = review.update(cx, |review, cx| {
            review.open_permission_preview(session(), new_request_id(), preview, respond, rule, cx)
        });
        assert_eq!(opened, None, "no review opens for a denied file");
        cx.run_until_parked();
        assert_eq!(*verdicts.borrow(), [PermissionVerdict::Deny]);

        // Other files are untouched by the rule; with review off they pass through.
        let outcome = gate_write(&review, agent_write("/p/src/main.rs", cx), cx)
            .await
            .unwrap();
        assert_eq!(outcome.content.as_deref(), Some(NEW));
    }

    #[gpui::test]
    async fn always_ask_rule_opens_a_review_regardless(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        set_test_rules(&review, r#"always_ask = ["Cargo.lock"]"#, cx);

        // Review mode is off, yet the lock file is held.
        let held = gate_write(&review, agent_write("/p/Cargo.lock", cx), cx);
        let (id, turn) = review.read_with(cx, |review, _| {
            let held = &review.reviews()[0];
            assert_eq!(held.rule().map(|rule| rule.kind), Some(RuleKind::AlwaysAsk));
            (held.id(), held.turn())
        });

        // "Accept all in turn" does not cover it either.
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Accepted, cx).unwrap();
            review.submit(id, cx).unwrap();
            review
                .decide_turn(turn, HunkDecision::Accepted, cx)
                .unwrap();
        });
        held.await.unwrap();
        let again = gate_write(&review, agent_write("/p/Cargo.lock", cx), cx);
        assert_eq!(
            review.read_with(cx, |review, _| review.open_reviews_in_turn(turn).count()),
            1,
            "held despite the turn decision"
        );
        let other = gate_write(&review, agent_write("/p/src/lib.rs", cx), cx)
            .await
            .unwrap();
        assert_eq!(
            other.content.as_deref(),
            Some(NEW),
            "other files follow the turn"
        );
        let id = review.read_with(cx, |review, _| {
            review.open_reviews_in_turn(turn).next().unwrap().id()
        });
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Rejected, cx).unwrap();
            review.submit(id, cx).unwrap();
        });
        assert_eq!(again.await.unwrap().content, None);
        let history = review.read_with(cx, |review, _| review.turn_history(turn).to_vec());
        assert!(history.iter().any(|entry| {
            entry
                .rule
                .as_ref()
                .is_some_and(|rule| rule.kind == RuleKind::AlwaysAsk)
        }));
    }

    #[gpui::test]
    async fn auto_accept_comes_from_user_settings_only(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));

        // A project file cannot wave edits through (integration-4 finding 1).
        set_test_rules(&review, r#"auto_accept = ["**"]"#, cx);
        let _held = gate_write(&review, agent_write("/p/docs/guide.md", cx), cx);
        assert_eq!(review.read_with(cx, |review, _| review.reviews().len()), 1);
        let id = review.read_with(cx, |review, _| review.reviews()[0].id());
        review.update(cx, |review, cx| review.close(id, cx));

        // The user's settings can.
        review.update(cx, |review, cx| {
            review.set_user_auto_accept(&["docs/**".to_string()], cx)
        });
        let outcome = gate_write(&review, agent_write("/p/docs/guide.md", cx), cx)
            .await
            .unwrap();
        assert_eq!(outcome.content.as_deref(), Some(NEW));
        assert!(review.read_with(cx, |review, _| review.reviews().is_empty()));
        let (respond, verdicts) = recording_responder();
        let preview = ProposedEdit::new(PathBuf::from("/p/docs/a.md"), OLD.into(), NEW.into());
        let rule = review.read_with(cx, |review, _| review.rule_for(preview.abs_path()));
        assert!(
            rule.unwrap()
                .describe()
                .contains("agent_review.auto_accept")
        );
        let rule = review.read_with(cx, |review, _| review.rule_for(preview.abs_path()));
        review.update(cx, |review, cx| {
            review.open_permission_preview(session(), new_request_id(), preview, respond, rule, cx)
        });
        cx.run_until_parked();
        assert_eq!(*verdicts.borrow(), [PermissionVerdict::Allow]);
    }

    /// integration-4 finding 1: in review mode, the agent writes `auto_accept = ["**"]` into
    /// `.fyx/review.toml` from its terminal and opens a new thread (a subagent): its later edits must
    /// still be reviewed, and the project's `deny` must stay.
    #[gpui::test]
    async fn an_agent_cannot_switch_review_off_through_the_rules_file(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({
                ".fyx": { "review.toml": "deny = [\"*.env\"]\n" },
                "src": { "main.rs": OLD },
            }),
        )
        .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let new_thread = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                acp_thread::AgentConnection::new_session(
                    connection.clone(),
                    project.clone(),
                    workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                    cx,
                )
            })
        };
        let _thread = new_thread(cx).await.unwrap();
        cx.run_until_parked();

        fs.save(
            Path::new(util::path!("/p/.fyx/review.toml")),
            &"auto_accept = [\"**\"]\n".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        let _subagent = new_thread(cx).await.unwrap();
        cx.run_until_parked();

        let root = PathBuf::from(util::path!("/p"));
        let rule = |name: &str, cx: &mut TestAppContext| {
            let path = root.join(name);
            review.read_with(cx, |review, _| review.rule_for(&path).map(|rule| rule.kind))
        };
        assert_eq!(
            rule(".env", cx),
            Some(RuleKind::Deny),
            "deny stays until the user agrees"
        );
        assert_eq!(rule("src/main.rs", cx), None, "nothing is auto-accepted");
        assert!(review.read_with(cx, |review, _| review.has_pending_rules(&root)));

        let mut write = agent_write(util::path!("/p/src/main.rs"), cx);
        write.abs_path = root.join("src").join("main.rs");
        let _held = gate_write(&review, write, cx);
        assert_eq!(
            review.read_with(cx, |review, _| review.open_review_count()),
            1,
            "the agent's next edit is still reviewed"
        );

        // Even confirmed, the file cannot auto-accept: it only drops the deny.
        review.update(cx, |review, cx| review.apply_pending_rules(&root, cx));
        assert_eq!(rule(".env", cx), None);
        assert_eq!(rule("src/main.rs", cx), None);
    }

    #[gpui::test]
    async fn stricter_rules_apply_at_once_and_read_errors_keep_the_rules(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        let root = PathBuf::from("/p");
        let parse = |text: &str| ReviewRules::parse(root.clone(), text).unwrap();
        review.update(cx, |review, cx| {
            assert!(
                review
                    .load_rules(parse(r#"deny = ["*.env"]"#), cx)
                    .is_empty()
            );
            // Adding a restriction applies without asking.
            assert!(
                review
                    .load_rules(parse(r#"deny = ["*.env", "*.pem"]"#), cx)
                    .is_empty()
            );
            assert_eq!(
                review
                    .rule_for(Path::new("/p/key.pem"))
                    .map(|rule| rule.kind),
                Some(RuleKind::Deny)
            );
            // Removing the file removes restrictions: it waits.
            assert!(
                !review
                    .load_rules(ReviewRules::none(root.clone()), cx)
                    .is_empty()
            );
            assert_eq!(
                review
                    .rule_for(Path::new("/p/key.pem"))
                    .map(|rule| rule.kind),
                Some(RuleKind::Deny)
            );
        });
    }

    #[gpui::test]
    async fn rules_file_is_loaded_and_reloaded(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({
                ".fyx": { "review.toml": "deny = [\"*.env\"]\n" },
                "a.txt": "a\n",
            }),
        )
        .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let _thread = cx
            .update(|cx| {
                acp_thread::AgentConnection::new_session(
                    connection.clone(),
                    project.clone(),
                    workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        cx.run_until_parked();
        let kind = |name: &str, cx: &mut TestAppContext| {
            let path = PathBuf::from(util::path!("/p")).join(name);
            review.read_with(cx, |review, _| review.rule_for(&path).map(|rule| rule.kind))
        };
        assert_eq!(kind(".env", cx), Some(RuleKind::Deny));
        assert_eq!(kind("Cargo.lock", cx), None);

        fs.save(
            Path::new(util::path!("/p/.fyx/review.toml")),
            &"always_ask = [\"Cargo.lock\"]\n".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        // Changed during a session: nothing applies until the user confirms.
        assert_eq!(kind(".env", cx), Some(RuleKind::Deny));
        assert_eq!(kind("Cargo.lock", cx), None);
        let root = PathBuf::from(util::path!("/p"));
        review.update(cx, |review, cx| {
            assert!(review.has_pending_rules(&root));
            review.apply_pending_rules(&root, cx);
        });
        assert_eq!(kind(".env", cx), None);
        assert_eq!(kind("Cargo.lock", cx), Some(RuleKind::AlwaysAsk));
    }

    #[gpui::test]
    async fn failed_host_write_keeps_the_review_open(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let answered = Rc::new(RefCell::new(Vec::new()));
        let respond: PermissionResponder = Rc::new({
            let answered = answered.clone();
            move |verdict, _, _| {
                if verdict == PermissionVerdict::ApplyAcceptedAndDeny {
                    // As when the fragment no longer occurs exactly once.
                    return Task::ready(Err(anyhow!("the text occurs more than once")));
                }
                answered.borrow_mut().push(verdict);
                Task::ready(Ok(()))
            }
        });
        let preview = ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into());
        let id = review
            .update(cx, |review, cx| {
                review.open_permission_preview(
                    session(),
                    new_request_id(),
                    preview,
                    respond,
                    None,
                    cx,
                )
            })
            .unwrap();
        review.update(cx, |review, cx| {
            review
                .set_decision(id, 0, HunkDecision::Accepted, cx)
                .unwrap();
            review
                .set_decision(id, 1, HunkDecision::Rejected, cx)
                .unwrap();
            review.submit(id, cx).unwrap();
            assert!(
                review.submit(id, cx).is_err(),
                "no second submit while applying"
            );
        });
        cx.run_until_parked();
        review.read_with(cx, |review, _| {
            let open = review.review(id).expect("the review stays open");
            assert!(!open.is_submitted());
            assert!(open.error().unwrap().contains("more than once"));
        });
        assert!(answered.borrow().is_empty(), "nothing was answered");

        // The user gives up on the partial write: rejecting everything answers Deny.
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Rejected, cx).unwrap();
            review.submit(id, cx).unwrap();
        });
        cx.run_until_parked();
        assert_eq!(*answered.borrow(), [PermissionVerdict::Deny]);
        assert!(review.read_with(cx, |review, _| review.reviews().is_empty()));
    }

    #[gpui::test]
    async fn host_write_leaves_a_dirty_buffer_unsaved(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({ "a.txt": "one\ntwo\nthree\n" }),
        )
        .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let action_log = cx.new(|_| ActionLog::new(project.clone()));
        let path = project
            .read_with(cx, |project, cx| {
                project.find_project_path(util::path!("/p/a.txt"), cx)
            })
            .unwrap();
        let buffer = project
            .update(cx, |project, cx| project.open_buffer(path, cx))
            .await
            .unwrap();
        // An unsaved edit of the user's, away from the agent's change.
        buffer.update(cx, |buffer, cx| {
            buffer.edit([(0..0, "// mine\n")], None, cx)
        });
        let mut edit = ProposedEdit::new(
            PathBuf::from(util::path!("/p/a.txt")),
            "two\n".into(),
            "TWO\n".into(),
        );
        edit.decide_all(HunkDecision::Accepted);
        let mut async_cx = cx.to_async();
        write_accepted_hunks(project, action_log, &edit, &mut async_cx)
            .await
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            buffer.read_with(cx, |buffer, _| buffer.text()),
            "// mine\none\nTWO\nthree\n"
        );
        assert!(buffer.read_with(cx, |buffer, _| buffer.is_dirty()));
        let on_disk = fs.load(Path::new(util::path!("/p/a.txt"))).await.unwrap();
        assert_eq!(
            on_disk, "one\ntwo\nthree\n",
            "nothing saved behind the user's back"
        );
    }

    #[gpui::test]
    async fn held_writes_end_with_the_turn_or_the_last_pane(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(util::path!("/p"), serde_json::json!({ "a.txt": "a\n" }))
            .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let thread = cx
            .update(|cx| {
                acp_thread::AgentConnection::new_session(
                    connection.clone(),
                    project.clone(),
                    workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());
        let mut write = agent_write("/p/a.txt", cx);
        write.session_id = session_id;
        let held = gate_write(&review, write, cx);
        assert_eq!(
            review.read_with(cx, |review, _| review.open_review_count()),
            1
        );

        // The connection drops: the agent stopped waiting, so the write fails unchanged.
        thread.update(cx, |_, cx| cx.emit(AcpThreadEvent::ConnectionLost));
        cx.run_until_parked();
        let Err(error) = held.await else {
            panic!("the held write should fail");
        };
        assert!(format!("{error:#}").contains("unchanged"), "{error:#}");
        assert_eq!(
            review.read_with(cx, |review, _| review.open_review_count()),
            0
        );

        // Closing the last pane of a turn closes its open reviews; another pane keeps them.
        let held = gate_write(&review, agent_write("/p/b.txt", cx), cx);
        let turn = review.read_with(cx, |review, _| review.reviews()[0].turn());
        review.update(cx, |review, cx| {
            review.pane_opened(turn);
            review.pane_opened(turn);
            review.pane_closed(turn, cx);
            assert_eq!(review.open_review_count(), 1);
            review.pane_closed(turn, cx);
            assert_eq!(review.open_review_count(), 0);
        });
        assert!(held.await.is_err());
    }

    fn v2_edit_permission(
        session_id: &acp_v2::SessionId,
        changes: serde_json::Value,
        patch: &str,
    ) -> acp_v2::RequestPermissionRequest {
        serde_json::from_value(serde_json::json!({
            "sessionId": session_id,
            "title": "Edit a.txt",
            "options": [
                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                {"optionId": "reject", "name": "Reject", "kind": "reject_once"}
            ],
            "subject": {
                "type": "tool_call",
                "toolCall": {
                    "toolCallId": "edit-1",
                    "title": "Edit a.txt",
                    "kind": "edit",
                    "content": [{
                        "type": "diff",
                        "changes": changes,
                        "patch": {"format": "git_patch", "text": patch}
                    }]
                }
            }
        }))
        .unwrap()
    }

    #[gpui::test]
    async fn v2_patch_edits_are_previewed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(util::path!("/p"), serde_json::json!({ "a.txt": OLD }))
            .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let thread = cx
            .update(|cx| {
                acp_thread::AgentConnection::new_session(
                    connection.clone(),
                    project.clone(),
                    workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());
        let patch = "--- a/a.txt\n+++ b/a.txt\n@@ -1,5 +1,5 @@\n-one\n+ONE\n two\n three\n four\n-five\n+FIVE\n";
        let request = v2_edit_permission(
            &session_id,
            serde_json::json!([{"operation": "modify", "path": util::path!("/p/a.txt")}]),
            patch,
        );
        let (_, answer) = thread
            .update(cx, |thread, cx| thread.request_permission(request, cx))
            .unwrap();
        cx.run_until_parked();

        let id = review.read_with(cx, |review, _| {
            let preview = review.reviews().first().expect("a preview of the v2 patch");
            assert_eq!(preview.edit().hunks().len(), 2);
            assert_eq!(preview.edit().old_text(), OLD, "against the file as it is");
            preview.id()
        });
        review.update(cx, |review, cx| {
            review
                .set_decision(id, 0, HunkDecision::Accepted, cx)
                .unwrap();
            review
                .set_decision(id, 1, HunkDecision::Rejected, cx)
                .unwrap();
            review.submit(id, cx).unwrap();
        });
        cx.run_until_parked();
        let on_disk = fs.load(Path::new(util::path!("/p/a.txt"))).await.unwrap();
        assert_eq!(
            on_disk, "ONE\ntwo\nthree\nfour\nfive\n",
            "the host wrote hunk 1 only"
        );
        match answer.await {
            acp_v2::RequestPermissionOutcome::Selected(selected) => {
                assert_eq!(
                    selected.option_id,
                    acp_v2::PermissionOptionId::new("reject")
                )
            }
            other => panic!("expected the reject option, got {other:?}"),
        }

        // A multi-file edit is not previewed; the user is told instead.
        let request = v2_edit_permission(
            &session_id,
            serde_json::json!([
                {"operation": "modify", "path": util::path!("/p/a.txt")},
                {"operation": "modify", "path": util::path!("/p/b.txt")}
            ]),
            patch,
        );
        let (_, _unanswered) = thread
            .update(cx, |thread, cx| thread.request_permission(request, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(review.read_with(cx, |review, _| review.reviews().is_empty()));
    }

    #[gpui::test]
    async fn a_file_changed_during_review_is_not_written(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(util::path!("/p"), serde_json::json!({ "a.txt": OLD }))
            .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let thread = cx
            .update(|cx| {
                acp_thread::AgentConnection::new_session(
                    connection.clone(),
                    project.clone(),
                    workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                    cx,
                )
            })
            .await
            .unwrap();
        let path = PathBuf::from(util::path!("/p/a.txt"));
        let accept_held = |cx: &mut TestAppContext| {
            let id = review.read_with(cx, |review, _| review.reviews()[0].id());
            review.update(cx, |review, cx| {
                review.decide_all(id, HunkDecision::Accepted, cx).unwrap();
                review.submit(id, cx).unwrap();
            });
        };

        // The user edits the file while the agent's write waits for review.
        let write = thread.update(cx, |thread, cx| {
            thread.write_text_file(path.clone(), NEW.into(), cx)
        });
        cx.run_until_parked();
        let project_path = project
            .read_with(cx, |project, cx| project.find_project_path(&path, cx))
            .unwrap();
        let buffer = project
            .update(cx, |project, cx| project.open_buffer(project_path, cx))
            .await
            .unwrap();
        buffer.update(cx, |buffer, cx| {
            buffer.edit([(0..0, "// user\n")], None, cx)
        });
        accept_held(cx);
        let error = write.await.expect_err("a stale write must not land");
        assert!(format!("{error:#}").contains("changed while your edit was under review"));
        assert_eq!(fs.load(&path).await.unwrap(), OLD, "nothing written");

        // Without a concurrent change, the accepted write lands.
        buffer.update(cx, |buffer, cx| {
            buffer.edit([(0.."// user\n".len(), "")], None, cx)
        });
        let write = thread.update(cx, |thread, cx| {
            thread.write_text_file(path.clone(), NEW.into(), cx)
        });
        cx.run_until_parked();
        accept_held(cx);
        write.await.unwrap();
        assert_eq!(fs.load(&path).await.unwrap(), NEW);
    }

    #[gpui::test]
    async fn a_reloaded_session_keeps_its_reviews(cx: &mut TestAppContext) {
        let (review, _) = setup(cx);
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let old_thread = EntityId::from(1u64);
        let new_thread = EntityId::from(2u64);
        review.update(cx, |review, _| {
            review.thread_opened(session(), old_thread);
            review.thread_opened(session(), new_thread);
        });
        let held = gate_write(&review, agent_write("/p/a.txt", cx), cx);
        let closed = review.update(cx, |review, _| {
            review.thread_released(&session(), old_thread)
        });
        assert!(!closed, "the new thread still holds the session");
        assert_eq!(
            review.read_with(cx, |review, _| review.open_review_count()),
            1
        );
        let closed = review.update(cx, |review, _| {
            review.thread_released(&session(), new_thread)
        });
        assert!(closed);
        review.update(cx, |review, cx| review.close_session(&session(), cx));
        assert!(held.await.is_err());
    }

    #[gpui::test]
    async fn the_permission_card_defers_to_an_open_preview(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let (respond, _) = recording_responder();
        let request_id = new_request_id();
        let preview = ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into());
        let id = review
            .update(cx, |review, cx| {
                review.open_permission_preview(session(), request_id, preview, respond, None, cx)
            })
            .unwrap();
        assert!(cx.update(|cx| permission_card_notice(request_id, cx).is_some()));
        assert!(cx.update(|cx| permission_card_notice(new_request_id(), cx).is_none()));
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Rejected, cx).unwrap();
            review.submit(id, cx).unwrap();
        });
        cx.run_until_parked();
        assert!(cx.update(|cx| permission_card_notice(request_id, cx).is_none()));
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

    #[gpui::test]
    async fn only_user_settings_turn_review_off(cx: &mut TestAppContext) {
        if review_mode_override().is_some() {
            return;
        }
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.agent_review.get_or_insert_default().enabled = Some(true);
                });
                // A project's `.zed/settings.json` tries to turn review off.
                store
                    .set_local_settings(
                        settings::WorktreeId::from_usize(1),
                        settings::LocalSettingsPath::InWorktree(
                            util::rel_path::rel_path("").into(),
                        ),
                        settings::LocalSettingsKind::Settings,
                        Some(r#"{ "agent_review": { "enabled": false } }"#),
                        cx,
                    )
                    .unwrap();
            });
        });
        cx.run_until_parked();
        assert!(
            cx.update(|cx| review_mode(cx)),
            "a project cannot turn review off"
        );
    }

    #[gpui::test]
    async fn changing_the_setting_keeps_held_writes(cx: &mut TestAppContext) {
        if review_mode_override().is_some() {
            return;
        }
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let set_enabled = |enabled: bool, cx: &mut TestAppContext| {
            cx.update(|cx| {
                SettingsStore::update_global(cx, |store, cx| {
                    store.update_user_settings(cx, |settings| {
                        settings.agent_review.get_or_insert_default().enabled = Some(enabled);
                    });
                });
            });
            cx.run_until_parked();
        };
        set_enabled(true, cx);
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        let held = gate_write(&review, agent_write("/p/a.txt", cx), cx);
        set_enabled(false, cx);
        assert!(!review.read_with(cx, |review, _| review.is_enabled()));
        assert_eq!(
            review.read_with(cx, |review, _| review.open_review_count()),
            1,
            "turning review off does not release a held write"
        );
        let id = review.read_with(cx, |review, _| review.reviews()[0].id());
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Rejected, cx).unwrap();
            review.submit(id, cx).unwrap();
        });
        assert_eq!(held.await.unwrap().content, None);
    }

    #[test]
    fn vscode_import_never_sets_agent_review() {
        let vscode = settings::VsCodeSettings::from_str(
            r#"{
                "chat.editing.autoAcceptDelay": 1,
                "chat.tools.autoApprove": true,
                "github.copilot.chat.agent.autoFix": true,
                "files.autoSave": "afterDelay"
            }"#,
            settings::VsCodeSettingsSource::VsCode,
        )
        .unwrap();
        assert_eq!(vscode.settings_content().agent_review, None);
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
