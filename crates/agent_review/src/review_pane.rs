//! The review UI: one workspace item per open review, showing the proposed edit in Zed's diff editor
//! (an `acp_thread::Diff` multibuffer, as the agent panel renders it) with per-hunk Accept / Reject.
//!
//! While a review pane has focus the host owns the keyboard: a keystroke interceptor turns the review
//! keys into review actions before the focused editor's own bindings see them. The interceptor survives
//! keymap reloads, which clear bindings registered with `bind_keys`.

use std::{ops::Range, path::Path, sync::Arc};

use anyhow::Context as _;

use acp_thread::Diff;
use buffer_diff::DiffHunkStatus;
use editor::{
    Anchor, DiffHunkRenderer, Editor, EditorEvent, EditorMode, MultiBufferSnapshot, SizingBehavior,
    ToPoint as _, actions::GoToHunk, actions::GoToPreviousHunk,
};
use gpui::{
    Action, AnyElement, App, Entity, EventEmitter, FocusHandle, Focusable, Keystroke, Pixels,
    SharedString, Subscription, WeakEntity, Window, actions, prelude::*,
};
use project::Project;
use ui::{Button, ButtonStyle, Label, LabelSize, Tooltip, prelude::*};
use workspace::{
    HideStatusItem, Item, ItemHandle, OpenOptions, StatusItemView, Workspace,
    item::{ItemEvent, TabContentParams},
};

use crate::{
    AgentReview, AgentReviewEvent, HunkDecision, ProposedEdit, Resolution, ReviewId, TurnId,
};

actions!(
    agent_review,
    [
        /// Accepts the agent's change in the hunk at the cursor.
        AcceptHunk,
        /// Rejects the agent's change in the hunk at the cursor.
        RejectHunk,
        /// Accepts every hunk of the review.
        AcceptAllHunks,
        /// Rejects every hunk of the review.
        RejectAllHunks,
        /// Accepts every edit of the agent's turn, including edits the turn proposes later.
        AcceptTurn,
        /// Rejects every edit of the agent's turn, including edits the turn proposes later.
        RejectTurn,
        /// Moves to the next hunk.
        NextHunk,
        /// Moves to the previous hunk.
        PreviousHunk,
        /// Focuses the comment field of the hunk at the cursor.
        CommentHunk,
        /// Returns from the comment field to the diff.
        FocusDiff,
        /// Sends the decisions; every hunk must be decided.
        SubmitReview,
        /// Opens the review panes of every open review in this workspace.
        OpenReviews,
        /// Turns review of agent edits on or off.
        ToggleReviewMode,
    ]
);

const KEY_CONTEXT: &str = "AgentReview";

/// Keys the host routes while a review pane has focus.
fn review_keys() -> Vec<(Keystroke, Box<dyn Action>)> {
    [
        ("alt-y", AcceptHunk.boxed_clone()),
        ("alt-n", RejectHunk.boxed_clone()),
        ("shift-alt-y", AcceptAllHunks.boxed_clone()),
        ("shift-alt-n", RejectAllHunks.boxed_clone()),
        ("ctrl-shift-y", AcceptTurn.boxed_clone()),
        ("ctrl-shift-n", RejectTurn.boxed_clone()),
        ("alt-j", NextHunk.boxed_clone()),
        ("alt-k", PreviousHunk.boxed_clone()),
        ("alt-c", CommentHunk.boxed_clone()),
        ("escape", FocusDiff.boxed_clone()),
        ("ctrl-enter", SubmitReview.boxed_clone()),
    ]
    .into_iter()
    .filter_map(|(key, action)| Some((Keystroke::parse(key).ok()?, action)))
    .collect()
}

fn key_label(action: &dyn Action) -> &'static str {
    let name = action.name();
    match name {
        _ if name == AcceptHunk.name() => "alt-y",
        _ if name == RejectHunk.name() => "alt-n",
        _ if name == AcceptAllHunks.name() => "shift-alt-y",
        _ if name == RejectAllHunks.name() => "shift-alt-n",
        _ if name == AcceptTurn.name() => "ctrl-shift-y",
        _ if name == RejectTurn.name() => "ctrl-shift-n",
        _ if name == CommentHunk.name() => "alt-c",
        _ if name == SubmitReview.name() => "ctrl-enter",
        _ => "",
    }
}

