//! The 0.4 install dialog (docs/fyx/wit-0.4/design.md §C.2), for dev, file and registry installs
//! and for approving a held update (§C.5): four groups, one checkbox per T2 item, unticked.
//!
//! 1. **Shows in Fyx** (T0): one line.
//! 2. **Reads** (T1): a line per capability, no checkbox.
//! 3. **Can run code or reach outside Fyx** (T2): a checkbox per item with its details and the
//!    author's reason ("The author says: …").
//! 4. **Asks you each time** (T3): listed, no checkbox.
//!
//! Every checkbox and the Install button ignore clicks for [`CLICK_GUARD`] after the dialog
//! appears. Install answers with what is ticked; Cancel, Escape or dismissing answers nothing.
//! There is no Enter default: a keystroke never installs.

use std::collections::BTreeSet;

use futures::channel::oneshot;
use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, Window, div, prelude::*, rems,
};
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Checkbox, Clickable as _, Color, Disableable as _,
    Label, LabelCommon as _, LabelSize, StyledExt as _, ToggleState, h_flex, v_flex,
};
use workspace::{ModalView, Workspace};

use crate::{
    consent_model::{ConsentItem, ConsentTier, ItemLine},
    notifications::{CLICK_GUARD, ClickGuard},
};

/// What the dialog shows besides the lines.
#[derive(Clone, Debug, Default)]
pub struct ConsentDialogText {
    /// "Install Codex Tools 1.2?"
    pub title: String,
    /// Where it comes from (a folder, the registry with its owners and commit), the Zed-extension
    /// warning, or the update's removals.
    pub notes: Vec<String>,
    /// "Install", or "Approve and Update".
    pub confirm: String,
}

