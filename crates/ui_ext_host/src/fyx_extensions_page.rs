//! The Fyx Extensions page (platform plan 3.5): UI extensions from the Fyx registry next to the
//! installed ones. Opened from the Extensions page ("Fyx Extensions", one `fyx:` hook) or with
//! `fyx_ui: open fyx extensions`.
//!
//! - **Search** over the verified index (name, id, description).
//! - **Install** the newest installable release: what it asks for is confirmed first (nothing to
//!   confirm for an all-T0 release), the package is verified against the signed hash, unpacked
//!   by the install store as a registry install, and loaded.
//! - **Installed:** version, origin ("Unsigned, not from the registry" for a file install), pin,
//!   roll back, uninstall; an update the plan names; a held update with its reasons, and
//!   "Approve and Update" when it waits for new permissions.
//! - [`RegistryHub`] holds the registry and what the page shows. It opens on first use, so nothing
//!   is read or fetched at startup; the page refreshes when it opens.

use anyhow::Result;
use editor::{Editor, EditorEvent};
use fs::Fs;
use fyx_registry::{
    Action, Freshness, HoldReason, InstalledExtension, Origin, Permissions, PlannedAction, Release,
    Unavailable,
};
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    PromptLevel, SharedString, Task, WeakEntity, Window, actions, div, prelude::*,
};
use settings::Settings as _;
use theme::ActiveTheme as _;
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Chip, Clickable as _, Color, Disableable as _,
    Headline, HeadlineSize, Label, LabelCommon as _, LabelSize, h_flex, v_flex,
};
use workspace::{
    Workspace,
    item::{Item, ItemEvent},
};

use crate::{
    UiExtensionRegistry,
    install::{FyxExtensionsSettings, capability_chip_label, describe_capability_ids},
    install_store::VersionSource,
    registry_service::{
        RefreshOutcome, Registry, RegistryLocation, RegistryStateDir, compiled_keyset, now,
    },
    store_actions::{
        Outcome, expect_load, grant_and_rescan, installed_outcome, load_outcome, show_outcome,
    },
};

actions!(
    fyx_ui,
    [
        /// Opens the Fyx Extensions page: UI extensions from the Fyx registry.
        OpenFyxExtensions,
    ]
);

/// The registry and what the page shows.
#[derive(Default)]
pub struct RegistryHub {
    registry: Option<Registry>,
    opened: bool,
    /// `fyx.extensions.registry_url` when it was opened.
    url: String,
    /// Tests keep the registry state out of the data dir.
    state_dir: Option<RegistryStateDir>,
    /// Why there is no registry (off, a bad URL, no trusted keys).
    off: Option<SharedString>,
    refreshing: bool,
    pub last_refresh: Option<RefreshOutcome>,
    /// The newest release of each extension in the index.
    pub listing: Vec<Release>,
    pub unavailable: Option<Unavailable>,
    pub freshness: Option<Freshness>,
    pub installed: Vec<InstalledExtension>,
    pub actions: Vec<PlannedAction>,
    _refresh: Option<Task<()>>,
}

struct GlobalRegistryHub(Entity<RegistryHub>);

impl Global for GlobalRegistryHub {}

pub(crate) fn init(cx: &mut App) {
    let hub = cx.new(|cx| {
        // A changed registry URL opens the registry again on next use.
        cx.observe_global::<settings::SettingsStore>(|hub: &mut RegistryHub, cx| {
            let url = FyxExtensionsSettings::get_global(cx).registry_url.clone();
            if hub.opened && hub.url != url {
                hub.reset();
                cx.notify();
            }
        })
        .detach();
        RegistryHub::default()
    });
    cx.set_global(GlobalRegistryHub(hub));
}