pub(crate) fn init(cx: &mut App) {
    let keys = review_keys();
    cx.intercept_keystrokes(move |event, window, cx| {
        let in_review = event
            .context_stack
            .iter()
            .any(|context| context.contains(KEY_CONTEXT));
        if !in_review {
            return;
        }
        let pressed = &event.keystroke;
        let Some((_, action)) = keys.iter().find(|(keystroke, _)| {
            keystroke.modifiers == pressed.modifiers && keystroke.key == pressed.key
        }) else {
            return;
        };
        window.dispatch_action(action.boxed_clone(), cx);
        cx.stop_propagation();
    })
    .detach();

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        workspace.register_action(|workspace, _: &OpenReviews, window, cx| {
            open_reviews(workspace, None, window, cx);
        });
        workspace.register_action(|_, _: &ToggleReviewMode, _, cx| {
            crate::set_review_mode_setting(!crate::review_mode(cx), cx);
        });
        let Some(review) = AgentReview::global(cx) else {
            return;
        };
        let status = cx.new(|cx| ReviewStatus::new(review.clone(), cx));
        workspace.status_bar().update(cx, |status_bar, cx| {
            status_bar.add_right_item(status, window, cx)
        });
        cx.subscribe_in(
            &review,
            window,
            |workspace, _, event, window, cx| match event {
                AgentReviewEvent::Opened(id) => open_reviews(workspace, Some(*id), window, cx),
                AgentReviewEvent::UnreviewedChanges(turn) => {
                    show_unreviewed_changes(workspace, *turn, window, cx)
                }
                _ => {}
            },
        )
        .detach();
    })
    .detach();
}

/// Opens (or brings forward) the turn pane of each open review (or only `only`) whose file belongs to
/// this workspace's project.
fn open_reviews(
    workspace: &mut Workspace,
    only: Option<ReviewId>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(review) = AgentReview::global(cx) else {
        return;
    };
    let project = workspace.project().clone();
    let mut turns = review
        .read(cx)
        .reviews()
        .iter()
        .filter(|candidate| only.is_none_or(|id| candidate.id() == id))
        .filter(|candidate| !candidate.is_submitted())
        .filter(|candidate| {
            project
                .read(cx)
                .project_path_for_absolute_path(candidate.edit().abs_path(), cx)
                .is_some()
        })
        .map(|candidate| candidate.turn())
        .collect::<Vec<_>>();
    turns.dedup();
    for turn in turns {
        let existing = workspace
            .items_of_type::<ReviewPane>(cx)
            .find(|pane| pane.read(cx).turn == turn);
        let pane = match existing {
            Some(pane) => {
                let focus = window.is_window_active();
                workspace.activate_item(&pane, true, focus, window, cx);
                pane
            }
            None => {
                let workspace_handle = cx.weak_entity();
                let pane = cx.new(|cx| {
                    ReviewPane::new(review.clone(), turn, project.clone(), workspace_handle, cx)
                });
                let focus = window.is_window_active();
                workspace.add_item_to_active_pane(Box::new(pane.clone()), None, focus, window, cx);
                pane
            }
        };
        pane.update(cx, |pane, cx| pane.show_next(window, cx));
    }
}

/// Opens the turn's pane, without focusing it, when files of this workspace's project changed during
/// the turn without review.
fn show_unreviewed_changes(
    workspace: &mut Workspace,
    turn: TurnId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(review) = AgentReview::global(cx) else {
        return;
    };
    let project = workspace.project().clone();
    let in_project = review.read(cx).unreviewed_changes(turn).any(|path| {
        project
            .read(cx)
            .project_path_for_absolute_path(path, cx)
            .is_some()
    });
    let has_pane = workspace
        .items_of_type::<ReviewPane>(cx)
        .any(|pane| pane.read(cx).turn == turn);
    if in_project && !has_pane {
        let workspace_handle = cx.weak_entity();
        let pane = cx.new(|cx| ReviewPane::new(review, turn, project, workspace_handle, cx));
        workspace.add_item_to_active_pane(Box::new(pane), None, false, window, cx);
    }
}

pub enum ReviewPaneEvent {
    Close,
}

/// The file of the turn currently shown in the diff editor.
struct CurrentReview {
    id: ReviewId,
    _diff: Entity<Diff>,
    editor: Entity<Editor>,
    _selections: Subscription,
    /// The diff fills in asynchronously; until the cursor first lands in a hunk, every change of
    /// the diff moves it to the first hunk.
    _diff_loaded: Subscription,
    reached_first_hunk: bool,
}

