//! T3 at the moment of action (fyx:ui 0.4, docs/fyx/wit-0.4/design.md §C.3): `terminal:read`
//! and `terminal:write` asked on a card in the notification stack of the active window, the
//! "Always allow" warning, and the status-bar indicator while an "Always allow" holds.
//!
//! - **Host-drawn with provenance:** the extension's name and id come from the manifest; the
//!   terminal's title is the host's.
//! - **Click guard and no keyboard:** the buttons ignore clicks for [`CLICK_GUARD`] after the card
//!   appears, and no key answers it. It never takes focus.
//! - **A read** offers "Once" and "This terminal until it closes". **A write** shows the exact
//!   text (tabs and invisible characters marked) and is confirmed once: every command has its
//!   own card.
//! - **Deny** is every other way out: the close button, the stack dismissed, the window gone,
//!   or the runtime dropping the request (the terminal closed, the extension unloaded), which
//!   closes the card.
//! - **"Always allow <extension>"**, only for a component with a hash (not a dev folder), first
//!   shows the owner's warning; its confirm button works after [`CLICK_GUARD`]. It answers this
//!   card `Once` and holds until revoked or the component changes.

use std::sync::Arc;

use futures::channel::oneshot;
use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, WeakEntity, Window, prelude::*,
};
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Clickable as _, Color, Disableable as _, Icon,
    IconButton, IconName, IconSize, Label, LabelCommon as _, LabelSize, StyledExt as _, Tooltip,
    h_flex, v_flex,
};
use ui_ext::terminal::ReadScope;
use ui_ext_wasm::permissions::{T3Answer, T3Capability, T3Prompt};
use workspace::{ItemHandle, StatusItemView, notifications::SuppressEvent};

use crate::{
    UiExtensionRegistry,
    approvals::display_line,
    notifications::{CLICK_GUARD, ClickGuard, provenance_line},
};

/// What the card draws of the terminal's title.
pub const MAX_TERMINAL_TITLE_CHARS: usize = 80;

/// The owner's warning before "Always allow" (§C.3), in their words, for reading and writing.
pub fn always_allow_warning(name: &str) -> String {
    format!(
        "{name} will see everything that appears in your terminals, including passwords and \
         tokens, and run any command as you."
    )
}

/// The text a write card shows: exactly what will be typed, with each tab drawn as `⇥` and every
/// character that would hide or reorder text as `<U+XXXX>`. `check_write_text` already refused
/// control characters other than tab.
pub fn visible_write_text(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\t' => shown.push('⇥'),
            character if is_invisible(character) => {
                shown.push_str(&format!("<U+{:04X}>", character as u32));
            }
            character => shown.push(character),
        }
    }
    shown
}

