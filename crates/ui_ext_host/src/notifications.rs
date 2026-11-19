//! Extension notifications (fyx:ui 0.2, design.md M.1 and M.4): a card in Zed's notification
//! stack of the active window. It never takes focus.
//!
//! - **Provenance:** a first line with a puzzle icon, the extension's name and its id, that no
//!   field of the notification can change (M.4). A name that fails the registry's `name` rule
//!   shows as the id.
//! - **Buttons:** at most two, ignored for [`CLICK_GUARD`] after the card appears; the close
//!   button works at once.
//! - **Auto-dismiss:** the level's time, paused while the pointer is over the card and while the
//!   window is in the background, so an expired card was on screen for all of it.
//! - **Events** go to the owner through the registry: `notification-action` for a button,
//!   `notification-closed` with `user`, `expired` or `window-closed`. A dismiss by the extension
//!   or the owner's removal sends none.
//!
//! [`PermissionCard`] is the T2 first-use prompt in the same stack (decision Q10): non-modal,
//! with the same provenance line and click guard, and no keyboard answer.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, SharedString, Task,
    WeakEntity, Window, prelude::*,
};
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Clickable as _, Color, Disableable as _, Icon,
    IconButton, IconName, IconSize, Label, LabelCommon as _, LabelSize, StyledExt as _, Tooltip,
    h_flex, v_flex,
};
use ui_ext::v0_2::{
    Notification, NotificationCloseReason, NotificationLevel, notifications::BUTTON_CLICK_GUARD,
};
use workspace::notifications::SuppressEvent;

use crate::UiExtensionRegistry;

/// How long after a card appears its buttons ignore clicks, so that a click meant for what was
/// there before cannot answer it (M.1's click-jacking guard; decision Q10's 1 s for consent).
/// Every extension card uses it, so it is never shorter than 0.2's guard.
pub(crate) const CLICK_GUARD: Duration = Duration::from_secs(1);
const _: () = assert!(CLICK_GUARD.as_millis() >= BUTTON_CLICK_GUARD.as_millis());

/// When a card appeared, and a repaint once its buttons take clicks.
pub(crate) struct ClickGuard {
    shown_at: Instant,
    _arm: Task<()>,
}

impl ClickGuard {
    pub(crate) fn new<T: 'static>(cx: &mut Context<T>) -> Self {
        Self {
            shown_at: cx.background_executor().now(),
            _arm: cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLICK_GUARD).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            }),
        }
    }

    pub(crate) fn armed(&self, cx: &App) -> bool {
        cx.background_executor()
            .now()
            .saturating_duration_since(self.shown_at)
            >= CLICK_GUARD
    }
}