/// Shows the dialog and answers the ticked items when the user confirms; dropped otherwise.
pub fn ask_consent(
    workspace: &mut Workspace,
    text: ConsentDialogText,
    lines: Vec<ItemLine>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> oneshot::Receiver<Vec<ConsentItem>> {
    let (answer, receiver) = oneshot::channel();
    workspace.toggle_modal(window, cx, move |_, cx| {
        ConsentDialog::new(text, lines, answer, cx)
    });
    receiver
}

pub struct ConsentDialog {
    text: ConsentDialogText,
    lines: Vec<ItemLine>,
    ticked: BTreeSet<ConsentItem>,
    guard: ClickGuard,
    answer: Option<oneshot::Sender<Vec<ConsentItem>>>,
    focus_handle: FocusHandle,
}

impl ConsentDialog {
    pub(crate) fn new(
        text: ConsentDialogText,
        lines: Vec<ItemLine>,
        answer: oneshot::Sender<Vec<ConsentItem>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            text,
            lines,
            ticked: BTreeSet::new(),
            guard: ClickGuard::new(cx),
            answer: Some(answer),
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn armed(&self, cx: &App) -> bool {
        self.guard.armed(cx)
    }

    /// A checkbox; ignored inside the click guard and for lines without one.
    pub(crate) fn toggle(&mut self, item: &ConsentItem, cx: &mut Context<Self>) {
        if !self.armed(cx)
            || !self
                .lines
                .iter()
                .any(|line| &line.item == item && line.has_checkbox())
        {
            return;
        }
        if !self.ticked.remove(item) {
            self.ticked.insert(item.clone());
        }
        cx.notify();
    }

    pub(crate) fn ticked(&self) -> Vec<ConsentItem> {
        self.ticked.iter().cloned().collect()
    }

    /// Install (or Approve): ignored inside the click guard.
    pub(crate) fn install(&mut self, cx: &mut Context<Self>) {
        if !self.armed(cx) {
            return;
        }
        if let Some(answer) = self.answer.take() {
            answer.send(self.ticked()).ok();
        }
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn group(&self, tier: ConsentTier) -> impl Iterator<Item = &ItemLine> {
        self.lines.iter().filter(move |line| line.tier == tier)
    }

    fn render_line(
        &self,
        index: usize,
        line: &ItemLine,
        armed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let title = Label::new(line.title.clone()).size(LabelSize::Small);
        let head = if line.has_checkbox() {
            let item = line.item.clone();
            let state = if self.ticked.contains(&line.item) {
                ToggleState::Selected
            } else {
                ToggleState::Unselected
            };
            h_flex()
                .gap_2()
                .child(
                    Checkbox::new(("fyx-consent-item", index), state)
                        // Looks clickable only once it is (the click guard).
                        .disabled(!armed)
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(&item, cx))),
                )
                .child(title)
        } else {
            h_flex().gap_2().child(title)
        };
        v_flex().gap_0p5().child(head).child(
            v_flex()
                .pl_6()
                .when(!line.details.is_empty(), |this| {
                    this.child(
                        Label::new(line.details.join("  ·  "))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                })
                .when_some(line.reason.clone(), |this, reason| {
                    this.child(
                        Label::new(format!("The author says: {reason}"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                            .italic(),
                    )
                })
                .children(line.warnings.iter().map(|warning| {
                    Label::new(warning.clone())
                        .size(LabelSize::XSmall)
                        .color(Color::Warning)
                })),
        )
    }
}

impl EventEmitter<DismissEvent> for ConsentDialog {}

impl Focusable for ConsentDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for ConsentDialog {}

fn heading(text: &'static str) -> impl IntoElement {
    Label::new(text)
        .size(LabelSize::Small)
        .color(Color::Default)
}

impl Render for ConsentDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let armed = self.armed(cx);
        let shows: Vec<String> = self
            .group(ConsentTier::T0)
            .map(|line| line.title.clone())
            .collect();
        let lines = self.lines.clone();
        let mut sections = Vec::new();
        for (tier, name) in [
            (ConsentTier::T1, "Reads"),
            (ConsentTier::T2, "Can run code or reach outside Fyx"),
            (ConsentTier::T3, "Asks you each time"),
        ] {
            let rows: Vec<_> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.tier == tier)
                .map(|(index, line)| self.render_line(index, line, armed, cx).into_any_element())
                .collect();
            if !rows.is_empty() {
                sections.push(
                    v_flex()
                        .gap_1()
                        .child(heading(name))
                        .children(rows)
                        .into_any_element(),
                );
            }
        }
        let has_checkboxes = lines.iter().any(ItemLine::has_checkbox);
        v_flex()
            .track_focus(&self.focus_handle)
            .elevation_3(cx)
            .w(rems(38.))
            .max_h(rems(40.))
            .p_3()
            .gap_2()
            .bg(cx.theme().colors().elevated_surface_background)
            // Escape cancels; nothing confirms from the keyboard.
            .on_action(cx.listener(Self::cancel))
            .child(Label::new(self.text.title.clone()))
            .child(
                v_flex()
                    .id("fyx-consent-groups")
                    .overflow_y_scroll()
                    .gap_2()
                    .child(
                        v_flex().gap_1().child(heading("Shows in Fyx")).child(
                            Label::new(if shows.is_empty() {
                                "Nothing beyond its own UI".to_string()
                            } else {
                                shows.join(", ")
                            })
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                        ),
                    )
                    .children(sections)
                    .when(has_checkboxes, |this| {
                        this.child(
                            Label::new(
                                "Unticked items stay off. You can change each one later on the \
                                 extension's page.",
                            )
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                        )
                    })
                    .children(self.text.notes.iter().map(|note| {
                        div().child(
                            Label::new(note.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                    })),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_1()
                    .child(
                        Button::new("fyx-consent-cancel", "Cancel")
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                    )
                    .child(
                        Button::new("fyx-consent-install", self.text.confirm.clone())
                            .style(ButtonStyle::Filled)
                            .disabled(!armed)
                            .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                    ),
            )
    }
}

const _: () = assert!(CLICK_GUARD.as_secs() >= 1);
