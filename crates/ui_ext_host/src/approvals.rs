//! Approval cards (fyx:ui 0.4, docs/fyx/wit-0.4/design.md §B.12): a tool that an extension
//! drives (Claude, Codex, Copilot, Gemini, OpenCode) asks to run a step, and the extension shows
//! the question as a card in the notification stack of the active window.
//!
//! - **Host-drawn:** the provenance line (the extension's name and id) comes from the manifest;
//!   the tool's name and the question come from the extension and are drawn under a "says"
//!   marker, so they never read as Fyx's own text.
//! - **Click guard and no keyboard:** the buttons ignore clicks for [`CLICK_GUARD`] after the card
//!   appears, and no key answers it (no default button, no Escape). It never takes focus.
//! - **It grants nothing in Fyx.** The answer goes back to the tool through the extension, as
//!   `io-event::approval-answered(id, some(option))`. Closing the card, dismissing the stack, or
//!   its window going away answers `none`. A reload, disable or uninstall closes the cards of the
//!   old instance with no answer: that instance is gone.
//! - **Limits:** at most [`MAX_OPEN_APPROVALS`] open per extension; past that, or with no window
//!   to show in, the request is answered `none` at once.
//!
//! The runtime checks the request's text and returns `limit-exceeded` past the open cap; the
//! card draws defensively anyway ([`display_line`], [`display_detail`]).

use std::sync::Arc;

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, SharedString, WeakEntity,
    Window, prelude::*,
};
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Clickable as _, Color, Disableable as _, Icon,
    IconButton, IconName, IconSize, Label, LabelCommon as _, LabelSize, StyledExt as _, Tooltip,
    h_flex, v_flex,
};
use workspace::notifications::SuppressEvent;

use crate::{
    UiExtensionRegistry,
    notifications::{CLICK_GUARD, ClickGuard, provenance_line},
};

/// Open approval cards per extension (§B.12).
pub const MAX_OPEN_APPROVALS: usize = 4;
/// What the card draws of the tool's name and the title (one line each) and of the detail.
pub const MAX_TOOL_CHARS: usize = 40;
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_DETAIL_CHARS: usize = 2000;
pub const MAX_DETAIL_LINES: usize = 12;

/// The WIT `approval-option`. The card draws them in this order, whatever order the extension
/// asked in, so "Always allow" never sits where "Allow once" usually is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApprovalChoice {
    Deny,
    AlwaysAllow,
    AllowOnce,
}

impl ApprovalChoice {
    pub fn label(self) -> &'static str {
        match self {
            ApprovalChoice::AllowOnce => "Allow once",
            ApprovalChoice::AlwaysAllow => "Always allow",
            ApprovalChoice::Deny => "Deny",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            ApprovalChoice::AllowOnce => "fyx-ui-approval-allow-once",
            ApprovalChoice::AlwaysAllow => "fyx-ui-approval-always-allow",
            ApprovalChoice::Deny => "fyx-ui-approval-deny",
        }
    }
}

/// One `approvals.ask` the runtime accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// The tool's name as the extension states it ("Codex"); `None` when it does not say.
    pub tool: Option<String>,
    pub title: String,
    pub detail: String,
    /// At least one.
    pub options: Vec<ApprovalChoice>,
}