/// One pane per agent turn: every file the agent proposed in the turn, one diff at a time.
pub struct ReviewPane {
    review: Entity<AgentReview>,
    turn: TurnId,
    project: Entity<Project>,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    current: Option<CurrentReview>,
    comment_editor: Option<Entity<Editor>>,
    current_hunk: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ReviewPane {
    fn new(
        review: Entity<AgentReview>,
        turn: TurnId,
        project: Entity<Project>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        review.update(cx, |review, _| review.pane_opened(turn));
        cx.on_release(move |this, cx| {
            this.review
                .update(cx, |review, cx| review.pane_closed(turn, cx));
        })
        .detach();
        Self {
            review,
            turn,
            project,
            workspace,
            focus_handle: cx.focus_handle(),
            current: None,
            comment_editor: None,
            current_hunk: None,
            _subscriptions: Vec::new(),
        }
    }

    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self._subscriptions.is_empty() {
            return;
        }
        let turn = self.turn;
        let comment_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Comment for the agent on this hunk (alt-c)", window, cx);
            editor
        });
        self._subscriptions = vec![
            cx.subscribe_in(
                &self.review,
                window,
                move |this, review, event, window, cx| {
                    let current = this.current_review_id();
                    match event {
                        AgentReviewEvent::Opened(id)
                            if current.is_none()
                                && review
                                    .read(cx)
                                    .review(*id)
                                    .is_some_and(|r| r.turn() == turn) =>
                        {
                            this.show_next(window, cx);
                        }
                        AgentReviewEvent::Opened(_) => cx.notify(),
                        AgentReviewEvent::Updated(id) | AgentReviewEvent::Resolved(id)
                            if Some(*id) == current =>
                        {
                            if let Some(current) = &this.current {
                                current.editor.update(cx, |_, cx| cx.notify());
                            }
                            cx.notify();
                        }
                        AgentReviewEvent::Closed(id) if Some(*id) == current => {
                            this.current = None;
                            this.show_next(window, cx);
                        }
                        AgentReviewEvent::TurnEnded(ended) if *ended == turn => {
                            // A turn with unreviewed changes stays on screen so the user sees them.
                            let unreviewed =
                                review.read(cx).unreviewed_changes(turn).next().is_some();
                            if this.current.is_none() && !unreviewed {
                                cx.emit(ReviewPaneEvent::Close);
                            }
                        }
                        _ => cx.notify(),
                    }
                },
            ),
            cx.subscribe(&comment_editor, |this, editor, event, cx| {
                if let EditorEvent::BufferEdited = event {
                    let text = editor.read(cx).text(cx);
                    this.update_review(cx, |review, id, hunk, cx| {
                        review.set_comment(id, hunk, Some(text), cx)
                    });
                }
            }),
        ];
        self.comment_editor = Some(comment_editor);
    }

    /// Shows the first open review of the turn unless one is shown already, and moves to its first
    /// hunk.
    fn show_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscribe(window, cx);
        let still_open = self.current.as_ref().is_some_and(|current| {
            self.review
                .read(cx)
                .review(current.id)
                .is_some_and(|review| !review.is_submitted())
        });
        if still_open {
            return;
        }
        let next = self
            .review
            .read(cx)
            .open_reviews_in_turn(self.turn)
            .next()
            .map(|review| review.id());
        match next {
            Some(id) => self.show_review(id, window, cx),
            None => {
                self.current = None;
                self.current_hunk = None;
                cx.notify();
            }
        }
    }

    /// Shows review `id` of this turn in the diff editor.
    pub fn show_review(&mut self, id: ReviewId, window: &mut Window, cx: &mut Context<Self>) {
        self.subscribe(window, cx);
        let Some(edit) = self
            .review
            .read(cx)
            .review(id)
            .map(|review| review.edit().clone())
        else {
            return;
        };
        let language_registry = self.project.read(cx).languages().clone();
        let diff = cx.new(|_| {
            Diff::finalized(
                edit.abs_path().to_string_lossy().into_owned(),
                Some(edit.old_text().to_string()),
                edit.new_text().to_string(),
                language_registry,
            )
        });
        let multibuffer = diff.update(cx, |diff, cx| diff.multibuffer(cx));
        let diff_loaded = cx.observe_in(&multibuffer, window, |this, _, window, cx| {
            let Some(current) = &this.current else {
                return;
            };
            if current.reached_first_hunk {
                return;
            }
            let focus = current.editor.focus_handle(cx);
            focus.dispatch_action(&GoToHunk, window, cx);
            this.update_current_hunk(window, cx);
        });
        let renderer: Arc<dyn DiffHunkRenderer> = Arc::new(ReviewHunkRenderer {
            review: self.review.downgrade(),
            pane: cx.weak_entity(),
            review_id: id,
        });
        let project = self.project.clone();
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(
                EditorMode::Full {
                    scale_ui_elements_with_buffer_font_size: true,
                    show_active_line_background: true,
                    sizing_behavior: SizingBehavior::Default,
                },
                multibuffer,
                Some(project),
                window,
                cx,
            );
            editor.set_read_only(true);
            editor.disable_diagnostics(cx);
            editor.set_expand_all_diff_hunks(cx);
            editor.set_diff_hunk_renderer(Some(renderer), cx);
            editor
        });
        let selections = cx.subscribe_in(&editor, window, |this, _, event, window, cx| {
            if let EditorEvent::SelectionsChanged { .. } = event {
                this.update_current_hunk(window, cx);
            }
        });
        self.current = Some(CurrentReview {
            id,
            _diff: diff,
            editor: editor.clone(),
            _selections: selections,
            _diff_loaded: diff_loaded,
            reached_first_hunk: false,
        });
        self.current_hunk = None;
        let focus = editor.focus_handle(cx);
        // A review that opens in a background window must not take the keyboard from the window the
        // user is typing in.
        if window.is_window_active() {
            window.focus(&focus, cx);
        }
        focus.dispatch_action(&GoToHunk, window, cx);
        self.update_current_hunk(window, cx);
        cx.notify();
    }

    pub fn turn(&self) -> TurnId {
        self.turn
    }

    /// The review shown in the diff editor, if any is still waiting in this turn.
    pub fn current_review_id(&self) -> Option<ReviewId> {
        self.current.as_ref().map(|current| current.id)
    }

    pub fn diff_focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.current {
            Some(current) => current.editor.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }

    /// Makes `hunk` the target of the hunk actions, as moving the cursor into it would.
    pub fn select_hunk(&mut self, hunk: usize, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let hunk_count = self.edit(cx).map_or(0, |edit| edit.hunks().len());
        anyhow::ensure!(hunk < hunk_count, "hunk {} of {hunk_count}", hunk + 1);
        self.current_hunk = Some(hunk);
        cx.notify();
        Ok(())
    }

    /// Makes `hunk` the target of the hunk actions and shows its comment.
    fn focus_hunk(&mut self, hunk: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.select_hunk(hunk, cx).is_err() {
            return;
        }
        let comment = self
            .edit(cx)
            .and_then(|edit| edit.hunks().get(hunk)?.comment.clone())
            .unwrap_or_default();
        if let Some(comment_editor) = self.comment_editor.clone() {
            comment_editor.update(cx, |editor, cx| editor.set_text(comment, window, cx));
        }
    }

    /// Types `text` into the comment field of `hunk`.
    pub fn set_comment_text(
        &mut self,
        hunk: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.select_hunk(hunk, cx)?;
        let comment_editor = self
            .comment_editor
            .clone()
            .context("the pane has not been shown yet")?;
        comment_editor.update(cx, |editor, cx| editor.set_text(text, window, cx));
        Ok(())
    }

    pub fn edit<'a>(&self, cx: &'a App) -> Option<&'a ProposedEdit> {
        Some(
            self.review
                .read(cx)
                .review(self.current_review_id()?)?
                .edit(),
        )
    }

    fn update_current_hunk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.current.as_ref().map(|current| current.editor.clone()) else {
            return;
        };
        let row = editor.update(cx, |editor, cx| {
            let snapshot = editor.buffer().read(cx).snapshot(cx);
            let head = editor.selections.newest_anchor().head();
            buffer_row(&snapshot, head)
        });
        let hunk = row.and_then(|row| self.edit(cx)?.hunk_at_new_row(row));
        if hunk.is_some()
            && let Some(current) = &mut self.current
        {
            current.reached_first_hunk = true;
        }
        if hunk == self.current_hunk {
            return;
        }
        self.current_hunk = hunk;
        let comment = hunk
            .and_then(|hunk| self.edit(cx)?.hunks().get(hunk)?.comment.clone())
            .unwrap_or_default();
        if let Some(comment_editor) = self.comment_editor.clone() {
            comment_editor.update(cx, |editor, cx| editor.set_text(comment, window, cx));
        }
        cx.notify();
    }

    fn update_review(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(
            &mut AgentReview,
            ReviewId,
            usize,
            &mut Context<AgentReview>,
        ) -> anyhow::Result<()>,
    ) {
        let (Some(id), Some(hunk)) = (self.current_review_id(), self.current_hunk) else {
            return;
        };
        if let Err(error) = self
            .review
            .update(cx, |review, cx| update(review, id, hunk, cx))
        {
            log::debug!("review update failed: {error:#}");
        }
    }

    fn decide_current(
        &mut self,
        decision: HunkDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_review(cx, |review, id, hunk, cx| {
            review.set_decision(id, hunk, decision, cx)
        });
        let undecided_left = self
            .edit(cx)
            .is_some_and(|edit| edit.resolution().is_pending());
        if undecided_left {
            self.diff_focus_handle(cx)
                .dispatch_action(&GoToHunk, window, cx);
        }
    }

    fn accept_hunk(&mut self, _: &AcceptHunk, window: &mut Window, cx: &mut Context<Self>) {
        self.decide_current(HunkDecision::Accepted, window, cx);
    }

    fn reject_hunk(&mut self, _: &RejectHunk, window: &mut Window, cx: &mut Context<Self>) {
        self.decide_current(HunkDecision::Rejected, window, cx);
    }

    fn decide_all(&mut self, decision: HunkDecision, cx: &mut Context<Self>) {
        let Some(id) = self.current_review_id() else {
            return;
        };
        if let Err(error) = self
            .review
            .update(cx, |review, cx| review.decide_all(id, decision, cx))
        {
            log::debug!("review update failed: {error:#}");
        }
    }

    fn accept_all(&mut self, _: &AcceptAllHunks, _: &mut Window, cx: &mut Context<Self>) {
        self.decide_all(HunkDecision::Accepted, cx);
    }

    fn reject_all(&mut self, _: &RejectAllHunks, _: &mut Window, cx: &mut Context<Self>) {
        self.decide_all(HunkDecision::Rejected, cx);
    }

    fn decide_turn(&mut self, decision: HunkDecision, cx: &mut Context<Self>) {
        let turn = self.turn;
        if let Err(error) = self
            .review
            .update(cx, |review, cx| review.decide_turn(turn, decision, cx))
        {
            log::info!("fyx: turn not decided: {error:#}");
        }
    }

    fn accept_turn(&mut self, _: &AcceptTurn, _: &mut Window, cx: &mut Context<Self>) {
        self.decide_turn(HunkDecision::Accepted, cx);
    }

    fn reject_turn(&mut self, _: &RejectTurn, _: &mut Window, cx: &mut Context<Self>) {
        self.decide_turn(HunkDecision::Rejected, cx);
    }

    fn next_hunk(&mut self, _: &NextHunk, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.diff_focus_handle(cx);
        window.focus(&focus, cx);
        focus.dispatch_action(&GoToHunk, window, cx);
    }

    fn previous_hunk(&mut self, _: &PreviousHunk, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.diff_focus_handle(cx);
        window.focus(&focus, cx);
        focus.dispatch_action(&GoToPreviousHunk, window, cx);
    }

    fn comment_hunk(&mut self, _: &CommentHunk, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_hunk.is_some()
            && let Some(comment_editor) = &self.comment_editor
        {
            window.focus(&comment_editor.focus_handle(cx), cx);
        }
    }

    fn focus_diff(&mut self, _: &FocusDiff, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.diff_focus_handle(cx), cx);
    }

    fn submit(&mut self, _: &SubmitReview, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current_review_id() else {
            return;
        };
        if let Err(error) = self.review.update(cx, |review, cx| review.submit(id, cx)) {
            log::info!("fyx: review not submitted: {error:#}");
        }
    }

    fn render_turn(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let review = self.review.read(cx);
        let current = self.current_review_id();
        let decided = review.turn_history(self.turn).to_vec();
        let open = review
            .open_reviews_in_turn(self.turn)
            .map(|review| (review.id(), review.edit().abs_path().to_path_buf()))
            .collect::<Vec<_>>();
        let turn_decision = review.turn_decision(self.turn);
        let summary = format!(
            "{} decided, {} waiting{}",
            decided.len(),
            open.len(),
            match turn_decision {
                Some(HunkDecision::Accepted) => " · accepting the rest of the turn",
                Some(HunkDecision::Rejected) => " · rejecting the rest of the turn",
                _ => "",
            }
        );

        let mut files = v_flex().gap_0p5();
        for entry in &decided {
            files = files.child(
                h_flex()
                    .gap_2()
                    .child(Label::new(file_name(&entry.path)).size(LabelSize::Small))
                    .child(
                        Label::new(resolution_label(entry.resolution))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .children(entry.rule.as_ref().map(|rule| {
                        Label::new(format!("by rule {}", rule.describe()))
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                    })),
            );
        }
        for (id, path) in open {
            let is_current = Some(id) == current;
            files = files.child(
                Button::new(("review-file", id.0), file_name(&path))
                    .label_size(LabelSize::Small)
                    .toggle_state(is_current)
                    .style(if is_current {
                        ButtonStyle::Filled
                    } else {
                        ButtonStyle::Subtle
                    })
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.show_review(id, window, cx)),
                    ),
            );
        }

        let unreviewed = review
            .unreviewed_changes(self.turn)
            .cloned()
            .collect::<Vec<_>>();
        let unreviewed_section = (!unreviewed.is_empty()).then(|| {
            let mut section = v_flex().gap_0p5().child(
                Label::new(format!(
                    "{} {} changed without review",
                    unreviewed.len(),
                    if unreviewed.len() == 1 {
                        "file"
                    } else {
                        "files"
                    }
                ))
                .size(LabelSize::Small)
                .color(Color::Warning),
            );
            for (index, path) in unreviewed.into_iter().enumerate() {
                let workspace = self.workspace.clone();
                section = section.child(
                    Button::new(("unreviewed-file", index), path.display().to_string())
                        .label_size(LabelSize::Small)
                        .style(ButtonStyle::Subtle)
                        .tooltip(Tooltip::text(
                            "Open the file; its git gutter shows the change",
                        ))
                        .on_click(move |_, window, cx| {
                            let path = path.clone();
                            if let Err(error) = workspace.update(cx, |workspace, cx| {
                                workspace
                                    .open_abs_path(path, OpenOptions::default(), window, cx)
                                    .detach_and_log_err(cx);
                            }) {
                                log::debug!("workspace closed: {error}");
                            }
                        }),
                );
            }
            section
        });

        v_flex()
            .p_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(Label::new("Agent turn review"))
                    .child(
                        Label::new(summary)
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1())
                    .child(self.action_button("accept-turn", "Accept all in turn", &AcceptTurn, cx))
                    .child(self.action_button(
                        "reject-turn",
                        "Reject all in turn",
                        &RejectTurn,
                        cx,
                    )),
            )
            .child(files)
            .children(unreviewed_section)
            .child(
                Label::new(crate::NOT_REVIEWED)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
    }

    fn render_file_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let edit = self.edit(cx);
        let hunk_count = edit.map_or(0, |edit| edit.hunks().len());
        let resolution = edit.map(ProposedEdit::resolution);
        let status = resolution.map_or("review closed", resolution_label);
        let current = match (self.current_hunk, edit) {
            (Some(hunk), Some(edit)) => {
                let decision = edit
                    .hunks()
                    .get(hunk)
                    .map_or(HunkDecision::Pending, |hunk| hunk.decision);
                format!(
                    "Hunk {} of {hunk_count}: {}",
                    hunk + 1,
                    decision_label(decision)
                )
            }
            _ => "Move the cursor into a hunk (alt-j / alt-k)".into(),
        };
        let path = edit.map_or_else(String::new, |edit| edit.abs_path().display().to_string());
        let rule = self.current_review_id().and_then(|id| {
            let review = self.review.read(cx).review(id)?;
            Some(format!("Asked by rule {}", review.rule()?.describe()))
        });
        let can_submit = resolution.is_some_and(|resolution| !resolution.is_pending());
        let error = self.current_review_id().and_then(|id| {
            let review = self.review.read(cx).review(id)?;
            Some(format!(
                "Could not apply: {}. Change the decisions and submit again, or reject the file.",
                review.error()?
            ))
        });

        v_flex()
            .p_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(Label::new(path).size(LabelSize::Small))
                    .children(
                        rule.map(|rule| {
                            Label::new(rule).size(LabelSize::Small).color(Color::Accent)
                        }),
                    )
                    .child(div().flex_1())
                    .child(Label::new(status).size(LabelSize::Small)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Label::new(current).size(LabelSize::Small))
                    .child(div().flex_1())
                    .child(self.action_button("accept-hunk", "Accept", &AcceptHunk, cx))
                    .child(self.action_button("reject-hunk", "Reject", &RejectHunk, cx))
                    .child(self.action_button("accept-all", "Accept file", &AcceptAllHunks, cx))
                    .child(self.action_button("reject-all", "Reject file", &RejectAllHunks, cx))
                    .child(
                        self.action_button("submit", "Submit", &SubmitReview, cx)
                            .style(ButtonStyle::Filled)
                            .disabled(!can_submit),
                    ),
            )
            .children(
                error.map(|error| Label::new(error).size(LabelSize::Small).color(Color::Error)),
            )
            .children(self.comment_editor.clone().map(|comment_editor| {
                h_flex()
                    .gap_2()
                    .child(
                        Label::new("Comment")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1().child(comment_editor))
            }))
    }

    fn action_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: &dyn Action,
        _cx: &mut Context<Self>,
    ) -> Button {
        let action = action.boxed_clone();
        let key = key_label(action.as_ref());
        Button::new(id, label)
            .tooltip(Tooltip::text(key))
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
    }
}

