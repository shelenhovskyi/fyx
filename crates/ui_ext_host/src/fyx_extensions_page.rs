//! The Fyx Extensions page (platform plan 3.5): UI extensions from the Fyx registry next to the
//! installed ones. Opened from the Extensions page ("Fyx Extensions", one `fyx:` hook) or with
//! `fyx_ui: open fyx extensions`.
//!
//! - **Search** over the verified index (name, id, description).
//! - **Install** the release the client's `plan_install` picks (the newest one that is not blocked
//!   and passed the 24 h wait; integration-5 R1): what it asks for is confirmed first (nothing to
//!   confirm for an all-T0 release), then `download_for_install` plans again, and the install
//!   stops if that picks another release. The package is verified against the signed hash,
//!   unpacked by the install store as a registry install, and loaded. A blocked extension shows
//!   "Blocked", one still in its wait "Available in …", both on a disabled button.
//! - **Checks** (decision 3): when the page opens or refreshes (block actions only: roll back,
//!   disable, enable again) and [`UPDATE_CHECK_INTERVAL`] after the last check (updates too). The
//!   last check's time is kept across restarts; nothing is checked at startup.
//! - **Installed:** version, origin ("Unsigned, not from the registry" for a file install), pin,
//!   roll back, uninstall; an update the plan names; a held update with its reasons, and
//!   "Approve and Update" when it waits for new permissions.
//! - [`RegistryHub`] holds the registry and what the page shows. It opens on first use, so nothing
//!   is read or fetched at startup; the page refreshes when it opens.

use anyhow::Result;
use editor::{Editor, EditorEvent};
use fs::Fs;
use fyx_registry::{
    Action, Freshness, HoldReason, InstallError, InstallRefusal, InstalledExtension, Origin,
    PlannedAction, Release, Timestamp, Unavailable,
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
    registry::LocalExtension,
    install::{FyxExtensionsSettings, capability_chip_label, describe_capability_ids},
    registry_service::{
        RefreshOutcome, Registry, RegistryLocation, RegistryStateDir, UPDATE_CHECK_INTERVAL,
        compiled_keyset, now,
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
        /// Checks the Fyx registry for extension updates now (the block list too) and applies
        /// them, as the 6 h check does.
        CheckForExtensionUpdates,
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
    /// When the last check started (`None`: not read from the state dir yet, or never).
    last_check: Option<Timestamp>,
    last_check_read: bool,
    pub last_refresh: Option<RefreshOutcome>,
    /// Each extension in the index, with what a fresh install would get.
    pub listing: Vec<Listed>,
    /// The releases the planned actions name (updates and held updates).
    pub releases: Vec<Release>,
    pub unavailable: Option<Unavailable>,
    pub freshness: Option<Freshness>,
    pub installed: Vec<InstalledExtension>,
    pub actions: Vec<PlannedAction>,
    /// Ids the store has disabled, with why.
    pub disabled: Vec<(String, String)>,
    /// What the last update check did, for the page.
    pub applied: Vec<String>,
    /// The next refresh fetches the block list even when it is not due.
    blocklist_now: bool,
    _refresh: Option<Task<()>>,
    _installed: Option<Task<()>>,
}

/// One extension of the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The release a fresh install gets, or the newest one when it cannot be installed.
    pub release: Release,
    /// Why a fresh install is refused (blocked, or still in its 24 h wait).
    pub refusal: Option<InstallRefusal>,
}

/// What a refresh may do by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    /// The block actions: roll back, disable, enable again (the page opening or refreshing).
    Blocks,
    /// The block actions and updates (the 6 h check).
    All,
}