fn is_invisible(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// What a read card says it reads.
fn read_scope_text(scope: Option<(ReadScope, u16)>) -> String {
    match scope {
        Some((ReadScope::LastCommand, _)) => "the last command and its output".into(),
        Some((ReadScope::LastLines, lines)) => format!("the last {lines} lines"),
        None => "its text".into(),
    }
}

/// One T3 card. Its answer goes back through the request's sender once; dropping the sender
/// unanswered is `Deny`.
pub struct T3Card {
    extension_id: Arc<str>,
    name: SharedString,
    prompt: T3Prompt,
    terminal_title: SharedString,
    write_text: SharedString,
    answer: Option<oneshot::Sender<T3Answer>>,
    can_always_allow: bool,
    /// The warning before "Always allow", with its own click guard.
    confirming_always: Option<ClickGuard>,
    registry: WeakEntity<UiExtensionRegistry>,
    guard: ClickGuard,
    focus_handle: FocusHandle,
    _cancelled: gpui::Task<()>,
}

impl T3Card {
    pub(crate) fn new(
        extension_id: Arc<str>,
        name: SharedString,
        prompt: T3Prompt,
        mut answer: oneshot::Sender<T3Answer>,
        can_always_allow: bool,
        registry: WeakEntity<UiExtensionRegistry>,
        cx: &mut Context<Self>,
    ) -> Self {
        // The runtime dropped the request: the terminal closed or the extension unloaded.
        let cancellation = answer.cancellation();
        let cancelled = cx.spawn(async move |this, cx| {
            cancellation.await;
            this.update(cx, |this, cx| {
                this.answer = None;
                cx.emit(DismissEvent);
            })
            .ok();
        });
        Self {
            extension_id,
            name,
            terminal_title: display_line(&prompt.terminal_title, MAX_TERMINAL_TITLE_CHARS),
            write_text: prompt
                .text
                .as_deref()
                .map(visible_write_text)
                .unwrap_or_default()
                .into(),
            prompt,
            answer: Some(answer),
            can_always_allow,
            confirming_always: None,
            registry,
            guard: ClickGuard::new(cx),
            focus_handle: cx.focus_handle(),
            _cancelled: cancelled,
        }
    }

    pub fn capability(&self) -> T3Capability {
        self.prompt.capability
    }

    pub(crate) fn buttons_armed(&self, cx: &App) -> bool {
        self.guard.armed(cx)
    }

    pub(crate) fn is_confirming_always(&self) -> bool {
        self.confirming_always.is_some()
    }

    /// The answers this card offers, in drawing order.
    pub fn answers(&self) -> &'static [T3Answer] {
        match self.prompt.capability {
            T3Capability::Read => &[T3Answer::Deny, T3Answer::UntilClosed, T3Answer::Once],
            T3Capability::Write => &[T3Answer::Deny, T3Answer::Once],
        }
    }

    fn send(&mut self, answer: T3Answer, cx: &mut Context<Self>) {
        if let Some(sender) = self.answer.take() {
            sender.send(answer).ok();
        }
        cx.emit(DismissEvent);
    }

    /// A button; ignored inside the click guard and for an answer the card does not offer.
    pub(crate) fn choose(&mut self, answer: T3Answer, cx: &mut Context<Self>) {
        if self.answer.is_none()
            || !self.buttons_armed(cx)
            || self.confirming_always.is_some()
            || !self.answers().contains(&answer)
        {
            return;
        }
        self.send(answer, cx);
    }

    /// The close button: `Deny`, at once (no guard: closing grants nothing).
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.answer.is_some() {
            self.send(T3Answer::Deny, cx);
        }
    }

    /// "Always allow <extension>": the warning first.
    pub(crate) fn ask_always_allow(&mut self, cx: &mut Context<Self>) {
        if self.answer.is_none() || !self.can_always_allow || !self.buttons_armed(cx) {
            return;
        }
        self.confirming_always = Some(ClickGuard::new(cx));
        cx.notify();
    }

    pub(crate) fn cancel_always_allow(&mut self, cx: &mut Context<Self>) {
        self.confirming_always = None;
        cx.notify();
    }

    /// The warning's confirm button, once its own guard passed: "Always allow" is saved and this
    /// request goes ahead once.
    pub(crate) fn confirm_always_allow(&mut self, cx: &mut Context<Self>) {
        if self.answer.is_none()
            || !self
                .confirming_always
                .as_ref()
                .is_some_and(|guard| guard.armed(cx))
        {
            return;
        }
        let (extension_id, capability) = (self.extension_id.clone(), self.prompt.capability);
        let saved = self
            .registry
            .update(cx, |registry, cx| {
                registry.set_always_allow(&extension_id, capability, true, cx)
            })
            .unwrap_or(false);
        self.send(if saved { T3Answer::Once } else { T3Answer::Deny }, cx);
    }

    fn title(&self) -> String {
        match self.prompt.capability {
            T3Capability::Read => format!(
                "{} wants to read {} of the terminal “{}”",
                self.name,
                read_scope_text(self.prompt.scope),
                self.terminal_title
            ),
            T3Capability::Write => format!(
                "{} wants to type into the terminal “{}”{}",
                self.name,
                self.terminal_title,
                if self.prompt.submit {
                    ", then press Enter"
                } else {
                    ""
                }
            ),
        }
    }
}

fn answer_label(capability: T3Capability, submit: bool, answer: T3Answer) -> &'static str {
    match (capability, answer) {
        (_, T3Answer::Deny) => "Deny",
        (T3Capability::Read, T3Answer::Once) => "Once",
        (T3Capability::Read, T3Answer::UntilClosed) => "This terminal until it closes",
        (T3Capability::Write, T3Answer::Once) if submit => "Type and press Enter",
        (T3Capability::Write, _) => "Type it",
    }
}

fn answer_element_id(answer: T3Answer) -> &'static str {
    match answer {
        T3Answer::Once => "fyx-ui-t3-once",
        T3Answer::UntilClosed => "fyx-ui-t3-until-closed",
        T3Answer::Deny => "fyx-ui-t3-deny",
    }
}

impl Drop for T3Card {
    fn drop(&mut self) {
        // Closed with the stack or the window: no answer is `Deny`.
        if let Some(sender) = self.answer.take() {
            sender.send(T3Answer::Deny).ok();
        }
    }
}

impl EventEmitter<DismissEvent> for T3Card {}
impl EventEmitter<SuppressEvent> for T3Card {}