fn decision_label(decision: HunkDecision) -> &'static str {
    match decision {
        HunkDecision::Pending => "undecided",
        HunkDecision::Accepted => "accepted",
        HunkDecision::Rejected => "rejected",
    }
}

fn resolution_label(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::Pending { .. } => "undecided hunks",
        Resolution::AcceptedAll => "accepted",
        Resolution::RejectedAll => "rejected",
        Resolution::Partial { .. } => "partly accepted",
        Resolution::Unchanged => "no changes",
    }
}

fn file_name(path: &Path) -> SharedString {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
        .into()
}

/// The row in the proposed (new) text at a position of the diff multibuffer.
fn buffer_row(snapshot: &MultiBufferSnapshot, anchor: Anchor) -> Option<u32> {
    let point = anchor.to_point(snapshot);
    let (_, buffer_point) = snapshot.point_to_buffer_point(point)?;
    Some(buffer_point.row)
}

impl EventEmitter<ReviewPaneEvent> for ReviewPane {}

impl Focusable for ReviewPane {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.diff_focus_handle(cx)
    }
}

impl Render for ReviewPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.subscribe(window, cx);
        let body = match &self.current {
            Some(current) => v_flex()
                .flex_1()
                .min_h_0()
                .child(self.render_file_header(cx))
                .child(div().flex_1().min_h_0().child(current.editor.clone())),
            None => v_flex().flex_1().p_4().child(
                Label::new("No edits are waiting. New edits of this turn appear here.")
                    .color(Color::Muted),
            ),
        };
        v_flex()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .on_action(cx.listener(Self::accept_hunk))
            .on_action(cx.listener(Self::reject_hunk))
            .on_action(cx.listener(Self::accept_all))
            .on_action(cx.listener(Self::reject_all))
            .on_action(cx.listener(Self::accept_turn))
            .on_action(cx.listener(Self::reject_turn))
            .on_action(cx.listener(Self::next_hunk))
            .on_action(cx.listener(Self::previous_hunk))
            .on_action(cx.listener(Self::comment_hunk))
            .on_action(cx.listener(Self::focus_diff))
            .on_action(cx.listener(Self::submit))
            .child(self.render_turn(cx))
            .child(body)
    }
}