/// The first timed check waits at least this long after start, so none runs at startup.
const STARTUP_GRACE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

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
        // The update check: 6 h after the last one (kept across restarts), never at startup,
        // only with a registry set.
        cx.spawn(async move |hub, cx| {
            let started = std::time::Instant::now();
            loop {
                let Ok(wait) = hub.update(cx, |hub, _| hub.next_check_in(started)) else {
                    break;
                };
                cx.background_executor().timer(wait).await;
                if hub
                    .update(cx, |hub, cx| {
                        if hub.check_due() {
                            hub.refresh_and_apply(Apply::All, cx)
                        }
                    })
                    .is_err()
                {
                    break;
                }
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

    fn state_dir(&self) -> RegistryStateDir {
        self.state_dir
            .clone()
            .unwrap_or_else(RegistryStateDir::default_dir)
    }

    /// When the last check started, read from the state dir once (a local file, no network).
    fn last_check(&mut self) -> Option<Timestamp> {
        if !self.last_check_read {
            self.last_check_read = true;
            self.last_check = self.state_dir().last_check();
        }
        self.last_check
    }

    pub(crate) fn check_due(&mut self) -> bool {
        self.last_check()
            .is_none_or(|last| now().0.saturating_sub(last.0) >= UPDATE_CHECK_INTERVAL.as_secs())
    }

    /// How long the timer waits: until 6 h after the last check, and at least until
    /// [`STARTUP_GRACE`] after start.
    pub(crate) fn next_check_in(&mut self, started: std::time::Instant) -> std::time::Duration {
        let since_last = self
            .last_check()
            .map(|last| std::time::Duration::from_secs(now().0.saturating_sub(last.0)));
        let until_due = since_last.map_or(std::time::Duration::ZERO, |since| {
            UPDATE_CHECK_INTERVAL.saturating_sub(since)
        });
        until_due
            .max(STARTUP_GRACE.saturating_sub(started.elapsed()))
            .max(std::time::Duration::from_secs(60))
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
                        self.state_dir(),
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

    /// Fetches the index (and the block list when due), applies the block actions at once
    /// (spec 19), then what the page shows.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_and_apply(Apply::Blocks, cx)
    }

    /// "Check for extension updates": the index and the block list now, then updates as the
    /// timer applies them.
    pub fn check_now(&mut self, cx: &mut Context<Self>) {
        self.blocklist_now = true;
        self.refresh_and_apply(Apply::All, cx)
    }

    /// Without a registry the store's installs are still listed (pin, roll back, uninstall).
    fn load_installed(&mut self, cx: &mut Context<Self>) {
        let Some(store) = UiExtensionRegistry::global(cx)
            .and_then(|registry| registry.read(cx).install_store().cloned())
        else {
            return;
        };
        let fs = <dyn Fs>::global(cx);
        self._installed = Some(cx.spawn(async move |this, cx| {
            let installed = store
                .installed_extensions(fs.as_ref())
                .await
                .unwrap_or_default();
            this.update(cx, |this, cx| {
                this.installed = installed;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Fetches the index (and the block list when due), then carries out the plan's automatic
    /// actions: a rollback of a blocked version, disabling a blocked version with nothing to roll
    /// back to (and enabling it when the block lifts), and with [`Apply::All`] the updates of
    /// registry installs. Held updates wait. Each one counts as a check for the 6 h timer.
    pub fn refresh_and_apply(&mut self, apply: Apply, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        let checked_at = now();
        self.last_check();
        self.last_check = Some(checked_at);
        let Some(registry) = self.ensure_open(cx) else {
            self.load_installed(cx);
            cx.notify();
            return;
        };
        let blocklist_now = std::mem::take(&mut self.blocklist_now);
        if let Err(error) = self.state_dir().set_last_check(checked_at) {
            log::warn!("fyx:ui: cannot store the registry check time: {error:#}");
        }
        self.refreshing = true;
        cx.notify();
        let store = UiExtensionRegistry::global(cx)
            .and_then(|registry| registry.read(cx).install_store().cloned());
        let fs = <dyn Fs>::global(cx);
        self._refresh = Some(cx.spawn(async move |this, cx| {
            let work = async move {
                let outcome = registry.refresh_with(now(), blocklist_now).await;
                let installed = match &store {
                    Some(store) => store
                        .installed_extensions(fs.as_ref())
                        .await
                        .unwrap_or_default(),
                    None => Vec::new(),
                };
                let client = registry.client().lock().await;
                let time = now();
                // The listing comes from the install plan, so what the page offers is what
                // `download_for_install` accepts (integration-5 R1).
                let (listing, unavailable) = match client.index(time) {
                    Ok(index) => (
                        index
                            .extensions()
                            .filter_map(|id| match client.plan_install(id, time) {
                                Ok(plan) => Some(Listed {
                                    release: plan.release.clone(),
                                    refusal: None,
                                }),
                                Err(InstallError::Refused(InstallRefusal::NotInIndex))
                                | Err(InstallError::Unavailable(_)) => None,
                                Err(InstallError::Refused(refusal)) => Some(Listed {
                                    release: index.latest(id)?.clone(),
                                    refusal: Some(refusal),
                                }),
                            })
                            .collect(),
                        None,
                    ),
                    Err(unavailable) => (Vec::new(), Some(unavailable)),
                };
                let actions = client.plan_updates(&installed, time);
                let freshness = client.blocklist_freshness(time);
                // The releases the actions name, while the index is at hand.
                let releases: Vec<Release> = match client.index(time) {
                    Ok(index) => actions
                        .iter()
                        .filter_map(|action| match &action.action {
                            Action::Update { to, .. } => index.release(&action.id, to).cloned(),
                            Action::Hold { version, .. } => {
                                index.release(&action.id, version).cloned()
                            }
                            _ => None,
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                };
                drop(client);
                let mut applied = PlanOutcome::default();
                let mut disabled = Vec::new();
                if let Some(store) = &store {
                    // Block actions on every refresh; updates only with `Apply::All`.
                    let automatic: Vec<PlannedAction> = actions
                        .iter()
                        .filter(|action| {
                            apply == Apply::All || !matches!(action.action, Action::Update { .. })
                        })
                        .cloned()
                        .collect();
                    applied =
                        apply_plan(&registry, store, fs.as_ref(), &automatic, &releases).await;
                    if let Ok(state) = store.state(fs.as_ref()).await {
                        disabled = state
                            .extensions
                            .iter()
                            .filter_map(|(id, entry)| Some((id.clone(), entry.disabled.clone()?)))
                            .collect();
                    }
                }
                (
                    outcome,
                    (listing, releases),
                    unavailable,
                    freshness,
                    installed,
                    actions,
                    applied,
                    disabled,
                )
            };
            let (
                outcome,
                (listing, releases),
                unavailable,
                freshness,
                installed,
                actions,
                applied,
                disabled,
            ) = cx.background_spawn(work).await;
            if !applied.lines.is_empty() {
                cx.update(|cx| {
                    if let Some(registry) = UiExtensionRegistry::global(cx) {
                        registry.update(cx, |registry, cx| {
                            // The name while the instance is still loaded.
                            for block in &applied.blocks {
                                let name = registry.provenance_name(&block.extension_id);
                                registry.notify_block(&name, block, cx);
                            }
                            registry.rescan(cx)
                        });
                    }
                });
            }
            this.update(cx, |this, cx| {
                this.refreshing = false;
                this.last_refresh = Some(outcome);
                this.listing = listing;
                this.releases = releases;
                this.unavailable = unavailable;
                this.freshness = Some(freshness);
                this.installed = installed;
                this.actions = actions;
                this.disabled = disabled;
                if !applied.lines.is_empty() || apply == Apply::All {
                    this.applied = applied.lines;
                }
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

    /// Every planned action for an id: a block can come with a held update (spec §4, failure
    /// 1: the extension is disabled, and the page offers "Approve and Update" or "Uninstall").
    pub fn actions_for<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Action> {
        self.actions
            .iter()
            .filter(move |action| action.id.as_str() == id)
            .map(|action| &action.action)
    }
}

/// What the user is told in the window when a block changes an installed extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockNotice {
    pub extension_id: String,
    /// "rolled back to 1.0.0" or "disabled".
    pub what: String,
    /// The block list's reason.
    pub reason: String,
}

/// What the update check did: one line per change (also logged), and the block changes to
/// show in the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanOutcome {
    pub lines: Vec<String>,
    pub blocks: Vec<BlockNotice>,
}

/// Carries out what the update check may do by itself.
pub(crate) async fn apply_plan(
    registry: &Registry,
    store: &crate::install_store::InstallStore,
    fs: &dyn Fs,
    actions: &[PlannedAction],
    updates: &[Release],
) -> PlanOutcome {
    let mut applied = Vec::new();
    let mut blocks = Vec::new();
    let mut note = |line: String| {
        log::warn!("fyx:ui: {line}");
        applied.push(line);
    };
    let disabled_now: Vec<String> = actions
        .iter()
        .filter(|action| matches!(action.action, Action::Disable { .. }))
        .map(|action| action.id.as_str().to_string())
        .collect();
    for action in actions {
        let id = action.id.as_str();
        match &action.action {
            Action::Update { to, .. } => {
                let Some(release) = updates
                    .iter()
                    .find(|release| release.id == action.id && &release.version == to)
                else {
                    continue;
                };
                let result = async {
                    let bytes = registry.download(release).await?;
                    store
                        .install_registry_package(bytes, release, None, fs)
                        .await
                }
                .await;
                match result {
                    Ok(_) => note(format!("updated {id} to {to}")),
                    Err(error) => note(format!("could not update {id} to {to}: {error:#}")),
                }
            }
            Action::Rollback { to, block } => match store.rollback(id, fs).await {
                Ok(_) => {
                    note(format!(
                        "rolled {id} back to {to}: blocked ({})",
                        block.reason
                    ));
                    blocks.push(BlockNotice {
                        extension_id: id.to_string(),
                        what: format!("rolled back to {to}"),
                        reason: block.reason.clone(),
                    });
                }
                Err(error) => note(format!("could not roll {id} back: {error:#}")),
            },
            Action::Disable { block } => {
                let reason = format!("blocked: {}", block.reason);
                match store.set_disabled(id, Some(reason.clone()), fs).await {
                    Ok(()) => {
                        note(format!("disabled {id}: {reason}"));
                        blocks.push(BlockNotice {
                            extension_id: id.to_string(),
                            what: "disabled".into(),
                            reason: block.reason.clone(),
                        });
                    }
                    Err(error) => note(format!("could not disable {id}: {error:#}")),
                }
            }
            Action::Hold { .. } => {}
        }
    }
    // A block that lifted enables the extension again.
    if let Ok(state) = store.state(fs).await {
        for (id, entry) in state.extensions {
            if entry.disabled.is_some() && !disabled_now.contains(&id) {
                if store.set_disabled(&id, None, fs).await.is_ok() {
                    note(format!("enabled {id} again: no longer blocked"));
                }
            }
        }
    }
    PlanOutcome {
        lines: applied,
        blocks,
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

/// What [`install_release`] installs.
#[derive(Clone)]
enum InstallMode {
    /// A fresh install of the release the listing's plan picked.
    Fresh,
    /// An update (or an approved held update) the plan named for an installed extension.
    Update(InstalledExtension),
}

fn format_wait(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60);
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h", minutes.div_ceil(60))
    }
}

/// Installs `release` (a fresh install, an update, or an approved held update), after the user
/// confirmed what it asks for when it asks for anything new. The download plans again: a fresh
/// install must get the same release (`download_for_install`), an update must still be planned.
fn install_release(
    workspace: WeakEntity<Workspace>,
    release: Release,
    mode: InstallMode,
    window: &mut Window,
    cx: &mut App,
) {
    let approved_before = match &mode {
        InstallMode::Fresh => None,
        InstallMode::Update(installed) => Some(installed.approved.clone()),
    };
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
                let bytes = match &mode {
                    InstallMode::Fresh => {
                        let (planned, bytes) =
                            registry.download_for_install(&release.id, now()).await?;
                        anyhow::ensure!(
                            planned.version == release.version
                                && planned.package.sha256 == release.package.sha256,
                            "the registry changed since the page showed {} {}; refresh and try again",
                            release.id,
                            release.version
                        );
                        bytes
                    }
                    InstallMode::Update(installed) => {
                        registry
                            .download_planned_update(installed, &release, now())
                            .await?
                    }
                };
                let mut approved = base;
                approved.approve(&release);
                let installed = store
                    .install_registry_package(bytes, &release, Some(approved), fs.as_ref())
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
/// Why the version a rollback would bring back may not run: the block list names it
/// (integration-5 R9). `None` without a registry or a block list.
pub(crate) async fn rollback_blocked(
    store: &crate::install_store::InstallStore,
    id: &str,
    fs: &dyn Fs,
    cx: &mut gpui::AsyncApp,
) -> Option<String> {
    let registry = cx.update(|cx| {
        let hub = RegistryHub::global(cx)?;
        hub.update(cx, |hub, cx| hub.registry(cx))
    })?;
    let installed = store
        .installed_extensions(fs)
        .await
        .ok()?
        .into_iter()
        .find(|installed| installed.id.as_str() == id)?;
    let previous = installed.previous?;
    let client = registry.client().lock().await;
    client
        .blocklist()?
        .block_for(
            &installed.id,
            &previous.version,
            previous.package_sha256.as_ref(),
        )
        .map(|block| format!("{id} {} is blocked: {}", previous.version, block.reason))
}

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
                        if let Some(blocked) = rollback_blocked(&store, &id, fs.as_ref(), cx).await
                        {
                            anyhow::bail!("{blocked}");
                        }
                        let version = store.rollback(&id, fs.as_ref()).await?;
                        format!("Rolled {id} back to {version}")
                    }
                    StoreChange::Uninstall => {
                        store.uninstall(&id, fs.as_ref()).await?;
                        let fs = fs.clone();
                        let forgotten = id.clone();
                        cx.update(|_, cx| {
                            if let Some(registry) = UiExtensionRegistry::global(cx) {
                                registry.update(cx, |registry, cx| {
                                    registry.forget_consent(&forgotten, fs, cx)
                                });
                            }
                        })?;
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
        let mut subscriptions = vec![
            cx.observe(&hub, |_, _, cx| cx.notify()),
            cx.subscribe(&query, |_, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    cx.notify();
                }
            }),
        ];
        // Dev extensions and permission rows follow the loaded extensions.
        if let Some(registry) = UiExtensionRegistry::global(cx) {
            subscriptions.push(cx.observe(&registry, |_, _, cx| cx.notify()));
        }
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
        let installed = hub.installed(&id).is_some();
        let refusal = hub
            .listing
            .iter()
            .find(|listed| listed.release.id == release.id)
            .and_then(|listed| listed.refusal.clone());
        let workspace = self.workspace.clone();
        // Updates are on the installed row, where the plan decides them.
        let (button, note) = match (&refusal, installed) {
            (_, true) => (
                Button::new(
                    SharedString::from(format!("fyx-reg-installed-{id}")),
                    "Installed",
                )
                .disabled(true),
                None,
            ),
            (Some(InstallRefusal::Blocked { block }), false) => (
                Button::new(
                    SharedString::from(format!("fyx-reg-blocked-{id}")),
                    "Blocked",
                )
                .disabled(true),
                Some((format!("Blocked: {}", block.reason), Color::Error)),
            ),
            (Some(InstallRefusal::TooNew { ready_at, .. }), false) => (
                Button::new(
                    SharedString::from(format!("fyx-reg-waiting-{id}")),
                    format!(
                        "Available in {}",
                        format_wait(ready_at.0.saturating_sub(now().0))
                    ),
                )
                .disabled(true),
                None,
            ),
            (Some(InstallRefusal::NotInIndex), false) | (None, false) => (
                Button::new(
                    SharedString::from(format!("fyx-reg-install-{id}")),
                    "Install",
                )
                .disabled(refusal.is_some()),
                None,
            ),
        };
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
                                    InstallMode::Fresh,
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
            .when_some(note, |this, (note, color)| {
                this.child(Label::new(note).size(LabelSize::Small).color(color))
            })
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
        let actions: Vec<Action> = hub.actions_for(&id).cloned().collect();
        let workspace = self.workspace.clone();
        let index_release = |version: &semver::Version| {
            hub.releases
                .iter()
                .find(|release| release.id.as_str() == id && &release.version == version)
                .cloned()
        };
        let mut notes: Vec<(String, Color)> = hub
            .disabled
            .iter()
            .filter(|(disabled, _)| *disabled == id)
            .map(|(_, reason)| (format!("Disabled, {reason}"), Color::Error))
            .collect();
        let mut update: Option<(Release, &'static str)> = None;
        for action in &actions {
            match action {
                Action::Update { to, .. } => {
                    if let Some(release) = index_release(to) {
                        update = Some((release, "Update"));
                    } else {
                        notes.push((format!("Update to {to} available"), Color::Accent));
                    }
                }
                Action::Hold { version, reasons } => {
                    // Approving helps only when nothing else holds the update.
                    let only_approval = reasons
                        .iter()
                        .all(|reason| matches!(reason, HoldReason::NeedsApproval(_)));
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
                                if only_approval && let Some(release) = index_release(version) {
                                    update = Some((release, "Approve and Update"));
                                }
                            }
                        }
                    }
                }
                Action::Rollback { to, block } => notes.push((
                    format!("Blocked ({}): rolls back to {to}", block.reason),
                    Color::Error,
                )),
                Action::Disable { block } => notes.push((
                    format!("Blocked ({}): disabled", block.reason),
                    Color::Error,
                )),
            }
        }
        let origin = match installed.active.origin {
            Origin::Registry => ("From the Fyx registry", Color::Muted),
            Origin::LocalFile => ("Unsigned, not from the registry", Color::Warning),
        };
        let pinned = installed.pinned;
        let has_previous = installed.previous.is_some();
        let installed_for_update = installed.clone();
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
                                let installed = installed_for_update.clone();
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
                                                InstallMode::Update(installed.clone()),
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
            .children(permission_rows(&id, cx))
    }

    /// A dev extension or an installed folder (not the install store): where it is, "Enable"
    /// when it waits for it, and its permissions.
    fn render_local(&self, local: &LocalExtension, cx: &mut Context<Self>) -> impl IntoElement {
        let id = local.extension_id.clone();
        let origin = if local.is_dev {
            "Dev extension, not from the registry"
        } else {
            "Installed folder, not from the registry"
        };
        let pending = local.pending.clone();
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
                        h_flex().gap_2().child(Label::new(id.clone())).child(
                            Label::new(origin)
                                .size(LabelSize::XSmall)
                                .color(Color::Warning),
                        ),
                    )
                    .when(pending.is_some(), |this| {
                        let id = id.clone();
                        this.child(
                            Button::new(
                                SharedString::from(format!("fyx-local-enable-{id}")),
                                "Enable",
                            )
                            .style(ButtonStyle::Filled)
                            .label_size(LabelSize::Small)
                            .on_click(move |_, _, cx| {
                                let fs = <dyn Fs>::global(cx);
                                if let Some(registry) = UiExtensionRegistry::global(cx) {
                                    registry.update(cx, |registry, cx| registry.enable(&id, fs, cx));
                                }
                            }),
                        )
                    }),
            )
            .child(
                Label::new(local.path.display().to_string())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .when_some(pending, |this, new| {
                let note = if new.is_empty() {
                    "Not enabled".to_string()
                } else {
                    format!(
                        "Its update waits for Enable: {}",
                        describe_capability_ids(&new).join(", ")
                    )
                };
                this.child(Label::new(note).size(LabelSize::Small).color(Color::Warning))
            })
            .children(permission_rows(&id, cx))
    }
}

/// The T1 and T2 capabilities an extension declared, with their state and the one way to change
/// it: Revoke when allowed; for T2, Allow when refused (plan 4.5; only this page changes a
/// refusal back). A revoked T1 capability comes back with "Enable" on the next load.
fn permission_rows(extension_id: &str, cx: &App) -> Vec<gpui::AnyElement> {
    use ui_ext_wasm::permissions::{PermissionState, Tier};
    let Some(registry) = UiExtensionRegistry::global(cx) else {
        return Vec::new();
    };
    let registry_read = registry.read(cx);
    registry_read
        .declared_capabilities(extension_id)
        .into_iter()
        .filter(|(_, tier, _)| *tier != Tier::T0)
        .map(|(capability, tier, granted)| {
            let label = capability_chip_label(&capability);
            let (text, action) = match (tier, granted) {
                (Tier::T1, true) => (format!("{label}: allowed"), Some("Revoke")),
                (Tier::T1, false) => (format!("{label}: not allowed until Enable"), None),
                _ => match registry_read.t2_state(extension_id, &capability) {
                    PermissionState::Granted => (format!("{label}: allowed"), Some("Revoke")),
                    PermissionState::Denied => (format!("{label}: not allowed"), Some("Allow")),
                    PermissionState::Ask => (format!("{label}: asks the first time"), None),
                    PermissionState::Unavailable => (format!("{label}: not available"), None),
                },
            };
            let (id, registry) = (extension_id.to_string(), registry.clone());
            h_flex()
                .gap_2()
                .child(Label::new(text).size(LabelSize::Small).color(Color::Muted))
                .when_some(action, |this, action| {
                    this.child(
                        Button::new(
                            SharedString::from(format!("fyx-t2-{id}-{capability}")),
                            action,
                        )
                        .style(ButtonStyle::Subtle)
                        .label_size(LabelSize::Small)
                        .on_click(move |_, _, cx| {
                            let fs = <dyn Fs>::global(cx);
                            registry.update(cx, |registry, cx| {
                                if action == "Allow" {
                                    registry.allow_t2(&id, &capability, cx);
                                } else if let Err(error) = registry.revoke(&id, &capability, fs, cx)
                                {
                                    log::error!("fyx:ui: {error:#}");
                                }
                            })
                        }),
                    )
                })
                .into_any_element()
        })
        .collect()
}

impl Render for FyxExtensionsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let listing: Vec<Release> = self
            .hub
            .read(cx)
            .listing
            .iter()
            .map(|listed| &listed.release)
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
        let mut installed_rows: Vec<_> = installed
            .iter()
            .map(|installed| self.render_installed(installed, cx).into_any_element())
            .collect();
        let local: Vec<LocalExtension> = UiExtensionRegistry::global(cx)
            .map(|registry| registry.read(cx).local_extensions())
            .unwrap_or_default()
            .into_iter()
            .filter(|local| {
                !installed
                    .iter()
                    .any(|installed| installed.id.as_str() == local.extension_id)
            })
            .collect();
        installed_rows.extend(
            local
                .iter()
                .map(|local| self.render_local(local, cx).into_any_element()),
        );
        let nothing_installed = installed_rows.is_empty();
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
            .children(self.hub.read(cx).applied.iter().map(|line| {
                Label::new(format!("Update check: {line}"))
                    .size(LabelSize::Small)
                    .color(Color::Muted)
            }))
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
            .when(nothing_installed, |this| {
                this.child(
                    Label::new("Nothing installed.")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
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
    // The check runs first, so the page that opens shows it (its own refresh waits for it).
    workspace.register_action(|workspace, _: &CheckForExtensionUpdates, window, cx| {
        if let Some(hub) = RegistryHub::global(cx) {
            hub.update(cx, |hub, cx| hub.check_now(cx));
        }
        FyxExtensionsPage::open(workspace, window, cx)
    });
}
