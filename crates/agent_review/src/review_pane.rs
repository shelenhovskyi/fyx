//! The review UI: one workspace item per open review, showing the proposed edit in Zed's diff editor
//! (an `acp_thread::Diff` multibuffer, as the agent panel renders it) with per-hunk Accept / Reject.
//!
//! While a review pane has focus the host owns the keyboard: a keystroke interceptor turns the review
//! keys into review actions before the focused editor's own bindings see them. The interceptor survives
//! keymap reloads, which clear bindings registered with `bind_keys`.

use std::{ops::Range, path::Path, sync::Arc};

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
    Item, Workspace,
    item::{ItemEvent, TabContentParams},
};

use crate::{AgentReview, AgentReviewEvent, HunkDecision, ProposedEdit, Resolution, ReviewId};

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
            if let Some(review) = AgentReview::global(cx) {
                review.update(cx, |review, cx| {
                    let enabled = !review.is_enabled();
                    review.set_enabled(enabled, cx);
                    log::info!(
                        "fyx: agent review mode {}",
                        if enabled { "on" } else { "off" }
                    );
                });
            }
        });
        let Some(review) = AgentReview::global(cx) else {
            return;
        };
        cx.subscribe_in(&review, window, |workspace, _, event, window, cx| {
            if let AgentReviewEvent::Opened(id) = event {
                open_reviews(workspace, Some(*id), window, cx);
            }
        })
        .detach();
    })
    .detach();
}

/// Opens a pane for each open review (or only `only`) whose file belongs to this workspace's project.
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
    let ids = review
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
        .map(|candidate| candidate.id())
        .collect::<Vec<_>>();
    for id in ids {
        let existing = workspace
            .items_of_type::<ReviewPane>(cx)
            .find(|pane| pane.read(cx).review_id == id);
        match existing {
            Some(pane) => {
                workspace.activate_item(&pane, true, true, window, cx);
            }
            None => {
                let Some(pane) = ReviewPane::new(review.clone(), id, &project, window, cx) else {
                    continue;
                };
                let pane = cx.new(|_| pane);
                workspace.add_item_to_active_pane(Box::new(pane.clone()), None, true, window, cx);
                pane.update(cx, |pane, cx| pane.focus_first_hunk(window, cx));
            }
        }
    }
}

pub enum ReviewPaneEvent {
    Close,
}

