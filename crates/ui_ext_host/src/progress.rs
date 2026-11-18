//! Extension progress (fyx:ui 0.2, design.md M.2): what `progress-start`, `progress-update` and
//! `progress-end` draw.
//!
//! - [`Progresses`] holds the running progresses of every extension and notifies at most
//!   [`MAX_PROGRESS_REDRAWS_PER_SECOND`] times a second, keeping only the last value.
//! - `status-bar`: one host status-bar chip per extension with a running one
//!   ([`ExtensionProgressStatus`], one per workspace): the provenance (puzzle icon and name, M.4),
//!   a spinner or the percentage and the newest progress's title; its tooltip lists them all with
//!   their messages, and a click offers Cancel and the owner panel.
//! - `panel`: a thin bar under the owner panel's header ([`panel_bar`]), at most
//!   [`MAX_PROGRESSES_PER_PANEL`].
//! - Cancel sends `progress-cancel-requested` once; the progress stays, its Cancel disabled, until
//!   the extension ends it.

use std::{sync::Arc, time::Duration};

use gpui::{
    AnyElement, App, Context, Entity, Global, IntoElement, Render, SharedString, Task, Window, div,
    prelude::*, px, relative,
};
use theme::ActiveTheme as _;
use ui::{
    ButtonCommon as _, ButtonLike, Clickable as _, Color, CommonAnimationExt as _, ContextMenu,
    Disableable as _, Icon, IconButton, IconName, IconSize, Label, LabelCommon as _, LabelSize,
    PopoverMenu, Tooltip, h_flex, v_flex,
};
use ui_ext::v0_2::{
    ProgressLocation, ProgressOptions, ProgressValue,
    notifications::{MAX_PROGRESS_REDRAWS_PER_SECOND, MAX_PROGRESSES_PER_PANEL},
};
use workspace::{HideStatusItem, ItemHandle, StatusItemView};

use crate::UiExtensionRegistry;

#[derive(Clone, Debug)]
pub struct RunningProgress {
    pub extension_id: Arc<str>,
    pub id: u64,
    pub options: ProgressOptions,
    pub value: ProgressValue,
    pub message: Option<String>,
    pub cancel_requested: bool,
}

#[derive(Default)]
pub struct Progresses {
    /// In start order: the last is the newest.
    running: Vec<RunningProgress>,
    redraw: Option<Task<()>>,
    dirty: bool,
}

struct GlobalProgresses(Entity<Progresses>);

impl Global for GlobalProgresses {}

pub(crate) fn init(cx: &mut App) {
    let progresses = cx.new(|_| Progresses::default());
    cx.set_global(GlobalProgresses(progresses));
}

