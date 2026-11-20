//! The 0.4 install dialog (docs/fyx/wit-0.4/design.md §C.2), for dev, file and registry installs
//! and for approving a held update (§C.5): four groups, one checkbox per T2 item, unticked.
//!
//! 1. **Shows in Fyx** (T0): one line.
//! 2. **Allowed without asking** (T1): a line per capability, no checkbox.
//! 3. **Off unless you tick it** (T2): a checkbox per item with its details, its risk line and the
//!    author's reason ("The author says: …").
//! 4. **Asks you each time** (T3): listed, no checkbox.
//!
//! An update's dialog (§C.5) lists only what changed: a group with no line is left out, "Shows in
//! Fyx" included.
//!
//! Every checkbox and the Install button ignore clicks for [`CLICK_GUARD`] after the dialog
//! appears. Install answers with what is ticked; Cancel, Escape or dismissing answers nothing.
//! There is no Enter default: a keystroke never installs.

use std::collections::BTreeSet;

use futures::channel::oneshot;
use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, FontWeight, ScrollHandle,
    Window, div, prelude::*, rems,
};
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Checkbox, Clickable as _, Color, Disableable as _,
    Label, LabelCommon as _, LabelSize, StyledExt as _, ToggleState, WithScrollbar as _, h_flex,
    v_flex,
};
use workspace::{ModalView, Workspace};