/// The first line of every extension card: drawn by the host only, from the manifest; nothing the
/// extension sends reaches it (M.4).
pub(crate) fn provenance_line(name: SharedString, extension_id: &str) -> impl IntoElement {
    h_flex()
        .id("fyx-ui-notification-provenance")
        .gap_1()
        .child(
            Icon::new(IconName::Blocks)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        )
        .child(Label::new(name).size(LabelSize::XSmall))
        .child(
            Label::new(format!("· {extension_id}"))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
}

/// The name in the provenance line: `name` if it passes the registry's rule (letters, digits and
/// plain punctuation, at most 64 characters), otherwise the id (M.4).
pub(crate) fn provenance_name(name: Option<&str>, extension_id: &str) -> SharedString {
    const MAX_NAME_CHARS: usize = 64;
    let fits = |name: &str| {
        let count = name.chars().count();
        (1..=MAX_NAME_CHARS).contains(&count)
            && name.trim() == name
            && name.chars().all(|character| {
                character.is_alphanumeric()
                    || character == ' '
                    || "-_.,:;'()&+!?/#@".contains(character)
            })
    };
    match name {
        Some(name) if fits(name) => SharedString::from(name.to_string()),
        _ => SharedString::from(extension_id.to_string()),
    }
}

pub struct ExtensionNotification {
    id: u64,
    extension_id: Arc<str>,
    name: SharedString,
    notification: Notification,
    registry: WeakEntity<UiExtensionRegistry>,
    guard: ClickGuard,
    /// Time left before it expires; `None` when it stays until closed.
    remaining: Option<Duration>,
    running_since: Option<Instant>,
    hovered: bool,
    window_active: bool,
    _timer: Option<Task<()>>,
    /// Set once the registry has been told (or did the closing itself).
    closed: bool,
    focus_handle: FocusHandle,
    _window_activation: gpui::Subscription,
}

impl ExtensionNotification {
    pub(crate) fn new(
        id: u64,
        extension_id: Arc<str>,
        name: SharedString,
        notification: Notification,
        registry: WeakEntity<UiExtensionRegistry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let remaining = notification
            .auto_dismiss
            .then(|| notification.level.auto_dismiss_after());
        cx.on_release(|this: &mut Self, cx| {
            // Gone without a close: its window closed.
            if !this.closed {
                this.tell_registry_closed(NotificationCloseReason::WindowClosed, cx);
            }
        })
        .detach();
        let mut this = Self {
            id,
            extension_id,
            name,
            notification,
            registry,
            guard: ClickGuard::new(cx),
            remaining,
            running_since: None,
            hovered: false,
            window_active: window.is_window_active(),
            _timer: None,
            closed: false,
            focus_handle: cx.focus_handle(),
            _window_activation: cx.observe_window_activation(window, |this, window, cx| {
                this.window_active = window.is_window_active();
                this.update_timer(cx);
            }),
        };
        this.update_timer(cx);
        this
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Restarts or pauses the auto-dismiss timer after a change of hover or window activation.
    fn update_timer(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        if let Some(since) = self.running_since.take() {
            self.remaining = self
                .remaining
                .map(|remaining| remaining.saturating_sub(now.saturating_duration_since(since)));
        }
        self._timer = None;
        let Some(remaining) = self.remaining else {
            return;
        };
        if self.hovered || !self.window_active {
            return;
        }
        self.running_since = Some(now);
        self._timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(remaining).await;
            this.update(cx, |this, cx| {
                this.close(NotificationCloseReason::Expired, cx)
            })
            .ok();
        }));
    }

    pub(crate) fn set_hovered(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if self.hovered != hovered {
            self.hovered = hovered;
            self.update_timer(cx);
        }
    }

    fn tell_registry_closed(&mut self, reason: NotificationCloseReason, cx: &mut App) {
        self.closed = true;
        let (extension_id, id) = (self.extension_id.clone(), self.id);
        self.registry
            .update(cx, |registry, cx| {
                registry.notification_closed(&extension_id, id, reason, cx)
            })
            .ok();
    }

    /// Closed by the user or its timer: the owner hears why.
    pub(crate) fn close(&mut self, reason: NotificationCloseReason, cx: &mut Context<Self>) {
        if self.closed {
            return;
        }
        self.tell_registry_closed(reason, cx);
        cx.emit(DismissEvent);
    }

    /// Closed by the extension (`dismiss-notification`) or its owner's removal: no event.
    pub(crate) fn close_by_host(&mut self, cx: &mut Context<Self>) {
        self.closed = true;
        cx.emit(DismissEvent);
    }

    /// Whether the buttons take clicks yet (M.1's guard against click-jacking).
    pub(crate) fn buttons_armed(&self, cx: &App) -> bool {
        self.guard.armed(cx)
    }

    pub(crate) fn click_action(&mut self, action: &str, cx: &mut Context<Self>) {
        if self.closed || !self.buttons_armed(cx) {
            return;
        }
        if !self
            .notification
            .actions
            .iter()
            .any(|known| known.id == action)
        {
            return;
        }
        self.closed = true;
        let (extension_id, id, action) = (self.extension_id.clone(), self.id, action.to_string());
        self.registry
            .update(cx, |registry, cx| {
                registry.notification_action(&extension_id, id, action, cx)
            })
            .ok();
        cx.emit(DismissEvent);
    }

    fn level_icon(&self) -> (IconName, Color) {
        match self.notification.level {
            NotificationLevel::Info => (IconName::Info, Color::Muted),
            NotificationLevel::Success => (IconName::Check, Color::Success),
            NotificationLevel::Warning => (IconName::Warning, Color::Warning),
            NotificationLevel::Error => (IconName::XCircle, Color::Error),
        }
    }
}

impl EventEmitter<DismissEvent> for ExtensionNotification {}
impl EventEmitter<SuppressEvent> for ExtensionNotification {}

