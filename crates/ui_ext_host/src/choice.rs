//! Fyx's own dialogs for the UI extension commands (W10 user check, item 8): in the window and
//! styled like the app, instead of the system's message boxes.
//!
//! - [`choose`]: a list in the command palette's style. Typing filters it; Up, Down and Enter, or
//!   a click, pick an entry; Escape or a click outside cancels. It scales to any number of
//!   entries.
//!
//! It answers like `Window::prompt`: the index of the entry, or `Canceled` when the dialog went
//! without an answer.

use editor::{Editor, EditorEvent};
use futures::channel::oneshot;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window, prelude::*, rems,
};
use theme::ActiveTheme as _;
use ui::{Label, LabelCommon as _, LabelSize, ListItem, StyledExt as _, Toggleable as _, v_flex};
use workspace::{ModalView, Workspace};

/// Shows a filterable list titled `title` and answers the index of the picked entry.
pub fn choose(
    workspace: &mut Workspace,
    title: &str,
    detail: Option<&str>,
    entries: Vec<String>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> oneshot::Receiver<usize> {
    let (answer, receiver) = oneshot::channel();
    let title = SharedString::from(title.to_string());
    let detail = detail.map(|detail| SharedString::from(detail.to_string()));
    workspace.toggle_modal(window, cx, move |window, cx| {
        ChoiceModal::new(title, detail, entries, answer, window, cx)
    });
    receiver
}

pub struct ChoiceModal {
    title: SharedString,
    detail: Option<SharedString>,
    entries: Vec<String>,
    query: Entity<Editor>,
    /// Into `matches`.
    selected: usize,
    /// Indices into `entries` that match the query.
    matches: Vec<usize>,
    answer: Option<oneshot::Sender<usize>>,
    _query_edited: Subscription,
}

impl ChoiceModal {
    fn new(
        title: SharedString,
        detail: Option<SharedString>,
        entries: Vec<String>,
        answer: oneshot::Sender<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Type to filter…", window, cx);
            editor
        });
        let query_edited = cx.subscribe(&query, |this: &mut Self, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                this.filter(cx);
            }
        });
        query.focus_handle(cx).focus(window, cx);
        Self {
            title,
            detail,
            matches: (0..entries.len()).collect(),
            entries,
            query,
            selected: 0,
            answer: Some(answer),
            _query_edited: query_edited,
        }
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).text(cx).to_lowercase();
        let query = query.trim();
        self.matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.to_lowercase().contains(query))
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
        cx.notify();
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(answer) = self.answer.take() {
            answer.send(index).ok();
        }
        cx.emit(DismissEvent);
    }

    fn confirm(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(&index) = self.matches.get(self.selected) {
            self.pick(index, cx);
        }
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
            cx.notify();
        }
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = self.selected.saturating_sub(1);
        cx.notify();
    }
}

impl EventEmitter<DismissEvent> for ChoiceModal {}

impl Focusable for ChoiceModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}

impl ModalView for ChoiceModal {}

impl Render for ChoiceModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let max_height = window.viewport_size().height * 0.6;
        let rows: Vec<_> = self
            .matches
            .iter()
            .enumerate()
            .map(|(position, &index)| {
                ListItem::new(("fyx-choice", index))
                    .inset(true)
                    .toggle_state(position == self.selected)
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(index, cx)))
                    .child(Label::new(self.entries[index].clone()))
            })
            .collect();
        v_flex()
            .key_context("Picker")
            .elevation_3(cx)
            .w(rems(34.))
            .max_h(max_height)
            .overflow_hidden()
            .bg(cx.theme().colors().elevated_surface_background)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .child(
                v_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(Label::new(self.title.clone()).size(LabelSize::Small))
                    .when_some(self.detail.clone(), |this, detail| {
                        this.child(
                            Label::new(detail)
                                .size(LabelSize::XSmall)
                                .color(ui::Color::Muted),
                        )
                    })
                    .child(self.query.clone()),
            )
            .child(
                v_flex()
                    .id("fyx-choice-list")
                    .p_1()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(self.matches.is_empty(), |this| {
                        this.child(
                            Label::new("No match")
                                .size(LabelSize::Small)
                                .color(ui::Color::Muted),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    async fn test_choose_filters_and_answers(cx: &mut TestAppContext) {
        crate::tests::init_test(cx);
        let fs = fs::FakeFs::new(cx.executor());
        let project = project::Project::test(fs, [], cx).await;
        let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
            workspace::MultiWorkspace::test_new(project.clone(), window, cx)
        });
        let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());
        // The workspace focuses its pane first.
        crate::decoration_tests::redraw(cx);
        let entries = vec![
            "Counter".to_string(),
            "TODOs".to_string(),
            "Charts".to_string(),
        ];
        let mut answer = workspace.update_in(cx, |workspace, window, cx| {
            choose(
                workspace,
                "Open which extension panel?",
                None,
                entries,
                window,
                cx,
            )
        });
        cx.run_until_parked();
        assert!(
            workspace.read_with(cx, |workspace, cx| workspace
                .active_modal::<ChoiceModal>(cx)
                .is_some()),
            "the list is shown"
        );
        crate::decoration_tests::redraw(cx);
        cx.simulate_input("to");
        crate::decoration_tests::redraw(cx);
        assert_eq!(
            workspace.read_with(cx, |workspace, cx| workspace
                .active_modal::<ChoiceModal>(cx)
                .map(|modal| modal.read(cx).matches.clone())),
            Some(vec![1])
        );
        cx.dispatch_action(menu::Confirm);
        cx.run_until_parked();
        assert_eq!(
            answer.try_recv(),
            Ok(Some(1)),
            "the filter leaves TODOs only"
        );
    }
}