use crate::{
    consent_model::{
        ConsentItem, ConsentTier, ItemLine, ItemLineExt as _, readable_text, unique_warnings,
    },
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
    /// An update: its lines are what changed, and "Shows in Fyx" shows only when T0 did.
    pub update: bool,
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

/// The extension page turning one T2 item on (§C.2 "Later"): the same line, ticked, with the
/// same guard; answers whether the user confirmed.
pub fn confirm_item(
    workspace: &mut Workspace,
    extension_name: &str,
    line: ItemLine,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> oneshot::Receiver<Vec<ConsentItem>> {
    let (answer, receiver) = oneshot::channel();
    let text = ConsentDialogText {
        title: format!("Turn this on for {extension_name}?"),
        notes: Vec::new(),
        confirm: "Turn On".into(),
        update: false,
    };
    workspace.toggle_modal(window, cx, move |_, cx| {
        let mut dialog = ConsentDialog::new(text, vec![line.clone()], answer, cx);
        dialog.ticked.insert(line.item);
        dialog
    });
    receiver
}

/// The extension page turning T3 "Always allow" on (§C.3): the owner's warning, with the
/// same guard on its confirm button; resolves when the user confirms, dropped otherwise.
pub fn confirm_always_allow(
    workspace: &mut Workspace,
    extension_name: &str,
    capability: ui_ext_wasm::permissions::T3Capability,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> oneshot::Receiver<Vec<ConsentItem>> {
    let (answer, receiver) = oneshot::channel();
    let text = ConsentDialogText {
        title: format!(
            "Always allow {extension_name} to {} without asking?",
            match capability {
                ui_ext_wasm::permissions::T3Capability::Read => "read your terminals",
                ui_ext_wasm::permissions::T3Capability::Write => "type into shared terminals",
            }
        ),
        notes: vec![
            crate::t3_cards::always_allow_warning(extension_name),
            "It holds until you revoke it, or until this extension's component changes. It \
             never applies in an untrusted worktree."
                .into(),
        ],
        confirm: format!("Always allow {extension_name}"),
        update: false,
    };
    workspace.toggle_modal(window, cx, move |_, cx| {
        let mut dialog = ConsentDialog::new(text, Vec::new(), answer, cx);
        dialog.show_groups = false;
        dialog
    });
    receiver
}

/// Groups 2–4 of §C.2, by tier.
const GROUPS: &[(ConsentTier, &str)] = &[
    (ConsentTier::T1, "Allowed without asking"),
    (ConsentTier::T2, "Off unless you tick it"),
    (ConsentTier::T3, "Asks you each time"),
];

pub struct ConsentDialog {
    text: ConsentDialogText,
    lines: Vec<ItemLine>,
    ticked: BTreeSet<ConsentItem>,
    guard: ClickGuard,
    answer: Option<oneshot::Sender<Vec<ConsentItem>>>,
    focus_handle: FocusHandle,
    /// False for a warning with nothing to list ("Always allow", §C.3): no "Shows in Fyx".
    show_groups: bool,
    scroll: ScrollHandle,
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
            show_groups: true,
            scroll: ScrollHandle::new(),
        }
    }

    /// "Shows in Fyx": always on an install, on an update only when T0 changed; never on a
    /// warning.
    pub(crate) fn shows_t0_group(&self) -> bool {
        self.show_groups
            && (!self.text.update || self.lines.iter().any(|line| line.tier == ConsentTier::T0))
    }

    /// The headings drawn, in order: what a reader sees of the groups.
    pub fn headings(&self) -> Vec<&'static str> {
        let t0 = self.shows_t0_group().then_some("Shows in Fyx");
        t0.into_iter()
            .chain(
                GROUPS
                    .iter()
                    .filter(|(tier, _)| self.lines.iter().any(|line| line.tier == *tier))
                    .map(|(_, name)| *name),
            )
            .collect()
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

    /// Plan 7.12, owner's decision: one click enables everything, with the risks listed above
    /// the button. Ignored inside the click guard.
    pub(crate) fn install_all(&mut self, cx: &mut Context<Self>) {
        if !self.armed(cx) {
            return;
        }
        self.ticked = self
            .lines
            .iter()
            .filter(|line| line.has_checkbox())
            .map(|line| line.item.clone())
            .collect();
        self.install(cx);
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
        let title = Label::new(readable_text(&line.title)).size(LabelSize::Small);
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
                    let details: Vec<String> = line
                        .details
                        .iter()
                        .map(|detail| readable_text(detail))
                        .collect();
                    this.child(
                        Label::new(details.join("  ·  "))
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
                }),
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

/// A group heading: heavier than the lines under it, T2's in the warning colour.
fn heading(text: &'static str, tier: Option<ConsentTier>) -> impl IntoElement {
    Label::new(text)
        .size(LabelSize::Small)
        .weight(FontWeight::SEMIBOLD)
        .color(if tier == Some(ConsentTier::T2) {
            Color::Warning
        } else {
            Color::Default
        })
}

impl Render for ConsentDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let armed = self.armed(cx);
        let shows: Vec<String> = self
            .group(ConsentTier::T0)
            .map(|line| readable_text(&line.title))
            .collect();
        let lines = self.lines.clone();
        let mut sections = Vec::new();
        for &(tier, name) in GROUPS {
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
                        .child(heading(name, Some(tier)))
                        .child(v_flex().pl_2().gap_1().children(rows))
                        .into_any_element(),
                );
            }
        }
        let has_checkboxes = lines.iter().any(ItemLine::has_checkbox);
        // The risks, once, above the buttons: what the T2 items do, then each warning once.
        let t2_titles: Vec<String> = lines
            .iter()
            .filter(|line| line.has_checkbox())
            .map(|line| readable_text(&line.title))
            .collect();
        let warnings = unique_warnings(&lines);
        let nothing_ticked = self.ticked.is_empty();
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
            .child(Label::new(self.text.title.clone()).weight(FontWeight::SEMIBOLD))
            .child(
                v_flex()
                    .id("fyx-consent-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("fyx-consent-groups")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .gap_2()
                            .when(self.shows_t0_group(), |this| {
                                this.child(
                                    v_flex().gap_1().child(heading("Shows in Fyx", None)).child(
                                        Label::new(if shows.is_empty() {
                                            "Nothing beyond its own UI".to_string()
                                        } else {
                                            shows.join(", ")
                                        })
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                                )
                            })
                            .children(sections)
                            .children(self.text.notes.iter().map(|note| {
                                div().child(
                                    Label::new(note.clone())
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                            })),
                    )
                    .vertical_scrollbar_for(&self.scroll, window, cx),
            )
            .when(has_checkboxes || !warnings.is_empty(), |this| {
                this.child(
                    v_flex()
                        .pt_2()
                        .gap_0p5()
                        .border_t_1()
                        .border_color(cx.theme().colors().border_variant)
                        .when(has_checkboxes, |this| {
                            this.child(
                                Label::new(format!(
                                    "With all permissions it also: {}.",
                                    t2_titles.join("; ")
                                ))
                                .size(LabelSize::Small)
                                .color(Color::Warning),
                            )
                        })
                        .children(warnings.iter().map(|warning| {
                            Label::new(warning.clone())
                                .size(LabelSize::XSmall)
                                .color(Color::Warning)
                        }))
                        .when(has_checkboxes, |this| {
                            this.child(
                                Label::new(
                                    "Without them, those stay off and what needs them does not \
                                     work. You can turn each one on later on the extension's \
                                     page.",
                                )
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            )
                        }),
                )
            })
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
                        Button::new(
                            "fyx-consent-install",
                            match (has_checkboxes, nothing_ticked) {
                                (false, _) => self.text.confirm.clone(),
                                (true, true) => format!("{} Without These", self.text.confirm),
                                (true, false) => format!("{} With Ticked Only", self.text.confirm),
                            },
                        )
                        .style(if has_checkboxes {
                            ButtonStyle::Outlined
                        } else {
                            ButtonStyle::Filled
                        })
                        .disabled(!armed)
                        .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                    )
                    .when(has_checkboxes, |this| {
                        this.child(
                            Button::new(
                                "fyx-consent-install-all",
                                format!("{} With All Permissions", self.text.confirm),
                            )
                            .style(ButtonStyle::Filled)
                            .disabled(!armed)
                            .on_click(cx.listener(|this, _, _, cx| this.install_all(cx))),
                        )
                    }),
            )
    }
}

const _: () = assert!(CLICK_GUARD.as_secs() >= 1);