impl Focusable for ExtensionNotification {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl workspace::notifications::Notification for ExtensionNotification {}

impl Render for ExtensionNotification {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (icon, color) = self.level_icon();
        let armed = self.buttons_armed(cx);
        let actions = self.notification.actions.clone();
        v_flex()
            .id(("fyx-ui-notification", self.id as usize))
            .occlude()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| this.set_hovered(*hovered, cx)))
            .on_action(cx.listener(|this, _: &menu::Cancel, _, cx| {
                this.close(NotificationCloseReason::User, cx)
            }))
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
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(Icon::new(icon).size(IconSize::Small).color(color))
                            .child(Label::new(self.notification.title.clone())),
                    )
                    .child(
                        IconButton::new("fyx-ui-notification-close", IconName::Close)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Close"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close(NotificationCloseReason::User, cx)
                            })),
                    ),
            )
            .when_some(self.notification.body.clone(), |this, body| {
                this.child(Label::new(body).size(LabelSize::Small).color(Color::Muted))
            })
            .when(!actions.is_empty(), |this| {
                this.child(h_flex().gap_1().justify_end().children(
                    actions.into_iter().enumerate().map(|(index, action)| {
                        let id = action.id.clone();
                        Button::new(("fyx-ui-notification-action", index), action.label)
                            .style(ButtonStyle::Filled)
                            .label_size(LabelSize::Small)
                            // Looks pressable only once it is (the click guard).
                            .disabled(!armed)
                            .on_click(cx.listener(move |this, _, _, cx| this.click_action(&id, cx)))
                    }),
                ))
            })
    }
}

/// The T2 first-use prompt (plan 4.5, decision Q10): a card in the notification stack of the
/// active window, never a modal, and it never takes focus. Allow and Don't Allow ignore clicks
/// for [`CLICK_GUARD`]; nothing answers it from the keyboard (no Enter default, no Escape). The
/// close button, or the window closing, answers nothing: the request is denied this time and
/// nothing is recorded.
pub struct PermissionCard {
    extension_id: Arc<str>,
    name: SharedString,
    capability: SharedString,
    title: SharedString,
    detail: SharedString,
    registry: WeakEntity<UiExtensionRegistry>,
    guard: ClickGuard,
    answered: bool,
    focus_handle: FocusHandle,
}

impl PermissionCard {
    pub(crate) fn new(
        extension_id: Arc<str>,
        capability: SharedString,
        registry: &gpui::Entity<UiExtensionRegistry>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (title, detail, name) = {
            let registry = registry.read(cx);
            let (title, detail) = registry.permission_prompt_text(&extension_id, &capability);
            (title, detail, registry.provenance_name(&extension_id))
        };
        cx.on_release(|this: &mut Self, cx| {
            if !this.answered {
                this.tell_registry(None, cx);
            }
        })
        .detach();
        Self {
            extension_id,
            name,
            capability,
            title: title.into(),
            detail: detail.into(),
            registry: registry.downgrade(),
            guard: ClickGuard::new(cx),
            answered: false,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn buttons_armed(&self, cx: &App) -> bool {
        self.guard.armed(cx)
    }

    fn tell_registry(&mut self, answer: Option<bool>, cx: &mut App) {
        self.answered = true;
        let (extension_id, capability) = (self.extension_id.clone(), self.capability.clone());
        self.registry
            .update(cx, |registry, cx| {
                registry.answer_permission(&extension_id, &capability, answer, false, cx)
            })
            .ok();
    }

    /// Allow or Don't Allow; ignored inside the click guard.
    pub(crate) fn answer(&mut self, allow: bool, cx: &mut Context<Self>) {
        if self.answered || !self.buttons_armed(cx) {
            return;
        }
        self.tell_registry(Some(allow), cx);
        cx.emit(DismissEvent);
    }

    /// The close button: no answer, so nothing is recorded.
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.answered {
            return;
        }
        self.tell_registry(None, cx);
        cx.emit(DismissEvent);
    }
}

impl EventEmitter<DismissEvent> for PermissionCard {}
impl EventEmitter<SuppressEvent> for PermissionCard {}

impl Focusable for PermissionCard {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl workspace::notifications::Notification for PermissionCard {}

impl Render for PermissionCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let armed = self.buttons_armed(cx);
        let button = |id: &'static str, label: &'static str, style, allow: bool| {
            Button::new(id, label)
                .style(style)
                .label_size(LabelSize::Small)
                // Looks pressable only once it is (the click guard).
                .disabled(!armed)
                .on_click(cx.listener(move |this, _, _, cx| this.answer(allow, cx)))
        };
        let deny = button(
            "fyx-ui-permission-deny",
            "Don't Allow",
            ButtonStyle::Subtle,
            false,
        );
        let allow = button(
            "fyx-ui-permission-allow",
            "Allow",
            ButtonStyle::Filled,
            true,
        );
        // No key handlers: a keystroke never answers it.
        v_flex()
            .id("fyx-ui-permission")
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
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                Icon::new(IconName::Warning)
                                    .size(IconSize::Small)
                                    .color(Color::Warning),
                            )
                            .child(Label::new(self.title.clone())),
                    )
                    .child(
                        IconButton::new("fyx-ui-permission-close", IconName::Close)
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
            .child(h_flex().gap_1().justify_end().child(deny).child(allow))
    }
}