impl Item for ReviewPane {
    type Event = ReviewPaneEvent;

    fn tab_content_text(&self, _detail: usize, cx: &App) -> SharedString {
        match self.edit(cx) {
            Some(edit) => format!("Review: {}", file_name(edit.abs_path())).into(),
            None => "Review: agent turn".into(),
        }
    }

    fn tab_content(&self, params: TabContentParams, _window: &Window, cx: &App) -> AnyElement {
        Label::new(self.tab_content_text(params.detail.unwrap_or_default(), cx))
            .color(params.text_color())
            .into_any_element()
    }

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::ZedAgent).color(Color::Muted))
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        match event {
            ReviewPaneEvent::Close => f(ItemEvent::CloseItem),
        }
    }
}

/// Review mode and waiting reviews in the status bar. Shown while review mode is on or a review
/// waits (e.g. one asked by an `always_ask` rule); clicking it brings the waiting reviews forward.
pub struct ReviewStatus {
    review: Entity<AgentReview>,
    _observe: Subscription,
}

impl ReviewStatus {
    fn new(review: Entity<AgentReview>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&review, |_, _, cx| cx.notify());
        Self {
            review,
            _observe: observe,
        }
    }
}

impl Render for ReviewStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let review = self.review.read(cx);
        let waiting = review.open_review_count();
        let label = match (review.is_enabled(), waiting) {
            (false, 0) => return div().into_any_element(),
            (true, 0) => "Review on".to_string(),
            (_, 1) => "Review: 1 waiting".to_string(),
            (_, waiting) => format!("Review: {waiting} waiting"),
        };
        Button::new("agent-review-status", label)
            .label_size(LabelSize::Small)
            .style(ButtonStyle::Subtle)
            .tooltip(Tooltip::text(crate::COVERAGE))
            .on_click(|_, window, cx| window.dispatch_action(OpenReviews.boxed_clone(), cx))
            .into_any_element()
    }
}