impl Focusable for T3Card {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl workspace::notifications::Notification for T3Card {}

impl Render for T3Card {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let armed = self.buttons_armed(cx);
        let (capability, submit) = (self.prompt.capability, self.prompt.submit);
        let header = h_flex()
            .gap_2()
            .items_start()
            .justify_between()
            .child(
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .child(
                        Icon::new(IconName::Terminal)
                            .size(IconSize::Small)
                            .color(Color::Warning),
                    )
                    .child(Label::new(self.title())),
            )
            .child(
                IconButton::new("fyx-ui-t3-close", IconName::Close)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Deny"))
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            );
        // No key handlers: a keystroke never answers it.
        let card = v_flex()
            .id("fyx-ui-t3-card")
            .occlude()
            .p_3()
            .gap_2()
            .elevation_3(cx)
            .child(provenance_line(self.name.clone(), &self.extension_id))
            .child(header)
            .when(capability == T3Capability::Write, |this| {
                this.child(
                    div()
                        .id("fyx-ui-t3-text")
                        .max_h_48()
                        .overflow_y_scroll()
                        .p_1()
                        .rounded_sm()
                        .bg(cx.theme().colors().editor_background)
                        .font_buffer(cx)
                        .text_sm()
                        .child(self.write_text.clone()),
                )
            });
        if let Some(confirm_guard) = &self.confirming_always {
            let confirm_armed = confirm_guard.armed(cx);
            return card
                .child(
                    Label::new(always_allow_warning(&self.name))
                        .size(LabelSize::Small)
                        .color(Color::Warning),
                )
                .child(
                    Label::new(
                        "It holds until you revoke it, or until this extension's component \
                         changes. It never applies in an untrusted worktree.",
                    )
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .justify_end()
                        .child(
                            Button::new("fyx-ui-t3-always-cancel", "Cancel")
                                .style(ButtonStyle::Subtle)
                                .label_size(LabelSize::Small)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_always_allow(cx)),
                                ),
                        )
                        .child(
                            Button::new(
                                "fyx-ui-t3-always-confirm",
                                format!("Always allow {}", self.name),
                            )
                            .style(ButtonStyle::Tinted(ui::TintColor::Warning))
                            .label_size(LabelSize::Small)
                            .disabled(!confirm_armed)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_always_allow(cx))),
                        ),
                )
                .into_any_element();
        }
        let buttons = self.answers().iter().map(|&answer| {
            Button::new(
                answer_element_id(answer),
                answer_label(capability, submit, answer),
            )
            .style(if answer == T3Answer::Once {
                ButtonStyle::Filled
            } else {
                ButtonStyle::Subtle
            })
            .label_size(LabelSize::Small)
            // Looks pressable only once it is (the click guard).
            .disabled(!armed)
            .on_click(cx.listener(move |this, _, _, cx| this.choose(answer, cx)))
        });
        card.child(
            Label::new(match capability {
                T3Capability::Read => "Plain text, without colours. It goes to the extension.",
                T3Capability::Write => "Fyx types exactly this; the tab shows who typed it.",
            })
            .size(LabelSize::XSmall)
            .color(Color::Muted),
        )
        .child(
            h_flex()
                .gap_1()
                .justify_between()
                .child(div().when(self.can_always_allow, |this| {
                    this.child(
                        Button::new(
                            "fyx-ui-t3-always",
                            format!("Always allow {}…", self.name),
                        )
                        .style(ButtonStyle::Transparent)
                        .label_size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .disabled(!armed)
                        .on_click(cx.listener(|this, _, _, cx| this.ask_always_allow(cx))),
                    )
                }))
                .child(h_flex().gap_1().children(buttons)),
        )
        .into_any_element()
    }
}

const _: () = assert!(CLICK_GUARD.as_secs() >= 1);

/// The status bar while any T3 "Always allow" holds (§C.3): who has access, and one click to
/// revoke it all. Nothing otherwise.
pub struct T3AccessStatus {
    registry: Entity<UiExtensionRegistry>,
    _observe: Subscription,
}

impl T3AccessStatus {
    pub fn new(registry: Entity<UiExtensionRegistry>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&registry, |_, _, cx| cx.notify());
        Self {
            registry,
            _observe: observe,
        }
    }
}

/// "<extension> has access" for each extension that holds an "Always allow", for the terminal
/// tabs and the status bar.
pub fn access_labels(registry: &UiExtensionRegistry) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = registry
        .always_allowed()
        .into_iter()
        .map(|(extension_id, _)| registry.provenance_name(&extension_id))
        .collect();
    names.dedup();
    names
        .into_iter()
        .map(|name| SharedString::from(format!("{name} has access")))
        .collect()
}

impl Render for T3AccessStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let registry = self.registry.read(cx);
        let held = registry.always_allowed();
        if held.is_empty() {
            return div().into_any_element();
        }
        let labels = access_labels(registry);
        let label = match labels.as_slice() {
            [one] => format!("{one} to terminals"),
            _ => format!("{} extensions have terminal access", labels.len()),
        };
        let details = held
            .iter()
            .map(|(extension_id, capability)| {
                format!(
                    "{} may {} without asking",
                    registry.provenance_name(extension_id),
                    match capability {
                        T3Capability::Read => "read your terminals",
                        T3Capability::Write => "type into shared terminals",
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let registry = self.registry.clone();
        h_flex()
            .id("fyx-ui-t3-access")
            .gap_1()
            .child(
                Icon::new(IconName::Warning)
                    .size(IconSize::Small)
                    .color(Color::Warning),
            )
            .child(Label::new(label).size(LabelSize::Small))
            .tooltip(Tooltip::text(details))
            .child(
                IconButton::new("fyx-ui-t3-access-revoke", IconName::Close)
                    .icon_size(IconSize::XSmall)
                    .tooltip(Tooltip::text("Revoke terminal access"))
                    .on_click(move |_, _, cx| {
                        registry.update(cx, |registry, cx| {
                            for (extension_id, capability) in registry.always_allowed() {
                                registry.set_always_allow(&extension_id, capability, false, cx);
                            }
                        });
                    }),
            )
            .into_any_element()
    }
}

impl StatusItemView for T3AccessStatus {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }
}