impl Progresses {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalProgresses>()
            .map(|global| global.0.clone())
    }

    fn position(&self, extension_id: &str, id: u64) -> Option<usize> {
        self.running
            .iter()
            .position(|running| &*running.extension_id == extension_id && running.id == id)
    }

    pub(crate) fn start(
        &mut self,
        extension_id: &str,
        id: u64,
        options: ProgressOptions,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.position(extension_id, id) {
            self.running.remove(index);
        }
        self.running.push(RunningProgress {
            extension_id: extension_id.into(),
            id,
            options,
            value: ProgressValue::Indeterminate,
            message: None,
            cancel_requested: false,
        });
        self.changed(cx);
    }

    /// `message: None` keeps the previous message (M.2).
    pub(crate) fn update(
        &mut self,
        extension_id: &str,
        id: u64,
        value: ProgressValue,
        message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.position(extension_id, id) else {
            return;
        };
        let running = &mut self.running[index];
        running.value = value;
        if message.is_some() {
            running.message = message;
        }
        self.changed(cx);
    }

    pub(crate) fn end(&mut self, extension_id: &str, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.position(extension_id, id) {
            self.running.remove(index);
            self.changed(cx);
        }
    }

    /// Ends an extension's progresses (`owner: None`) or one owner's, without events.
    pub(crate) fn end_of(
        &mut self,
        extension_id: &str,
        owner: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let before = self.running.len();
        self.running.retain(|running| {
            !(&*running.extension_id == extension_id
                && owner.is_none_or(|owner| running.options.owner == owner))
        });
        if self.running.len() != before {
            self.changed(cx);
        }
    }

    /// Marks a cancellable progress as asked to cancel, once; its owner, to send the event to.
    pub(crate) fn request_cancel(
        &mut self,
        extension_id: &str,
        id: u64,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let index = self.position(extension_id, id)?;
        let running = &mut self.running[index];
        if !running.options.cancellable || running.cancel_requested {
            return None;
        }
        running.cancel_requested = true;
        let owner = running.options.owner.clone();
        self.changed(cx);
        Some(owner)
    }

    pub fn running(&self) -> &[RunningProgress] {
        &self.running
    }

    /// The extensions with a running status-bar progress, in the order they first started one,
    /// each with its progresses oldest first.
    pub fn status_bar(&self) -> Vec<(Arc<str>, Vec<&RunningProgress>)> {
        let mut groups: Vec<(Arc<str>, Vec<&RunningProgress>)> = Vec::new();
        for running in &self.running {
            if running.options.location != ProgressLocation::StatusBar {
                continue;
            }
            match groups
                .iter_mut()
                .find(|(extension_id, _)| *extension_id == running.extension_id)
            {
                Some((_, group)) => group.push(running),
                None => groups.push((running.extension_id.clone(), vec![running])),
            }
        }
        groups
    }

    /// The panel progresses of one owner panel, newest last, at most [`MAX_PROGRESSES_PER_PANEL`].
    pub fn on_panel(&self, extension_id: &str, owner: &str) -> Vec<&RunningProgress> {
        let on_panel: Vec<&RunningProgress> = self
            .running
            .iter()
            .filter(|running| {
                running.options.location == ProgressLocation::Panel
                    && &*running.extension_id == extension_id
                    && running.options.owner == owner
            })
            .collect();
        let skip = on_panel.len().saturating_sub(MAX_PROGRESSES_PER_PANEL);
        on_panel.into_iter().skip(skip).collect()
    }

    /// Notifies at once, then at most every 1/[`MAX_PROGRESS_REDRAWS_PER_SECOND`] s.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if self.redraw.is_some() {
            self.dirty = true;
            return;
        }
        cx.notify();
        let interval =
            Duration::from_millis(1000 / u64::from(MAX_PROGRESS_REDRAWS_PER_SECOND.max(1)));
        self.redraw = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(interval).await;
            this.update(cx, |this, cx| {
                this.redraw = None;
                if std::mem::take(&mut this.dirty) {
                    this.changed(cx);
                }
            })
            .ok();
        }));
    }
}

fn value_element(value: ProgressValue, id: impl Into<SharedString>) -> AnyElement {
    match value {
        ProgressValue::Indeterminate => Icon::new(IconName::LoadCircle)
            .size(IconSize::XSmall)
            .color(Color::Muted)
            .with_rotate_animation(2)
            .into_any_element(),
        ProgressValue::Percent(percent) => div()
            .id(id.into())
            .child(
                Label::new(format!("{percent}%"))
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .into_any_element(),
    }
}

fn describe(running: &RunningProgress) -> String {
    let value = match running.value {
        ProgressValue::Indeterminate => String::new(),
        ProgressValue::Percent(percent) => format!(" ({percent}%)"),
    };
    match &running.message {
        Some(message) => format!("{}{value}: {message}", running.options.title),
        None => format!("{}{value}", running.options.title),
    }
}

fn request_cancel(extension_id: Arc<str>, id: u64, cx: &mut App) {
    if let Some(registry) = UiExtensionRegistry::global(cx) {
        registry.update(cx, |registry, cx| {
            registry.request_progress_cancel(&extension_id, id, cx)
        });
    }
}

/// The status-bar chips of extension progress, one per extension (M.2). One per workspace.
pub struct ExtensionProgressStatus {
    progresses: Entity<Progresses>,
    _observe: gpui::Subscription,
}

impl ExtensionProgressStatus {
    pub fn new(progresses: Entity<Progresses>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&progresses, |_, _, cx| cx.notify()),
            progresses,
        }
    }
}