/// Whether a character would hide or reorder text: controls (newlines too unless `newlines`),
/// zero-width and bidi format characters, line and paragraph separators.
fn is_hidden(character: char, newlines: bool) -> bool {
    if character == '\n' {
        return !newlines;
    }
    character.is_control()
        || matches!(
            character,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// One line of the extension's text as the card draws it: hidden characters become spaces,
/// runs of whitespace one space, at most `max` characters.
pub(crate) fn display_line(text: &str, max: usize) -> SharedString {
    let visible: String = text
        .chars()
        .map(|character| {
            if is_hidden(character, false) {
                ' '
            } else {
                character
            }
        })
        .collect();
    let joined = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&joined, max).into()
}

/// The detail as the card draws it: hidden characters dropped (newlines kept), trailing spaces
/// trimmed, at most [`MAX_DETAIL_LINES`] lines and [`MAX_DETAIL_CHARS`] characters.
pub(crate) fn display_detail(text: &str) -> SharedString {
    let visible: String = text
        .chars()
        .filter(|&character| !is_hidden(character, true))
        .collect();
    let lines: Vec<&str> = visible.trim().lines().map(str::trim_end).collect();
    let mut shown = lines[..lines.len().min(MAX_DETAIL_LINES)].join("\n");
    if lines.len() > MAX_DETAIL_LINES {
        shown.push_str("\n…");
    }
    truncate_chars(&shown, MAX_DETAIL_CHARS).into()
}

/// The card. Its answer goes to [`UiExtensionRegistry::approval_answered`] once.
pub struct ApprovalCard {
    extension_id: Arc<str>,
    id: u64,
    name: SharedString,
    tool: Option<SharedString>,
    title: SharedString,
    detail: SharedString,
    /// In drawing order, without duplicates.
    options: Vec<ApprovalChoice>,
    registry: WeakEntity<UiExtensionRegistry>,
    guard: ClickGuard,
    answered: bool,
    focus_handle: FocusHandle,
}

impl ApprovalCard {
    pub(crate) fn new(
        extension_id: Arc<str>,
        id: u64,
        name: SharedString,
        request: ApprovalRequest,
        registry: WeakEntity<UiExtensionRegistry>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.on_release(|this: &mut Self, cx| {
            if !this.answered {
                this.tell_registry(None, cx);
            }
        })
        .detach();
        let mut options = request.options;
        options.sort_unstable();
        options.dedup();
        Self {
            extension_id,
            id,
            name,
            tool: request
                .tool
                .as_deref()
                .map(|tool| display_line(tool, MAX_TOOL_CHARS))
                .filter(|tool| !tool.is_empty()),
            title: display_line(&request.title, MAX_TITLE_CHARS),
            detail: display_detail(&request.detail),
            options,
            registry,
            guard: ClickGuard::new(cx),
            answered: false,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn buttons_armed(&self, cx: &App) -> bool {
        self.guard.armed(cx)
    }

    #[cfg(test)]
    pub(crate) fn options(&self) -> &[ApprovalChoice] {
        &self.options
    }

    fn tell_registry(&mut self, answer: Option<ApprovalChoice>, cx: &mut App) {
        self.answered = true;
        let (extension_id, id) = (self.extension_id.clone(), self.id);
        self.registry
            .update(cx, |registry, _| {
                registry.approval_answered(&extension_id, id, answer)
            })
            .ok();
    }

    /// A button; ignored inside the click guard and for an option the extension did not offer.
    pub(crate) fn answer(&mut self, choice: ApprovalChoice, cx: &mut Context<Self>) {
        if self.answered || !self.buttons_armed(cx) || !self.options.contains(&choice) {
            return;
        }
        self.tell_registry(Some(choice), cx);
        cx.emit(DismissEvent);
    }

    /// The close button: answers `none`, at once (no guard: closing grants nothing).
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.answered {
            return;
        }
        self.tell_registry(None, cx);
        cx.emit(DismissEvent);
    }

    /// The instance is gone (reload, disable, uninstall): the card closes with no answer.
    pub(crate) fn close_by_host(&mut self, cx: &mut Context<Self>) {
        self.answered = true;
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for ApprovalCard {}
impl EventEmitter<SuppressEvent> for ApprovalCard {}

impl Focusable for ApprovalCard {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl workspace::notifications::Notification for ApprovalCard {}

impl Render for ApprovalCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let armed = self.buttons_armed(cx);
        let buttons = self.options.iter().map(|&choice| {
            Button::new(choice.element_id(), choice.label())
                .style(if choice == ApprovalChoice::AllowOnce {
                    ButtonStyle::Filled
                } else {
                    ButtonStyle::Subtle
                })
                .label_size(LabelSize::Small)
                // Looks pressable only once it is (the click guard).
                .disabled(!armed)
                .on_click(cx.listener(move |this, _, _, cx| this.answer(choice, cx)))
        });
        let speaker = match &self.tool {
            Some(tool) => format!("{tool} says, through {}:", self.name),
            None => format!("A tool says, through {}:", self.name),
        };
        let note = format!(
            "Your answer goes back to {} through {}. It changes nothing in Fyx.",
            self.tool.as_ref().map_or("the tool", |tool| tool.as_ref()),
            self.name
        );
        // No key handlers: a keystroke never answers it.
        v_flex()
            .id("fyx-ui-approval")
            .occlude()
            .p_3()
            .gap_2()
            .elevation_3(cx)
            .child(provenance_line(self.name.clone(), &self.extension_id))
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .justify_between()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                Label::new(speaker)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        Icon::new(IconName::Terminal)
                                            .size(IconSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .child(Label::new(self.title.clone())),
                            ),
                    )
                    .child(
                        IconButton::new("fyx-ui-approval-close", IconName::Close)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Close without answering"))
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    ),
            )
            .when(!self.detail.is_empty(), |this| {
                this.child(
                    Label::new(self.detail.clone())
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .child(Label::new(note).size(LabelSize::XSmall).color(Color::Muted))
            .child(h_flex().gap_1().justify_end().children(buttons))
    }
}

const _: () = assert!(CLICK_GUARD.as_secs() >= 1);