impl RegistryHub {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalRegistryHub>()
            .map(|global| global.0.clone())
    }

    fn reset(&mut self) {
        let state_dir = self.state_dir.take();
        *self = Self {
            state_dir,
            ..Self::default()
        };
    }

    #[cfg(test)]
    pub(crate) fn set_state_dir(&mut self, dir: RegistryStateDir) {
        self.state_dir = Some(dir);
    }

    /// Opens the registry from the settings on first use. No network.
    fn ensure_open(&mut self, cx: &App) -> Option<Registry> {
        if !self.opened {
            self.opened = true;
            let url = FyxExtensionsSettings::get_global(cx).registry_url.clone();
            self.url = url.clone();
            match RegistryLocation::parse(&url) {
                Ok(None) => {
                    self.off = Some("The registry is off (fyx.extensions.registry_url).".into())
                }
                Err(error) => {
                    self.off = Some(format!("fyx.extensions.registry_url: {error:#}").into())
                }
                Ok(Some(location)) => {
                    self.registry = Registry::open(
                        compiled_keyset(),
                        location,
                        cx.http_client(),
                        self.state_dir
                            .clone()
                            .unwrap_or_else(RegistryStateDir::default_dir),
                    );
                    if self.registry.is_none() {
                        self.off = Some(
                            "This build trusts no registry keys yet, so it cannot install from the registry."
                                .into(),
                        );
                    }
                }
            }
        }
        self.registry.clone()
    }

    pub fn off_reason(&self) -> Option<&SharedString> {
        self.off.as_ref()
    }

    pub fn is_refreshing(&self) -> bool {
        self.refreshing
    }

    pub fn registry(&mut self, cx: &App) -> Option<Registry> {
        self.ensure_open(cx)
    }

    /// Fetches the index (and the block list when due), then what the page shows.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        let Some(registry) = self.ensure_open(cx) else {
            cx.notify();
            return;
        };
        self.refreshing = true;
        cx.notify();
        let store = UiExtensionRegistry::global(cx)
            .and_then(|registry| registry.read(cx).install_store().cloned());
        let fs = <dyn Fs>::global(cx);
        self._refresh = Some(cx.spawn(async move |this, cx| {
            let work = async move {
                let outcome = registry.refresh(now()).await;
                let installed = match &store {
                    Some(store) => store
                        .installed_extensions(fs.as_ref())
                        .await
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                let client = registry.client().lock().await;
                let time = now();
                let (listing, unavailable) = match client.index(time) {
                    Ok(index) => (
                        index
                            .extensions()
                            .filter_map(|id| index.latest(id).cloned())
                            .collect(),
                        None,
                    ),
                    Err(unavailable) => (Vec::new(), Some(unavailable)),
                };
                let actions = client.plan_updates(&installed, time);
                let freshness = client.blocklist_freshness(time);
                (outcome, listing, unavailable, freshness, installed, actions)
            };
            let (outcome, listing, unavailable, freshness, installed, actions) =
                cx.background_spawn(work).await;
            this.update(cx, |this, cx| {
                this.refreshing = false;
                this.last_refresh = Some(outcome);
                this.listing = listing;
                this.unavailable = unavailable;
                this.freshness = Some(freshness);
                this.installed = installed;
                this.actions = actions;
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn installed(&self, id: &str) -> Option<&InstalledExtension> {
        self.installed
            .iter()
            .find(|installed| installed.id.as_str() == id)
    }

    pub fn action_for(&self, id: &str) -> Option<&Action> {
        self.actions
            .iter()
            .find(|action| action.id.as_str() == id)
            .map(|action| &action.action)
    }
}

/// What a confirmation lists: the capabilities, domains and programs a release adds.
fn describe_additions(additions: &fyx_registry::Additions) -> Vec<String> {
    let mut lines =
        describe_capability_ids(&additions.capabilities.iter().cloned().collect::<Vec<_>>());
    if additions.capabilities.is_empty() {
        lines.clear();
    }
    lines.extend(
        additions
            .network_domains
            .iter()
            .map(|domain| format!("Connects to {domain}")),
    );
    lines.extend(
        additions
            .processes
            .iter()
            .map(|program| format!("Runs {program}")),
    );
    if let Some(tier) = additions.tier_raised {
        lines.push(format!("Permission tier {tier:?}"));
    }
    lines
}

/// Installs `release` (a fresh install, an update, or an approved held update), after the user
/// confirmed what it asks for when it asks for anything new.
fn install_release(
    workspace: WeakEntity<Workspace>,
    release: Release,
    approved_before: Option<Permissions>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(hub) = RegistryHub::global(cx) else {
        return;
    };
    let Some(registry) = hub.update(cx, |hub, cx| hub.registry(cx)) else {
        return;
    };
    let Some(store) = UiExtensionRegistry::global(cx)
        .and_then(|registry| registry.read(cx).install_store().cloned())
    else {
        return;
    };
    let fs = <dyn Fs>::global(cx);
    window
        .spawn(cx, async move |cx: &mut AsyncWindowContext| {
            let result: Result<Option<Outcome>> = async {
                let base = approved_before.clone().unwrap_or_default();
                let additions = base.additions(&release);
                let asks = describe_additions(&additions);
                if !asks.is_empty() {
                    let detail = format!(
                        "It asks for:\n{}\n\nFrom the Fyx registry, published by {}.\nSource: {} at {}",
                        asks.join("\n"),
                        release.owners.iter().cloned().collect::<Vec<_>>().join(", "),
                        release.source.repository,
                        &release.source.commit[..release.source.commit.len().min(12)],
                    );
                    let answer = cx.update(|window, cx| {
                        window.prompt(
                            PromptLevel::Info,
                            &format!("Install {} {}?", release.name, release.version),
                            Some(&detail),
                            &["Install", "Cancel"],
                            cx,
                        )
                    })?;
                    if answer.await != Ok(0) {
                        return Ok(None);
                    }
                }
                let bytes = registry.download(&release).await?;
                let mut approved = base;
                approved.approve(&release);
                let installed = store
                    .install_package_bytes(
                        bytes,
                        &format!("{} {}", release.id, release.version),
                        VersionSource {
                            origin: Origin::Registry,
                            package_sha256: Some(release.package.sha256),
                        },
                        Some(approved),
                        fs.as_ref(),
                    )
                    .await?;
                let waiter = cx.update(|_, cx| expect_load(&installed.extension_id, cx))?;
                grant_and_rescan(&store, &installed.extension_id, fs.clone(), cx).await?;
                let loaded = load_outcome(waiter, cx).await;
                Ok(Some(installed_outcome(
                    format!("Installed {} {}", release.name, release.version),
                    loaded,
                )))
            }
            .await;
            let outcome = match result {
                Ok(Some(outcome)) => outcome,
                Ok(None) => return,
                Err(error) => Outcome {
                    message: format!("Could not install {}: {error:#}", release.id),
                    failed: true,
                },
            };
            workspace
                .update(cx, |workspace, cx| show_outcome(workspace, outcome, cx))
                .ok();
            cx.update(|_, cx| hub.update(cx, |hub, cx| hub.refresh(cx))).ok();
        })
        .detach();
}

/// Pin, roll back or uninstall, then rescan and refresh.
fn store_change(
    workspace: WeakEntity<Workspace>,
    id: String,
    change: StoreChange,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(store) = UiExtensionRegistry::global(cx)
        .and_then(|registry| registry.read(cx).install_store().cloned())
    else {
        return;
    };
    let fs = <dyn Fs>::global(cx);
    window
        .spawn(cx, async move |cx: &mut AsyncWindowContext| {
            let result: Result<String> = async {
                Ok(match change {
                    StoreChange::Pin(pinned) => {
                        store.set_pinned(&id, pinned, fs.as_ref()).await?;
                        if pinned {
                            format!("Pinned {id}")
                        } else {
                            format!("Unpinned {id}")
                        }
                    }
                    StoreChange::RollBack => {
                        let version = store.rollback(&id, fs.as_ref()).await?;
                        format!("Rolled {id} back to {version}")
                    }
                    StoreChange::Uninstall => {
                        store.uninstall(&id, fs.as_ref()).await?;
                        format!("Uninstalled {id}")
                    }
                })
            }
            .await;
            let outcome = match result {
                Ok(message) => Outcome {
                    message,
                    failed: false,
                },
                Err(error) => Outcome {
                    message: format!("{id}: {error:#}"),
                    failed: true,
                },
            };
            cx.update(|_, cx| {
                if let Some(registry) = UiExtensionRegistry::global(cx) {
                    registry.update(cx, |registry, cx| registry.rescan(cx));
                }
                if let Some(hub) = RegistryHub::global(cx) {
                    hub.update(cx, |hub, cx| hub.refresh(cx));
                }
            })
            .ok();
            workspace
                .update(cx, |workspace, cx| show_outcome(workspace, outcome, cx))
                .ok();
        })
        .detach();
}

#[derive(Clone, Copy)]
enum StoreChange {
    Pin(bool),
    RollBack,
    Uninstall,
}

pub struct FyxExtensionsPage {
    workspace: WeakEntity<Workspace>,
    query: Entity<Editor>,
    hub: Entity<RegistryHub>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl FyxExtensionsPage {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        hub: Entity<RegistryHub>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Search Fyx extensions…", window, cx);
            editor
        });
        let subscriptions = vec![
            cx.observe(&hub, |_, _, cx| cx.notify()),
            cx.subscribe(&query, |_, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    cx.notify();
                }
            }),
        ];
        hub.update(cx, |hub, cx| hub.refresh(cx));
        Self {
            workspace,
            query,
            hub,
            _subscriptions: subscriptions,
        }
    }

    /// Opens the page in the active pane, or shows the open one.
    pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
        let Some(hub) = RegistryHub::global(cx) else {
            return;
        };
        if let Some(page) = workspace.item_of_type::<FyxExtensionsPage>(cx) {
            workspace.activate_item(&page, true, true, window, cx);
            hub.update(cx, |hub, cx| hub.refresh(cx));
            return;
        }
        let weak = cx.entity().downgrade();
        let page = cx.new(|cx| FyxExtensionsPage::new(weak, hub, window, cx));
        workspace.add_item_to_active_pane(Box::new(page), None, true, window, cx);
    }

    fn matches(&self, release: &Release, cx: &App) -> bool {
        let query = self.query.read(cx).text(cx).to_lowercase();
        let query = query.trim();
        query.is_empty()
            || release.id.as_str().contains(query)
            || release.name.to_lowercase().contains(query)
            || release.description.to_lowercase().contains(query)
    }

    fn status_line(&self, cx: &App) -> Option<(String, Color)> {
        let hub = self.hub.read(cx);
        if let Some(off) = hub.off_reason() {
            return Some((off.to_string(), Color::Muted));
        }
        if hub.is_refreshing() {
            return Some(("Checking the registry…".into(), Color::Muted));
        }
        if let Some(unavailable) = &hub.unavailable {
            let text = match unavailable {
                Unavailable::NeverFetched => match &hub.last_refresh {
                    Some(RefreshOutcome {
                        index: fyx_registry::IndexRefresh::FetchFailed(error),
                        ..
                    }) => format!("Could not reach the registry: {error}"),
                    _ => "The registry has not been reached yet.".into(),
                },
                Unavailable::Expired { .. } => {
                    "The registry's index has expired; installs and updates wait for a fresh one."
                        .into()
                }
                Unavailable::Refused(error) => {
                    format!("The registry's index was refused, installs and updates stop: {error}")
                }
            };
            return Some((text, Color::Warning));
        }
        if let Some(Freshness::Stale { age }) = &hub.freshness {
            let days = age.as_secs() / (24 * 60 * 60);
            return Some((format!("Block list is {days} days old."), Color::Warning));
        }
        None
    }

    fn render_release(
        &self,
        release: &Release,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hub = self.hub.read(cx);
        let id = release.id.as_str().to_string();
        let installed = hub
            .installed(&id)
            .map(|installed| installed.active.version.clone());
        let workspace = self.workspace.clone();
        let button = match &installed {
            Some(version) if *version >= release.version => Button::new(
                SharedString::from(format!("fyx-reg-installed-{id}")),
                "Installed",
            )
            .disabled(true),
            Some(_) => Button::new(
                SharedString::from(format!("fyx-reg-update-{id}")),
                format!("Update to {}", release.version),
            ),
            None => Button::new(
                SharedString::from(format!("fyx-reg-install-{id}")),
                "Install",
            ),
        };
        let approved = hub
            .installed(&id)
            .map(|installed| installed.approved.clone());
        let release_for_click = release.clone();
        let _ = window;
        v_flex()
            .p_3()
            .gap_1()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(release.name.clone()))
                            .child(
                                Label::new(format!("{} · {}", release.id, release.version))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        button
                            .style(ButtonStyle::Filled)
                            .on_click(move |_, window, cx| {
                                install_release(
                                    workspace.clone(),
                                    release_for_click.clone(),
                                    approved.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            )
            .child(
                Label::new(release.description.clone())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Label::new(format!(
                            "by {}",
                            release
                                .owners
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                    )
                    .children(
                        release
                            .capabilities
                            .iter()
                            .map(|capability| Chip::new(capability_chip_label(capability))),
                    ),
            )
    }

    fn render_installed(
        &self,
        installed: &InstalledExtension,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hub = self.hub.read(cx);
        let id = installed.id.as_str().to_string();
        let action = hub.action_for(&id).cloned();
        let workspace = self.workspace.clone();
        let index_release = |version: &semver::Version| {
            hub.listing
                .iter()
                .find(|release| release.id.as_str() == id && &release.version == version)
                .cloned()
        };
        let mut notes: Vec<(String, Color)> = Vec::new();
        let mut update: Option<(Release, &'static str)> = None;
        match &action {
            Some(Action::Update { to, .. }) => {
                if let Some(release) = index_release(to) {
                    update = Some((release, "Update"));
                } else {
                    notes.push((format!("Update to {to} available"), Color::Accent));
                }
            }
            Some(Action::Hold { version, reasons }) => {
                for reason in reasons {
                    match reason {
                        HoldReason::Pinned => {
                            notes.push((format!("{version} available; pinned"), Color::Muted))
                        }
                        HoldReason::TooNew { .. } => notes.push((
                            format!("{version} available after the 24 h wait"),
                            Color::Muted,
                        )),
                        HoldReason::NeedsApproval(additions) => {
                            notes.push((
                                format!(
                                    "Update {version} needs approval: {}",
                                    describe_additions(additions).join(", ")
                                ),
                                Color::Warning,
                            ));
                            if let Some(release) = index_release(version) {
                                update = Some((release, "Approve and Update"));
                            }
                        }
                    }
                }
            }
            Some(Action::Rollback { to, block }) => notes.push((
                format!("Blocked ({}): rolls back to {to}", block.reason),
                Color::Error,
            )),
            Some(Action::Disable { block }) => notes.push((
                format!("Blocked ({}): disabled", block.reason),
                Color::Error,
            )),
            None => {}
        }
        let origin = match installed.active.origin {
            Origin::Registry => ("From the Fyx registry", Color::Muted),
            Origin::LocalFile => ("Unsigned, not from the registry", Color::Warning),
        };
        let pinned = installed.pinned;
        let has_previous = installed.previous.is_some();
        let approved = installed.approved.clone();
        let button = |label: &'static str, key: &str| {
            Button::new(SharedString::from(format!("fyx-reg-{key}-{id}")), label)
                .style(ButtonStyle::Subtle)
                .label_size(LabelSize::Small)
        };
        v_flex()
            .p_3()
            .gap_1()
            .border_1()
            .rounded_md()
            .border_color(cx.theme().colors().border_variant)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(id.clone()))
                            .child(
                                Label::new(installed.active.version.to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .child(Label::new(origin.0).size(LabelSize::XSmall).color(origin.1)),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .when_some(update, |this, (release, label)| {
                                let workspace = workspace.clone();
                                let approved = approved.clone();
                                this.child(
                                    Button::new(
                                        SharedString::from(format!("fyx-reg-up-{id}")),
                                        label,
                                    )
                                    .style(ButtonStyle::Filled)
                                    .label_size(LabelSize::Small)
                                    .on_click(
                                        move |_, window, cx| {
                                            install_release(
                                                workspace.clone(),
                                                release.clone(),
                                                Some(approved.clone()),
                                                window,
                                                cx,
                                            )
                                        },
                                    ),
                                )
                            })
                            .child({
                                let (workspace, id) = (workspace.clone(), id.clone());
                                button(if pinned { "Unpin" } else { "Pin" }, "pin").on_click(
                                    move |_, window, cx| {
                                        store_change(
                                            workspace.clone(),
                                            id.clone(),
                                            StoreChange::Pin(!pinned),
                                            window,
                                            cx,
                                        )
                                    },
                                )
                            })
                            .when(has_previous, |this| {
                                let (workspace, id) = (workspace.clone(), id.clone());
                                this.child(button("Roll Back", "rollback").on_click(
                                    move |_, window, cx| {
                                        store_change(
                                            workspace.clone(),
                                            id.clone(),
                                            StoreChange::RollBack,
                                            window,
                                            cx,
                                        )
                                    },
                                ))
                            })
                            .child({
                                let id = id.clone();
                                button("Uninstall", "uninstall").on_click(move |_, window, cx| {
                                    store_change(
                                        workspace.clone(),
                                        id.clone(),
                                        StoreChange::Uninstall,
                                        window,
                                        cx,
                                    )
                                })
                            }),
                    ),
            )
            .children(
                notes
                    .into_iter()
                    .map(|(note, color)| Label::new(note).size(LabelSize::Small).color(color)),
            )
    }
}

impl Render for FyxExtensionsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let listing: Vec<Release> = self
            .hub
            .read(cx)
            .listing
            .iter()
            .filter(|release| self.matches(release, cx))
            .cloned()
            .collect();
        let installed = self.hub.read(cx).installed.clone();
        let status = self.status_line(cx);
        let refreshing = self.hub.read(cx).is_refreshing();
        let hub = self.hub.clone();
        let releases: Vec<_> = listing
            .iter()
            .map(|release| self.render_release(release, window, cx).into_any_element())
            .collect();
        let installed_rows: Vec<_> = installed
            .iter()
            .map(|installed| self.render_installed(installed, cx).into_any_element())
            .collect();
        v_flex()
            .id("fyx-extensions-page")
            .size_full()
            .p_4()
            .gap_3()
            .overflow_y_scroll()
            .bg(cx.theme().colors().editor_background)
            .child(
                h_flex()
                    .justify_between()
                    .child(Headline::new("Fyx Extensions").size(HeadlineSize::Large))
                    .child(
                        Button::new("fyx-reg-refresh", "Refresh")
                            .style(ButtonStyle::Outlined)
                            .disabled(refreshing)
                            .on_click(move |_, _, cx| hub.update(cx, |hub, cx| hub.refresh(cx))),
                    ),
            )
            .when_some(status, |this, (status, color)| {
                this.child(Label::new(status).size(LabelSize::Small).color(color))
            })
            .child(
                div()
                    .h_8()
                    .px_2()
                    .py_1()
                    .border_1()
                    .rounded_md()
                    .border_color(cx.theme().colors().border)
                    .child(self.query.clone()),
            )
            .child(
                Label::new("Available")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(releases)
            .when(listing.is_empty(), |this| {
                this.child(
                    Label::new("Nothing to show.")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .child(
                Label::new("Installed")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .children(installed_rows)
    }
}

impl EventEmitter<ItemEvent> for FyxExtensionsPage {}

impl Focusable for FyxExtensionsPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.query.read(cx).focus_handle(cx)
    }
}

impl Item for FyxExtensionsPage {
    type Event = ItemEvent;

    fn tab_content_text(&self, _: usize, _: &App) -> SharedString {
        "Fyx Extensions".into()
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        f(*event)
    }
}

/// The Extensions page's "Fyx Extensions" button (its one `fyx:` hook).
pub fn fyx_extensions_button() -> impl IntoElement {
    Button::new("fyx-open-fyx-extensions", "Fyx Extensions")
        .style(ButtonStyle::Outlined)
        .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFyxExtensions), cx))
}

pub(crate) fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &OpenFyxExtensions, window, cx| {
        FyxExtensionsPage::open(workspace, window, cx)
    });
}