impl Render for ExtensionProgressStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let registry = UiExtensionRegistry::global(cx);
        let groups: Vec<(Arc<str>, Vec<RunningProgress>)> = self
            .progresses
            .read(cx)
            .status_bar()
            .into_iter()
            .map(|(extension_id, group)| (extension_id, group.into_iter().cloned().collect()))
            .collect();
        h_flex()
            .gap_2()
            .children(groups.into_iter().map(|(extension_id, group)| {
                let name = registry
                    .as_ref()
                    .map(|registry| registry.read(cx).provenance_name(&extension_id))
                    .unwrap_or_else(|| SharedString::from(extension_id.to_string()));
                let newest = group.last().cloned().expect("a group is never empty");
                let tooltip = group.iter().map(describe).collect::<Vec<_>>().join("\n");
                let chip = ButtonLike::new(SharedString::from(format!(
                    "fyx-ui-progress-{extension_id}"
                )))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(IconName::Blocks)
                                .size(IconSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(value_element(
                            newest.value,
                            format!("fyx-ui-progress-value-{extension_id}"),
                        ))
                        .child(Label::new(name.clone()).size(LabelSize::Small))
                        .child(
                            Label::new(newest.options.title)
                                .size(LabelSize::Small)
                                .color(Color::Muted)
                                .truncate(),
                        ),
                )
                .tooltip(Tooltip::text(format!("{name} · {extension_id}\n{tooltip}")));
                let menu_extension = extension_id.clone();
                PopoverMenu::new(SharedString::from(format!(
                    "fyx-ui-progress-menu-{extension_id}"
                )))
                .menu(move |window, cx| {
                    let extension_id = menu_extension.clone();
                    let group = group.clone();
                    Some(ContextMenu::build(window, cx, move |mut menu, _, _| {
                        for running in group {
                            let title = running.options.title.clone();
                            if running.cancel_requested {
                                menu = menu.label(format!("{title}: cancelling…"));
                            } else if running.options.cancellable {
                                let (extension_id, id) = (extension_id.clone(), running.id);
                                menu = menu.entry(format!("Cancel {title}"), None, move |_, cx| {
                                    request_cancel(extension_id.clone(), id, cx)
                                });
                            } else {
                                menu = menu.label(describe(&running));
                            }
                            let (extension_id, owner) =
                                (extension_id.clone(), running.options.owner.clone());
                            menu = menu.entry(format!("Show {owner}"), None, move |_, cx| {
                                if let Some(registry) = UiExtensionRegistry::global(cx) {
                                    registry.update(cx, |registry, cx| {
                                        registry.reveal_owner_panel(&extension_id, &owner, cx)
                                    });
                                }
                            });
                        }
                        menu
                    }))
                })
                .anchor(gpui::Anchor::BottomLeft)
                .trigger(chip)
            }))
    }
}

impl StatusItemView for ExtensionProgressStatus {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// Present only while an extension runs a progress; hiding it would hide who is working.
    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}

/// The bar under an extension panel's header with the progresses it owns (M.2): a fill for the
/// percentage, the title, the message on hover, and Cancel.
pub(crate) fn panel_bar(extension_id: &str, owner: &str, cx: &App) -> Option<AnyElement> {
    let progresses = Progresses::global(cx)?;
    let on_panel: Vec<RunningProgress> = progresses
        .read(cx)
        .on_panel(extension_id, owner)
        .into_iter()
        .cloned()
        .collect();
    if on_panel.is_empty() {
        return None;
    }
    let colors = cx.theme().colors();
    let (accent, track) = (colors.text_accent, colors.border_variant);
    Some(
        v_flex()
            .id("fyx-ui-panel-progress")
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(colors.border)
            .children(on_panel.into_iter().map(move |running| {
                let extension_id: Arc<str> = running.extension_id.clone();
                let id = running.id;
                let fill = match running.value {
                    ProgressValue::Percent(percent) => relative(f32::from(percent) / 100.),
                    ProgressValue::Indeterminate => relative(1.),
                };
                v_flex()
                    .id(("fyx-ui-panel-progress-row", id as usize))
                    .gap_0p5()
                    .when_some(running.message.clone(), |this, message| {
                        this.tooltip(Tooltip::text(message))
                    })
                    .child(
                        h_flex()
                            .gap_1()
                            .child(value_element(
                                running.value,
                                format!("fyx-ui-panel-progress-value-{id}"),
                            ))
                            .child(
                                Label::new(running.options.title.clone())
                                    .size(LabelSize::XSmall)
                                    .truncate(),
                            )
                            .when(running.options.cancellable, |this| {
                                this.child(
                                    IconButton::new(
                                        ("fyx-ui-progress-cancel", id as usize),
                                        IconName::Close,
                                    )
                                    .icon_size(IconSize::XSmall)
                                    .disabled(running.cancel_requested)
                                    .tooltip(Tooltip::text(if running.cancel_requested {
                                        "Cancelling…"
                                    } else {
                                        "Cancel"
                                    }))
                                    .on_click(
                                        move |_, _, cx| {
                                            request_cancel(extension_id.clone(), id, cx)
                                        },
                                    ),
                                )
                            }),
                    )
                    .child(
                        div().h(px(2.)).w_full().bg(track).child(
                            div()
                                .h_full()
                                .w(fill)
                                .bg(accent)
                                .when(running.value == ProgressValue::Indeterminate, |this| {
                                    this.opacity(0.4)
                                }),
                        ),
                    )
            }))
            .into_any_element(),
    )
}