pub struct ReviewPane {
    review: Entity<AgentReview>,
    review_id: ReviewId,
    title: SharedString,
    _diff: Entity<Diff>,
    editor: Entity<Editor>,
    comment_editor: Entity<Editor>,
    current_hunk: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl ReviewPane {
    fn new(
        review: Entity<AgentReview>,
        review_id: ReviewId,
        project: &Entity<Project>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Option<Self> {
        let edit = review.read(cx).review(review_id)?.edit().clone();
        let path = edit.abs_path().to_string_lossy().into_owned();
        let title = file_name(edit.abs_path());
        let language_registry = project.read(cx).languages().clone();
        let diff = cx.new(|_| {
            Diff::finalized(
                path,
                Some(edit.old_text().to_string()),
                edit.new_text().to_string(),
                language_registry,
            )
        });
        let multibuffer = diff.update(cx, |diff, cx| diff.multibuffer(cx));
        let renderer: Arc<dyn DiffHunkRenderer> = Arc::new(ReviewHunkRenderer {
            review: review.downgrade(),
            review_id,
        });
        let editor = cx.new(|cx| {
            let mut editor = Editor::new(
                EditorMode::Full {
                    scale_ui_elements_with_buffer_font_size: true,
                    show_active_line_background: true,
                    sizing_behavior: SizingBehavior::Default,
                },
                multibuffer,
                Some(project.clone()),
                window,
                cx,
            );
            editor.set_read_only(true);
            editor.disable_diagnostics(cx);
            editor.set_expand_all_diff_hunks(cx);
            editor.set_diff_hunk_renderer(Some(renderer), cx);
            editor
        });
        let comment_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Comment for the agent on this hunk (alt-c)", window, cx);
            editor
        });
        Some(Self {
            review,
            review_id,
            title,
            _diff: diff,
            editor,
            comment_editor,
            current_hunk: None,
            _subscriptions: Vec::new(),
        })
    }

    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self._subscriptions.is_empty() {
            return;
        }
        let review_id = self.review_id;
        self._subscriptions = vec![
            cx.subscribe(&self.review, move |this, _, event, cx| match event {
                AgentReviewEvent::Updated(id) | AgentReviewEvent::Resolved(id)
                    if *id == review_id =>
                {
                    this.editor.update(cx, |_, cx| cx.notify());
                    cx.notify();
                }
                AgentReviewEvent::Closed(id) if *id == review_id => {
                    cx.emit(ReviewPaneEvent::Close);
                }
                _ => {}
            }),
            cx.subscribe_in(&self.editor, window, |this, _, event, window, cx| {
                if let EditorEvent::SelectionsChanged { .. } = event {
                    this.update_current_hunk(window, cx);
                }
            }),
            cx.subscribe(&self.comment_editor, |this, editor, event, cx| {
                if let EditorEvent::BufferEdited = event {
                    let text = editor.read(cx).text(cx);
                    this.update_review(cx, |review, id, hunk, cx| {
                        review.set_comment(id, hunk, Some(text), cx)
                    });
                }
            }),
        ];
    }

    fn focus_first_hunk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscribe(window, cx);
        let focus = self.editor.focus_handle(cx);
        window.focus(&focus, cx);
        focus.dispatch_action(&GoToHunk, window, cx);
        self.update_current_hunk(window, cx);
    }

    pub fn review_id(&self) -> ReviewId {
        self.review_id
    }

    pub(crate) fn diff_focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }

    /// Makes `hunk` the target of the hunk actions, as moving the cursor into it would.
    pub(crate) fn select_hunk(
        &mut self,
        hunk: usize,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let hunk_count = self.edit(cx).map_or(0, |edit| edit.hunks().len());
        anyhow::ensure!(hunk < hunk_count, "hunk {} of {hunk_count}", hunk + 1);
        self.current_hunk = Some(hunk);
        cx.notify();
        Ok(())
    }

    /// Types `text` into the comment field of `hunk`.
    pub(crate) fn set_comment_text(
        &mut self,
        hunk: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.select_hunk(hunk, cx)?;
        self.comment_editor
            .update(cx, |editor, cx| editor.set_text(text, window, cx));
        Ok(())
    }

    pub(crate) fn edit<'a>(&self, cx: &'a App) -> Option<&'a ProposedEdit> {
        Some(self.review.read(cx).review(self.review_id)?.edit())
    }

    fn update_current_hunk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.editor.update(cx, |editor, cx| {
            let snapshot = editor.buffer().read(cx).snapshot(cx);
            let head = editor.selections.newest_anchor().head();
            buffer_row(&snapshot, head)
        });
        let hunk = row.and_then(|row| self.edit(cx)?.hunk_at_new_row(row));
        if hunk == self.current_hunk {
            return;
        }
        self.current_hunk = hunk;
        let comment = hunk
            .and_then(|hunk| self.edit(cx)?.hunks().get(hunk)?.comment.clone())
            .unwrap_or_default();
        self.comment_editor
            .update(cx, |editor, cx| editor.set_text(comment, window, cx));
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
        let Some(hunk) = self.current_hunk else {
            return;
        };
        let id = self.review_id;
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
            self.editor
                .focus_handle(cx)
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
        let id = self.review_id;
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

    fn next_hunk(&mut self, _: &NextHunk, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.editor.focus_handle(cx);
        window.focus(&focus, cx);
        focus.dispatch_action(&GoToHunk, window, cx);
    }

    fn previous_hunk(&mut self, _: &PreviousHunk, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.editor.focus_handle(cx);
        window.focus(&focus, cx);
        focus.dispatch_action(&GoToPreviousHunk, window, cx);
    }

    fn comment_hunk(&mut self, _: &CommentHunk, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_hunk.is_some() {
            window.focus(&self.comment_editor.focus_handle(cx), cx);
        }
    }

    fn focus_diff(&mut self, _: &FocusDiff, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.editor.focus_handle(cx), cx);
    }

    fn submit(&mut self, _: &SubmitReview, _: &mut Window, cx: &mut Context<Self>) {
        let id = self.review_id;
        if let Err(error) = self.review.update(cx, |review, cx| review.submit(id, cx)) {
            log::info!("fyx: review not submitted: {error:#}");
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let edit = self.edit(cx);
        let hunk_count = edit.map_or(0, |edit| edit.hunks().len());
        let resolution = edit.map(ProposedEdit::resolution);
        let status = match resolution {
            Some(Resolution::Pending { undecided }) => {
                format!("{undecided} of {hunk_count} undecided")
            }
            Some(Resolution::AcceptedAll) => "all hunks accepted".into(),
            Some(Resolution::RejectedAll) => "all hunks rejected".into(),
            Some(Resolution::Partial { accepted, rejected }) => {
                format!("{accepted} accepted, {rejected} rejected")
            }
            Some(Resolution::Unchanged) => "no changes".into(),
            None => "review closed".into(),
        };
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
        let can_submit = resolution.is_some_and(|resolution| !resolution.is_pending());

        v_flex()
            .p_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(Label::new("Agent edit review"))
                    .child(Label::new(path).size(LabelSize::Small).color(Color::Muted))
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
                    .child(self.action_button("accept-all", "Accept all", &AcceptAllHunks, cx))
                    .child(self.action_button("reject-all", "Reject all", &RejectAllHunks, cx))
                    .child(
                        self.action_button("submit", "Submit", &SubmitReview, cx)
                            .style(ButtonStyle::Filled)
                            .disabled(!can_submit),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new("Comment")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(div().flex_1().child(self.comment_editor.clone())),
            )
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
        self.editor.focus_handle(cx)
    }
}