impl StatusItemView for ReviewStatus {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// Shown only while the `agent_review.enabled` setting is on or a review waits.
    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}

struct ReviewHunkRenderer {
    review: WeakEntity<AgentReview>,
    pane: WeakEntity<ReviewPane>,
    review_id: ReviewId,
}

impl DiffHunkRenderer for ReviewHunkRenderer {
    fn render_hunk_controls(
        &self,
        row: u32,
        _status: &DiffHunkStatus,
        hunk_range: Range<Anchor>,
        _is_created_file: bool,
        line_height: Pixels,
        editor: &Entity<Editor>,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let snapshot = editor.read(cx).buffer().read(cx).snapshot(cx);
        let Some(review) = self.review.upgrade() else {
            return div().into_any_element();
        };
        let review_id = self.review_id;
        let pane = self.pane.clone();
        let hunk = buffer_row(&snapshot, hunk_range.start).and_then(|row| {
            let edit = review.read(cx).review(review_id)?.edit();
            let index = edit.hunk_drawn_at(row)?;
            Some((index, edit.hunks().get(index)?.decision))
        });
        let Some((index, decision)) = hunk else {
            return div().into_any_element();
        };
        let button = |id: &'static str, label: &'static str, choice: HunkDecision| {
            let review = review.clone();
            Button::new((id, row as u64), label)
                .style(if decision == choice {
                    ButtonStyle::Filled
                } else {
                    ButtonStyle::Subtle
                })
                .toggle_state(decision == choice)
                .on_click({
                    let pane = pane.clone();
                    move |_, window, cx| {
                        review.update(cx, |review, cx| {
                            if let Err(error) = review.set_decision(review_id, index, choice, cx) {
                                log::debug!("review update failed: {error:#}");
                            }
                        });
                        // A comment typed next belongs to the hunk just decided.
                        if let Err(error) =
                            pane.update(cx, |pane, cx| pane.focus_hunk(index, window, cx))
                        {
                            log::debug!("review pane closed: {error}");
                        }
                    }
                })
        };
        h_flex()
            .h(line_height)
            .gap_1()
            .px_1()
            .rounded_b_md()
            .border_x_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().editor_background)
            .block_mouse_except_scroll()
            .child(button("accept", "Accept", HunkDecision::Accepted))
            .child(button("reject", "Reject", HunkDecision::Rejected))
            .into_any_element()
    }

    fn render_hunk_as_staged(&self, _status: &DiffHunkStatus, _cx: &App) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use acp_thread::fyx_review::AgentWrite;
    use agent_client_protocol::v2::schema as acp_v2;
    use gpui::TestAppContext;
    use language::Buffer;
    use project::{FakeFs, Project};
    use settings::SettingsStore;
    use workspace::MultiWorkspace;

    use super::*;

    const OLD: &str = "one\ntwo\nthree\nfour\nfive\n";
    const NEW: &str = "ONE\ntwo\nthree\nfour\nFIVE\n";

    #[gpui::test]
    async fn one_pane_shows_every_file_of_the_turn(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({ "a.txt": OLD, "b.txt": OLD }),
        )
        .await;
        let project = Project::test(fs, [util::path!("/p").as_ref()], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());

        let mut held = Vec::new();
        for name in ["a.txt", "b.txt"] {
            let buffer = cx.new(|cx| Buffer::local(OLD, cx));
            let write = AgentWrite {
                session_id: acp_v2::SessionId::new("session"),
                abs_path: PathBuf::from(util::path!("/p")).join(name),
                buffer,
                old_text: OLD.into(),
                new_text: NEW.into(),
            };
            held.push(review.update(cx, |review, cx| review.review_write(write, cx)));
        }
        cx.run_until_parked();

        let panes = workspace.read_with(cx, |workspace, cx| {
            workspace
                .items_of_type::<ReviewPane>(cx)
                .collect::<Vec<_>>()
        });
        assert_eq!(panes.len(), 1, "one pane for the turn");
        let pane = panes[0].clone();
        let (first, second) = review.read_with(cx, |review, _| {
            (review.reviews()[0].id(), review.reviews()[1].id())
        });
        assert_eq!(
            pane.read_with(cx, |pane, _| pane.current_review_id()),
            Some(first)
        );

        cx.simulate_keystrokes("shift-alt-y");
        cx.simulate_keystrokes("ctrl-enter");
        cx.run_until_parked();
        assert_eq!(
            pane.read_with(cx, |pane, _| pane.current_review_id()),
            Some(second),
            "the pane moves on to the turn's next file"
        );

        cx.simulate_keystrokes("ctrl-shift-n");
        cx.run_until_parked();
        let mut outcomes = Vec::new();
        for task in held {
            outcomes.push(task.await.unwrap().content);
        }
        assert_eq!(outcomes, [Some(NEW.to_string()), None]);
        assert_eq!(pane.read_with(cx, |pane, _| pane.current_review_id()), None);
    }

    #[gpui::test]
    async fn review_keys_decide_hunks_and_submit(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(util::path!("/p"), serde_json::json!({ "a.txt": OLD }))
            .await;
        let project = Project::test(fs, [util::path!("/p").as_ref()], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());

        let buffer = cx.new(|cx| Buffer::local(OLD, cx));
        let write = AgentWrite {
            session_id: acp_v2::SessionId::new("session"),
            abs_path: PathBuf::from(util::path!("/p/a.txt")),
            buffer,
            old_text: OLD.into(),
            new_text: NEW.into(),
        };
        let held = review.update(cx, |review, cx| review.review_write(write, cx));
        cx.run_until_parked();

        let pane = workspace
            .read_with(cx, |workspace, cx| {
                workspace.items_of_type::<ReviewPane>(cx).next()
            })
            .expect("a review pane opens for the held write");
        pane.update_in(cx, |pane, window, cx| pane.show_next(window, cx));
        cx.run_until_parked();
        assert_eq!(pane.read_with(cx, |pane, _| pane.current_hunk), Some(0));

        // The editor binds none of these; the interceptor routes them to the review.
        cx.simulate_keystrokes("alt-y");
        cx.run_until_parked();
        assert_eq!(
            pane.read_with(cx, |pane, _| pane.current_hunk),
            Some(1),
            "accepting moves to the next undecided hunk"
        );
        cx.simulate_keystrokes("alt-n");
        cx.run_until_parked();
        review.read_with(cx, |review, _| {
            let decisions = review.reviews()[0]
                .edit()
                .hunks()
                .iter()
                .map(|hunk| hunk.decision)
                .collect::<Vec<_>>();
            assert_eq!(decisions, [HunkDecision::Accepted, HunkDecision::Rejected]);
        });

        cx.simulate_keystrokes("ctrl-enter");
        cx.run_until_parked();
        let outcome = held.await.unwrap();
        assert_eq!(
            outcome.content.as_deref(),
            Some("ONE\ntwo\nthree\nfour\nfive\n")
        );
        assert_eq!(
            pane.read_with(cx, |pane, _| pane.current_review_id()),
            None,
            "the pane waits for the next edit of the turn"
        );
        review.update(cx, |review, cx| {
            review.end_turn(&acp_v2::SessionId::new("session"), cx)
        });
        cx.run_until_parked();
        assert!(
            workspace.read_with(cx, |workspace, cx| workspace
                .items_of_type::<ReviewPane>(cx)
                .next()
                .is_none()),
            "the pane closes when the turn ends with nothing waiting"
        );
    }
}