impl Render for ReviewPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.subscribe(window, cx);
        v_flex()
            .key_context(KEY_CONTEXT)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .on_action(cx.listener(Self::accept_hunk))
            .on_action(cx.listener(Self::reject_hunk))
            .on_action(cx.listener(Self::accept_all))
            .on_action(cx.listener(Self::reject_all))
            .on_action(cx.listener(Self::next_hunk))
            .on_action(cx.listener(Self::previous_hunk))
            .on_action(cx.listener(Self::comment_hunk))
            .on_action(cx.listener(Self::focus_diff))
            .on_action(cx.listener(Self::submit))
            .child(self.render_header(cx))
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
    }
}

impl Item for ReviewPane {
    type Event = ReviewPaneEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        format!("Review: {}", self.title).into()
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

struct ReviewHunkRenderer {
    review: WeakEntity<AgentReview>,
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
        let hunk = buffer_row(&snapshot, hunk_range.start).and_then(|row| {
            let edit = review.read(cx).review(review_id)?.edit();
            let index = edit.hunk_at_new_row(row)?;
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
                .on_click(move |_, _, cx| {
                    review.update(cx, |review, cx| {
                        if let Err(error) = review.set_decision(review_id, index, choice, cx) {
                            log::debug!("review update failed: {error:#}");
                        }
                    });
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
        pane.update_in(cx, |pane, window, cx| pane.focus_first_hunk(window, cx));
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
        assert!(
            workspace.read_with(cx, |workspace, cx| workspace
                .items_of_type::<ReviewPane>(cx)
                .next()
                .is_none()),
            "the pane closes with its review"
        );
    }
}
