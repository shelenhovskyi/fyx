//! Loaded UI extensions and their surfaces, and how they reach every workspace.
//!
//! Panels are registered dynamically: when an extension finishes loading, each open workspace
//! gets a panel (or status item) per surface, and new workspaces get them on creation. Nothing
//! is listed statically in `crates/zed`.
//!
//! Extensions come from the installed-extensions directory (`ExtensionStore::installed_dir`):
//! every directory there whose `extension.toml` has a `[fyx_ui]` table is loaded with
//! `UiExtension::load_from_dir`, including dev extensions, which Zed's "Install Dev Extension"
//! links into that directory. The directory is scanned again when the extension store reports an
//! install, an uninstall or an update: a new or changed component (by modification time and size)
//! is loaded and its surfaces move to the new instance; a removed one stops its surfaces.
//! `FYX_UI_DEV_EXTENSIONS` adds paths outside that directory.
//!
//! An installed extension loads only with the user's consent to its current form (see
//! `consent`): one that has none waits in `pending_consent`, and every workspace offers an
//! "Enable" notification for it.
//!
//! Dev extensions (installed as links, or from `FYX_UI_DEV_EXTENSIONS`) reload by themselves:
//! their component file is watched, and a change, once writes have paused for
//! `HOT_RELOAD_DEBOUNCE`, loads it again and remounts its surfaces.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use anyhow::Context as _;
use collections::{HashMap, HashSet};
use extension_host::ExtensionStore;
use fs::{Fs, MTime, PathEvent};
use futures::{FutureExt as _, StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, Global, SharedString, Task, WeakEntity, Window,
};
use settings::Settings as _;
use ui_ext::v0_2::{
    NotificationActionEvent, NotificationCloseReason, StatusBarSide, Surface, SurfaceKind, UiEvent,
    Update,
};
use ui_ext_wasm::{
    CAPABILITY_PANEL, CAPABILITY_STATUS_BAR, LoadOptions, PrecompiledCache, UiExtension,
    UiManifest,
    permissions::{Tier, tier_of},
};
use workspace::{
    Workspace, WorkspaceSettings,
    notifications::{NotificationId, simple_message_notification::MessageNotification},
};

use crate::{
    ToggleExtensionPanel,
    backend::HostEffect,
    backend::UiBackend,
    consent::{self, ConsentRecord, ConsentState, ConsentStore},
    decorations::Decorations,
    install_store::InstallStore,
    modal::ExtensionModal,
    panel::{
        AnyExtensionPanel, ExtensionStatusGroup, ExtensionStatusItem, SurfaceInfo,
        add_extension_panel,
    },
    progress::{ExtensionProgressStatus, Progresses},
    surface::{SurfaceLink, SurfaceState},
};

/// Opt-in for the sprint: a list of `fyx:ui` extensions to load at startup, separated like
/// `PATH` (`;` on Windows, `:` elsewhere). Each entry is an extension directory (an
/// `extension.toml` with a `[fyx_ui]` table plus the component it names, loaded with
/// `UiExtension::load_from_dir`), or a bare `.wasm` component whose file name becomes its id.
pub const DEV_EXTENSIONS_ENV: &str = "FYX_UI_DEV_EXTENSIONS";

/// A dev extension's component is loaded again once writes to it have paused this long (a build
/// writes the file in several steps).
pub const HOT_RELOAD_DEBOUNCE: Duration = Duration::from_millis(300);

/// Extension panels sort after every built-in panel (those use 0..=7).
const FIRST_ACTIVATION_PRIORITY: u32 = 100;

pub struct LoadedSurface {
    /// The surface's structured key; `info.key` is only its display and persistence form.
    pub extension_id: Arc<str>,
    pub surface_id: Arc<str>,
    pub info: Rc<SurfaceInfo>,
    pub state: Entity<SurfaceState>,
}

/// Where an extension was loaded from. An extension id belongs to the first source that loads it:
/// a reinstall from the same source reloads it, another source with the same id is refused, so
/// one extension cannot take over another's surfaces by declaring its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtensionSource {
    /// A directory (installed, or a dev path).
    Directory(PathBuf),
    /// A bare component file (`FYX_UI_DEV_EXTENSIONS`).
    File(PathBuf),
    /// Fyx's install store (`install_store`), whichever version is active.
    Store,
}

#[derive(Default)]
pub struct UiExtensionRegistry {
    surfaces: Vec<LoadedSurface>,
    next_activation_priority: u32,
    /// Extensions loaded from the installed-extensions directory, by id.
    installed: HashMap<Arc<str>, Fingerprint>,
    scanning: bool,
    rescan_requested: bool,
    /// The component watch of each dev extension, by id.
    watches: HashMap<Arc<str>, Task<()>>,
    /// The source each loaded extension id belongs to.
    sources: HashMap<Arc<str>, ExtensionSource>,
    consent: ConsentStore,
    /// Installed extensions not loaded because the user has not enabled them as they are now.
    pending_consent: Vec<PendingConsent>,
    /// Tests only: load installed extensions without consent.
    trust_installed: bool,
    /// Fyx's install store, scanned with the installed-extensions directory.
    store: Option<InstallStore>,
    /// The directory and file system of the last scan, for `rescan`.
    last_scan: Option<(PathBuf, Arc<dyn Fs>)>,
    /// `fyx.extensions.dev_paths`: folders scanned with the installed ones, as dev extensions.
    dev_paths: Vec<PathBuf>,
    /// The instance of each registered extension, for surfaces it adds later (`add-surface`).
    backends: HashMap<Arc<str>, Rc<dyn UiBackend>>,
    /// Per extension: the token bucket and CPU share of its surfaces' ticks (design.md, B).
    tick_budgets: HashMap<Arc<str>, ui_ext_wasm::ticks::TickBudget>,
    /// The workspace whose window was active last: where `show-surface` opens a modal.
    active_workspace: Option<gpui::EntityId>,
    /// Install commands waiting for the outcome of the next load of an id (triage-3 #7).
    load_waiters: HashMap<Arc<str>, Vec<futures::channel::oneshot::Sender<Result<(), String>>>>,
    /// Open extension notifications, by extension and the id its instance gave them.
    notifications: HashMap<(Arc<str>, u64), OpenNotification>,
    /// The last reload of each dev path. A new one replaces (drops) the one in flight, so two
    /// loads of one extension never race and the newest component wins (triage-3, ASYNC 9).
    dev_reloads: HashMap<PathBuf, Task<()>>,
}

/// Requests for the workspaces that show a surface (design.md, C and E).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryEvent {
    /// `show-surface` of a panel: open its dock and make it the active panel, without moving
    /// focus.
    RevealPanel(SharedString),
    /// `hide-surface` of a panel: close its dock if it is the dock's active panel.
    HidePanel(SharedString),
    /// `show-surface` of a modal: the workspace named opens it (the active window's).
    ShowModal {
        key: SharedString,
        workspace: gpui::EntityId,
    },
    /// The extension closed a modal (`hide-surface`, `remove-surface`): no `dismissed`.
    CloseModal(SharedString),
    /// A message from Fyx itself (no provenance line) for the workspace named: a block list
    /// entry rolled back or disabled an installed extension (platform plan 3.4).
    HostNotice {
        id: SharedString,
        title: SharedString,
        message: SharedString,
        workspace: gpui::EntityId,
    },
    /// `notify` (design.md M.1): the workspace named shows it (the active window's).
    ShowNotification {
        extension_id: Arc<str>,
        id: u64,
        workspace: gpui::EntityId,
    },
}

/// An extension notification from `notify` until it closes (design.md M.1).
struct OpenNotification {
    /// The surface or decoration that gets its events.
    owner: String,
    /// Until the workspace takes it to draw.
    pending: Option<ui_ext::v0_2::Notification>,
    view: Option<WeakEntity<crate::notifications::ExtensionNotification>>,
}

impl gpui::EventEmitter<RegistryEvent> for UiExtensionRegistry {}

/// An installed extension waiting for the user's "Enable".
#[derive(Clone, Debug)]
pub struct PendingConsent {
    pub extension_id: Arc<str>,
    /// The folder in the installed-extensions directory.
    pub dir: PathBuf,
    pub installed_dir: PathBuf,
    /// As `extension.toml` declares them.
    pub capabilities: Vec<String>,
    pub is_dev: bool,
    /// Not empty for a held update: an extension enabled before that now asks for these T1
    /// capabilities as well (its old version keeps running, if loaded).
    pub new_capabilities: Vec<String>,
    record: ConsentRecord,
}

/// How a watched dev extension is loaded again.
#[derive(Clone, Debug)]
enum Reload {
    /// Linked into this installed-extensions directory: rescan it.
    Installed(PathBuf),
    /// A `FYX_UI_DEV_EXTENSIONS` entry: load it again.
    DevPath(PathBuf),
}

/// What tells a reinstalled component from an unchanged one: its folder (a store update moves
/// to a new version folder) and the component's modification time and size.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    dir: PathBuf,
    mtime: MTime,
    len: u64,
}

/// A UI extension found in the installed-extensions directory or the install store.
struct Found {
    dir: PathBuf,
    extension_id: Arc<str>,
    fingerprint: Fingerprint,
    component: PathBuf,
    /// A link (Zed's dev install): its component is watched for changes.
    is_dev: bool,
    capabilities: Vec<String>,
    source: ExtensionSource,
    /// For store installs: where the precompiled component goes.
    cache: Option<PrecompiledCache>,
    /// The manifest as read by this scan; the load uses this one, not a second read.
    manifest: UiManifest,
}

struct GlobalRegistry(Entity<UiExtensionRegistry>);

impl Global for GlobalRegistry {}

impl UiExtensionRegistry {
    /// Consent is kept in `consent_path`.
    pub fn new(consent_path: PathBuf) -> Self {
        Self {
            consent: ConsentStore::at(consent_path),
            ..Self::default()
        }
    }

    /// Also loads the active versions of `store`'s extensions.
    pub fn with_store(mut self, store: InstallStore) -> Self {
        self.store = Some(store);
        self
    }

    pub fn install_store(&self) -> Option<&InstallStore> {
        self.store.as_ref()
    }

    /// Follows `fyx.extensions.dev_paths`: a new folder loads, a removed one stops.
    pub fn set_dev_paths(&mut self, dev_paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.dev_paths != dev_paths {
            self.dev_paths = dev_paths;
            self.rescan(cx);
        }
    }

    /// Loads an extension again on a fresh instance, from where it came from; its surfaces
    /// remount on it (Restart extension, Retry after a failed reload).
    pub fn restart_extension(&mut self, extension_id: &str, cx: &mut Context<Self>) {
        log::info!("fyx:ui: restarting {extension_id}");
        // Installed, store and `dev_paths` extensions: forget the fingerprint, so the scan
        // loads it again even though nothing changed on disk.
        if self.installed.remove(extension_id).is_some() {
            self.rescan(cx);
            return;
        }
        match self.sources.get(extension_id).cloned() {
            Some(ExtensionSource::Directory(path) | ExtensionSource::File(path)) => {
                let fs = match &self.last_scan {
                    Some((_, fs)) => fs.clone(),
                    None => <dyn Fs>::global(cx),
                };
                self.reload(Reload::DevPath(path), fs, cx)
            }
            Some(ExtensionSource::Store) | None => {
                log::error!("fyx:ui: cannot restart {extension_id}: unknown source")
            }
        }
    }

    /// A reload of a loaded extension failed: its surfaces keep the old instance and show the
    /// error with Retry.
    fn reload_failed(&mut self, extension_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message: SharedString = format!("Reload failed: {error:#}").into();
        for surface in self.surfaces_of(extension_id) {
            surface
                .state
                .update(cx, |state, cx| state.show_reload_error(message.clone(), cx));
        }
    }

    /// The capabilities a loaded extension declared, with their tier and whether each is granted
    /// (T0 always is), for the chips and the revoke command.
    pub fn declared_capabilities(&self, extension_id: &str) -> Vec<(String, Tier, bool)> {
        self.consent
            .declared(extension_id)
            .into_iter()
            .filter_map(|capability| {
                let tier = tier_of(&capability)?;
                let granted = self.consent.is_granted(extension_id, &capability);
                Some((capability, tier, granted))
            })
            .collect()
    }

    /// Granted capabilities that can be taken back, as (extension id, capability).
    pub fn revocable(&self) -> Vec<(String, String)> {
        self.consent.revocable()
    }

    /// Takes a granted capability back: host imports are denied it from the next call, and the
    /// extension asks again on its next load.
    pub fn revoke(
        &mut self,
        extension_id: &str,
        capability: &str,
        fs: Arc<dyn Fs>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if let Some((path, text)) = self.consent.revoke(extension_id, capability)? {
            cx.background_spawn(async move { consent::save(path, text, fs.as_ref()).await })
                .detach();
        }
        log::info!("fyx:ui: revoked {capability} from {extension_id}");
        cx.notify();
        Ok(())
    }

    /// The broker's check, as host imports get it.
    pub fn capability_check(&self) -> Arc<dyn ui_ext_wasm::permissions::CapabilityCheck> {
        self.consent.checker()
    }

    /// Scans again what the last scan did (after an install-store change).
    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        if let Some((dir, fs)) = self.last_scan.clone() {
            self.scan_installed(dir, fs, cx);
        }
    }

    /// Loads installed extensions without asking: for tests that are not about consent.
    #[cfg(test)]
    pub(crate) fn trusting() -> Self {
        Self {
            trust_installed: true,
            ..Self::default()
        }
    }

    pub fn pending_consent(&self) -> &[PendingConsent] {
        &self.pending_consent
    }

    /// Records the user's consent to an extension as it is now and loads it.
    pub fn enable(&mut self, extension_id: &str, fs: Arc<dyn Fs>, cx: &mut Context<Self>) {
        let Some(index) = self
            .pending_consent
            .iter()
            .position(|pending| pending.extension_id.as_ref() == extension_id)
        else {
            return;
        };
        let pending = self.pending_consent.remove(index);
        log::info!(
            "fyx:ui: {extension_id} enabled with {:?}",
            pending.capabilities
        );
        self.grant_consent(extension_id, pending.record, fs.clone(), cx);
        self.scan_installed(pending.installed_dir, fs, cx);
        cx.notify();
    }

    /// Records consent the user gave elsewhere (the install confirmation).
    pub fn grant_consent(
        &mut self,
        extension_id: &str,
        record: ConsentRecord,
        fs: Arc<dyn Fs>,
        cx: &mut Context<Self>,
    ) {
        if let Some((path, text)) = self.consent.grant(extension_id, record) {
            cx.background_spawn(async move { consent::save(path, text, fs.as_ref()).await })
                .detach();
        }
    }

    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalRegistry>()
            .map(|global| global.0.clone())
    }

    pub fn surfaces(&self) -> &[LoadedSurface] {
        &self.surfaces
    }

    /// By display key, `<extension id>/<surface id>` (unambiguous: ids cannot contain `/`).
    pub fn surface(&self, key: &str) -> Option<&LoadedSurface> {
        self.surfaces.iter().find(|surface| surface.info.key == key)
    }

    pub fn surface_of(&self, extension_id: &str, surface_id: &str) -> Option<&LoadedSurface> {
        self.surfaces.iter().find(|surface| {
            surface.extension_id.as_ref() == extension_id
                && surface.surface_id.as_ref() == surface_id
        })
    }

    pub fn surfaces_of<'a>(
        &'a self,
        extension_id: &'a str,
    ) -> impl Iterator<Item = &'a LoadedSurface> + 'a {
        self.surfaces
            .iter()
            .filter(move |surface| surface.extension_id.as_ref() == extension_id)
    }

    /// Claims `extension_id` for `source`. Fails when another source already holds it.
    pub fn claim_extension_id(
        &mut self,
        extension_id: &Arc<str>,
        source: ExtensionSource,
    ) -> anyhow::Result<()> {
        match self.sources.get(extension_id) {
            Some(owner) if *owner != source => anyhow::bail!(
                "extension id {extension_id:?} is already used by {owner:?}; not loading {source:?}"
            ),
            Some(_) => Ok(()),
            None => {
                self.sources.insert(extension_id.clone(), source);
                Ok(())
            }
        }
    }

    fn release_extension_id(&mut self, extension_id: &str) {
        self.sources.remove(extension_id);
    }

    /// The capabilities a loaded UI extension uses, from the surfaces it declared: `ui:panel` for
    /// dock panels, `ui:status-bar` for status items. Empty when `extension_id` is not a loaded
    /// UI extension.
    pub fn capabilities(&self, extension_id: &str, cx: &App) -> Vec<&'static str> {
        let kinds: Vec<SurfaceKind> = self
            .surfaces_of(extension_id)
            .map(|surface| surface.state.read(cx).surface().kind.clone())
            .collect();
        let mut capabilities = Vec::new();
        if kinds
            .iter()
            .any(|kind| matches!(kind, SurfaceKind::Panel(_)))
        {
            capabilities.push(CAPABILITY_PANEL);
        }
        if kinds
            .iter()
            .any(|kind| matches!(kind, SurfaceKind::StatusItem(_)))
        {
            capabilities.push(CAPABILITY_STATUS_BAR);
        }
        capabilities
    }

    /// Adds every surface of a loaded extension and mounts it. Open workspaces pick them up.
    ///
    /// If the extension is already loaded (a reinstall), its surfaces move to the new instance
    /// and mount again; panels and status items stay where they are. Surfaces the new version no
    /// longer declares stop.
    pub fn register_backend(
        &mut self,
        backend: Rc<dyn UiBackend>,
        surfaces: &[Surface],
        cx: &mut Context<Self>,
    ) {
        // A new instance (a reload) starts with none of the old one's notifications.
        self.close_notifications_of(backend.extension_id(), None, cx);
        // A reload starts without decorations; the new instance places its own.
        remove_decorations(backend.extension_id(), cx);
        for loaded in self.surfaces_of(backend.extension_id()) {
            let surface_id = &loaded.surface_id;
            if !surfaces
                .iter()
                .any(|surface| surface.id.as_str() == surface_id.as_ref())
            {
                let message = format!(
                    "{} no longer has the surface {surface_id:?}",
                    backend.extension_id()
                );
                loaded
                    .state
                    .update(cx, |state, cx| state.fail(message.into(), false, cx));
            }
        }
        self.backends
            .insert(backend.extension_id().into(), backend.clone());
        for surface in surfaces {
            self.add_surface(backend.clone(), surface, cx);
        }
        cx.notify();
    }

    /// Adds one surface of a registered extension, or moves an existing one to `backend`.
    fn add_surface(
        &mut self,
        backend: Rc<dyn UiBackend>,
        surface: &Surface,
        cx: &mut Context<Self>,
    ) {
        let info = SurfaceInfo::new(
            backend.extension_id(),
            surface,
            FIRST_ACTIVATION_PRIORITY + self.next_activation_priority,
        );
        if let Some(index) = self.surfaces.iter().position(|loaded| {
            loaded.extension_id.as_ref() == backend.extension_id()
                && loaded.surface_id.as_ref() == surface.id
        }) {
            log::info!("fyx:ui: reloading surface {}", info.key);
            // The new instance's title and icon (triage-3, BUG 31); containers made from now on
            // use them.
            self.surfaces[index].info = Rc::new(info);
            self.surfaces[index].state.update(cx, |state, cx| {
                state.replace_backend(backend.clone(), surface.clone(), cx)
            });
            return;
        }
        self.next_activation_priority += 1;
        let link: Rc<dyn SurfaceLink> = Rc::new(RegistryLink {
            registry: cx.weak_entity(),
            extension_id: backend.extension_id().into(),
        });
        let state =
            cx.new(|cx| SurfaceState::new(backend.clone(), surface.clone(), Some(link), cx));
        self.surfaces.push(LoadedSurface {
            extension_id: backend.extension_id().into(),
            surface_id: surface.id.as_str().into(),
            info: Rc::new(info),
            state,
        });
    }

    /// The outcome of the next load of `extension_id`: loaded, or why not. An install command
    /// awaits it before it says "Installed".
    pub fn wait_for_load(
        &mut self,
        extension_id: &str,
    ) -> futures::channel::oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.load_waiters
            .entry(extension_id.into())
            .or_default()
            .push(tx);
        rx
    }

    fn report_load(&mut self, extension_id: &str, outcome: Result<(), String>) {
        if let Some(waiters) = self.load_waiters.remove(extension_id) {
            for waiter in waiters {
                waiter.send(outcome.clone()).ok();
            }
        }
    }

    /// `notify`: shown in the active window, or with none dropped and the owner told
    /// (`notification-closed(window-closed)`, M.1).
    fn show_notification(
        &mut self,
        extension_id: &str,
        id: u64,
        notification: ui_ext::v0_2::Notification,
        cx: &mut Context<Self>,
    ) {
        let owner = notification.owner.clone();
        let Some(workspace) = self.active_workspace else {
            log::info!(
                "fyx:ui: {extension_id}: no workspace window for the notification {}",
                ui_ext::sanitize_for_log(&notification.title)
            );
            self.send_to_owner(
                extension_id,
                &owner,
                UiEvent::NotificationClosed(id, NotificationCloseReason::WindowClosed),
                cx,
            );
            return;
        };
        let extension_id: Arc<str> = extension_id.into();
        self.notifications.insert(
            (extension_id.clone(), id),
            OpenNotification {
                owner,
                pending: Some(notification),
                view: None,
            },
        );
        cx.emit(RegistryEvent::ShowNotification {
            extension_id,
            id,
            workspace,
        });
    }

    /// For the workspace drawing a notification: what to draw, and the provenance name.
    pub(crate) fn take_pending_notification(
        &mut self,
        extension_id: &str,
        id: u64,
    ) -> Option<(SharedString, ui_ext::v0_2::Notification)> {
        let notification = self
            .notifications
            .get_mut(&(Arc::from(extension_id), id))?
            .pending
            .take()?;
        let name = crate::notifications::provenance_name(
            self.backends
                .get(extension_id)
                .and_then(|backend| backend.display_name()),
            extension_id,
        );
        Some((name, notification))
    }

    pub(crate) fn set_notification_view(
        &mut self,
        extension_id: &str,
        id: u64,
        view: WeakEntity<crate::notifications::ExtensionNotification>,
    ) {
        if let Some(open) = self.notifications.get_mut(&(Arc::from(extension_id), id)) {
            open.view = Some(view);
        }
    }

    /// `dismiss-notification`: closed without an event (D's rule).
    fn dismiss_notification(&mut self, extension_id: &str, id: u64, cx: &mut Context<Self>) {
        if let Some(open) = self.notifications.remove(&(Arc::from(extension_id), id)) {
            close_notification_view(open, cx);
        }
    }

    /// The user or the timer closed a notification, or its window went away.
    pub(crate) fn notification_closed(
        &mut self,
        extension_id: &str,
        id: u64,
        reason: NotificationCloseReason,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.notifications.remove(&(Arc::from(extension_id), id)) {
            self.send_to_owner(
                extension_id,
                &open.owner,
                UiEvent::NotificationClosed(id, reason),
                cx,
            );
        }
    }

    /// A button of a notification was clicked (after the click guard): it closes, and only
    /// `notification-action` goes to the owner.
    pub(crate) fn notification_action(
        &mut self,
        extension_id: &str,
        id: u64,
        action: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.notifications.remove(&(Arc::from(extension_id), id)) {
            self.send_to_owner(
                extension_id,
                &open.owner,
                UiEvent::NotificationAction(NotificationActionEvent {
                    notification: id,
                    action,
                }),
                cx,
            );
        }
    }

    /// Tells the user, in the active window, that a block changed an installed extension. With
    /// no window it stays in the log (the update check logged it already).
    pub(crate) fn notify_block(
        &mut self,
        name: &str,
        block: &crate::fyx_extensions_page::BlockNotice,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.active_workspace else {
            return;
        };
        cx.emit(RegistryEvent::HostNotice {
            id: format!("fyx-ui-block-{}", block.extension_id).into(),
            title: format!("{name} was {}", block.what).into(),
            message: format!(
                "The Fyx block list says: {}\n({} · {})",
                block.reason, name, block.extension_id
            )
            .into(),
            workspace,
        });
    }

    /// The name in the provenance line of an extension's notifications and progress (M.4).
    pub fn provenance_name(&self, extension_id: &str) -> SharedString {
        crate::notifications::provenance_name(
            self.backends
                .get(extension_id)
                .and_then(|backend| backend.display_name()),
            extension_id,
        )
    }

    /// Cancel on a progress: `progress-cancel-requested` to its owner, once (M.2).
    pub(crate) fn request_progress_cancel(
        &mut self,
        extension_id: &str,
        id: u64,
        cx: &mut Context<Self>,
    ) {
        let owner = Progresses::global(cx).and_then(|progresses| {
            progresses.update(cx, |progresses, cx| {
                progresses.request_cancel(extension_id, id, cx)
            })
        });
        if let Some(owner) = owner {
            self.send_to_owner(
                extension_id,
                &owner,
                UiEvent::ProgressCancelRequested(id),
                cx,
            );
        }
    }

    /// A click on a status-bar progress: the owner, if it is a panel, opens.
    pub(crate) fn reveal_owner_panel(
        &mut self,
        extension_id: &str,
        owner: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(surface) = self.surface_of(extension_id, owner)
            && matches!(surface.state.read(cx).surface().kind, SurfaceKind::Panel(_))
        {
            let key = surface.info.key.clone();
            cx.emit(RegistryEvent::RevealPanel(key));
        }
    }

    /// Closes the notifications of an extension (`owner: None`) or of one of its owners, and
    /// ends its progresses, without events: the owner is gone, or the instance is.
    fn close_notifications_of(
        &mut self,
        extension_id: &str,
        owner: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if let Some(progresses) = Progresses::global(cx) {
            progresses.update(cx, |progresses, cx| {
                progresses.end_of(extension_id, owner, cx)
            });
        }
        let closing: Vec<(Arc<str>, u64)> = self
            .notifications
            .iter()
            .filter(|((id, _), open)| {
                id.as_ref() == extension_id && owner.is_none_or(|owner| open.owner == owner)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in closing {
            if let Some(open) = self.notifications.remove(&key) {
                close_notification_view(open, cx);
            }
        }
    }

    /// The open notifications of an extension, for tests and the Inspector.
    pub fn open_notifications(&self, extension_id: &str) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .notifications
            .keys()
            .filter(|(id, _)| id.as_ref() == extension_id)
            .map(|(_, id)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// An event for a notification or progress, to its owner surface or decoration.
    fn send_to_owner(
        &mut self,
        extension_id: &str,
        owner: &str,
        event: UiEvent,
        cx: &mut Context<Self>,
    ) {
        let state = match self.surface_of(extension_id, owner) {
            Some(surface) => Some(surface.state.clone()),
            None => Decorations::global(cx).and_then(|decorations| {
                let decorations = decorations.read(cx);
                let id = decorations.extension_decoration(extension_id, owner)?;
                decorations.state(id).cloned()
            }),
        };
        match state {
            Some(state) if state.read(cx).tree().is_some() => {
                state.update(cx, |state, cx| state.dispatch(event, cx))
            }
            Some(_) => self.deliver_to_unmounted(extension_id, owner, event, cx),
            None => log::debug!("fyx:ui: {extension_id}: no owner {owner:?} for {event:?}"),
        }
    }

    /// An event for an owner with no tree right now (a closed modal, a remount): the surface
    /// would drop it, but the extension must still hear it, and the host frees the notification's
    /// or progress's slot when it passes. Its effects apply; the update has no tree to go to.
    fn deliver_to_unmounted(
        &mut self,
        extension_id: &str,
        owner: &str,
        event: UiEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.backends.get(extension_id).cloned() else {
            return;
        };
        log::debug!("fyx:ui: {extension_id}: {owner:?} is not mounted, sending {event:?} anyway");
        let call = backend.handle_event(owner, event);
        let extension_id: Arc<str> = extension_id.into();
        cx.spawn(async move |this, cx| {
            let effects = match call.await {
                Ok(reply) => reply.effects,
                Err(error) => match error.downcast_ref::<crate::backend::ReplyError>() {
                    Some(rejected) => rejected.effects.clone(),
                    None => {
                        log::error!(
                            "fyx:ui: {extension_id}: event for an unmounted surface failed: {}",
                            ui_ext::sanitize_for_log(&format!("{error:#}"))
                        );
                        return;
                    }
                },
            };
            this.update(cx, |this, cx| {
                for effect in effects {
                    this.apply_effect(&extension_id, effect, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The workspace whose window became active; modals open there.
    pub fn set_active_workspace(&mut self, workspace: gpui::EntityId) {
        self.active_workspace = Some(workspace);
    }

    /// `show-surface` of a modal (the host checked the gesture): a fresh mount, shown in the
    /// active window. With no workspace window the effect is dropped and the host told.
    fn show_modal(&mut self, extension_id: &str, surface_id: &str, cx: &mut Context<Self>) {
        let Some(surface) = self.surface_of(extension_id, surface_id) else {
            return;
        };
        let Some(workspace) = self.active_workspace else {
            log::warn!(
                "fyx:ui: {extension_id}: no workspace window to show the modal {surface_id:?} in"
            );
            if let Some(backend) = self.backends.get(extension_id) {
                backend.modal_closed(surface_id);
            }
            return;
        };
        let key = surface.info.key.clone();
        if surface.state.read(cx).is_open() {
            return;
        }
        surface.state.update(cx, |state, cx| state.show(cx));
        cx.emit(RegistryEvent::ShowModal { key, workspace });
    }

    /// The extension closed a modal: nothing is sent to it.
    fn close_modal(&mut self, extension_id: &str, surface_id: &str, cx: &mut Context<Self>) {
        let Some(surface) = self.surface_of(extension_id, surface_id) else {
            return;
        };
        if !surface.state.read(cx).is_modal() {
            return;
        }
        let key = surface.info.key.clone();
        surface.state.update(cx, |state, cx| state.close(None, cx));
        cx.emit(RegistryEvent::CloseModal(key));
    }

    /// `remove-surface`: its containers close in every workspace; its tree, queued events and
    /// tick go with its state. No event reaches the extension.
    fn remove_surface(&mut self, extension_id: &str, surface_id: &str, cx: &mut Context<Self>) {
        // Nothing outlives its owner, and its removal sends no events (design.md M).
        self.close_notifications_of(extension_id, Some(surface_id), cx);
        let before = self.surfaces.len();
        self.surfaces.retain(|surface| {
            !(surface.extension_id.as_ref() == extension_id
                && surface.surface_id.as_ref() == surface_id)
        });
        if self.surfaces.len() == before {
            log::debug!("fyx:ui: {extension_id} removed unknown surface {surface_id:?}");
        }
        cx.notify();
    }

    /// Registers a WASM extension and applies the updates it pushes outside of any call (the
    /// pushes made during a call come back with its reply), batched per wake-up: a burst of
    /// pushes becomes one repaint on the next frame.
    pub fn register_extension(
        &mut self,
        extension: UiExtension,
        mut pushed: mpsc::UnboundedReceiver<ui_ext_wasm::PushedUpdate>,
        cx: &mut Context<Self>,
    ) {
        let extension_id = extension.extension_id().clone();
        let surfaces: Vec<Surface> = extension.surfaces_v0_2().to_vec();
        let initial_effects = extension.initial_effects().to_vec();
        self.register_backend(Rc::new(extension), &surfaces, cx);
        // Effects of instantiation and `surfaces` (0.2), after the static surfaces exist.
        for effect in initial_effects {
            self.apply_effect(&extension_id, effect, cx);
        }
        cx.spawn(async move |this, cx| {
            while let Some(first) = pushed.next().await {
                let mut batch = vec![first];
                while let Ok(next) = pushed.try_recv() {
                    batch.push(next);
                }
                let applied = this.update(cx, |this, cx| {
                    for pushed in batch {
                        let (surface_id, update) = (pushed.surface_id, pushed.update.into());
                        this.apply_pushed(&extension_id, &surface_id, update, cx);
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Loads the UI extensions of the installed-extensions directory that are new or changed since
    /// the last scan, and stops the surfaces of those that are gone. Scans do not overlap: a
    /// request during a scan makes it run once more at the end.
    pub fn scan_installed(&mut self, dir: PathBuf, fs: Arc<dyn Fs>, cx: &mut Context<Self>) {
        self.last_scan = Some((dir.clone(), fs.clone()));
        if self.scanning {
            self.rescan_requested = true;
            return;
        }
        self.scanning = true;
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            loop {
                // Folders that exist but cannot be read: not uninstalls (triage-3 #7).
                let mut unreadable = Vec::new();
                let mut found = find_ui_extensions(&dir, fs.as_ref(), &mut unreadable).await;
                if let Some(store) = &store {
                    found.extend(find_store_extensions(store, fs.as_ref(), &mut unreadable).await);
                }
                // Read again on every round: a change during the scan asks for another round.
                let dev_paths = this
                    .update(cx, |this, _| this.dev_paths.clone())
                    .unwrap_or_default();
                for path in dev_paths {
                    let source = ExtensionSource::Directory(path.clone());
                    match found_at(
                        path.clone(),
                        true,
                        source,
                        None,
                        fs.as_ref(),
                        &mut unreadable,
                    )
                    .await
                    {
                        Some(extension) => found.push(extension),
                        None => log::error!(
                            "fyx:ui: fyx.extensions.dev_paths: {} is not a built UI extension",
                            path.display()
                        ),
                    }
                }
                let Ok((to_load, consent_file)) = this.update(cx, |this, cx| {
                    (
                        this.plan_scan(found, &unreadable, cx),
                        this.consent.path_to_load(),
                    )
                }) else {
                    return;
                };
                if let Some(path) = consent_file {
                    let records = consent::read_records(&path, fs.as_ref()).await;
                    this.update(cx, |this, _| this.consent.set_loaded(records))
                        .ok();
                }
                // Check each extension in turn (consent may ask), then load them all at once: each
                // compiles on the background executor, so the loads overlap (triage-3, PERF 8).
                let mut ready = Vec::new();
                for found in to_load {
                    let trusted = this
                        .update(cx, |this, _| this.trust_installed)
                        .unwrap_or(false);
                    // One read of the component: the consent check and the load use the same
                    // bytes, as they use the same manifest (triage-3, SEC T1).
                    let mut bytes = match fs.load_bytes(&found.component).await {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            log::error!(
                                "fyx:ui: cannot read {}: {error:#}",
                                found.component.display()
                            );
                            this.update(cx, |this, _| {
                                this.report_load(&found.extension_id, Err(format!("{error:#}")))
                            })
                            .ok();
                            continue;
                        }
                    };
                    if !trusted {
                        let record = match ConsentRecord::for_bytes(
                            &found.dir,
                            &found.capabilities,
                            bytes,
                            found.is_dev,
                            fs.as_ref(),
                            &cx.background_executor(),
                        )
                        .await
                        {
                            Ok((record, same_bytes)) => {
                                bytes = same_bytes;
                                record
                            }
                            Err(error) => {
                                log::error!(
                                    "fyx:ui: not loading {}: {error:#}",
                                    found.dir.display()
                                );
                                continue;
                            }
                        };
                        let allowed = this
                            .update(cx, |this, cx| this.check_consent(&found, &dir, record, cx));
                        if !matches!(allowed, Ok(true)) {
                            this.update(cx, |this, _| {
                                this.report_load(
                                    &found.extension_id,
                                    Err("it waits for your approval (see the notification)".into()),
                                )
                            })
                            .ok();
                            continue;
                        }
                    }
                    let checker = this.update(cx, |this, _| this.consent.checker()).ok();
                    let options = LoadOptions {
                        cache: found.cache.clone(),
                        capability_check: checker,
                        allow_staged: cx.update(|cx| staged_allowed(found.is_dev, cx)),
                        ..LoadOptions::default()
                    };
                    ready.push((found, bytes, options));
                }
                let loads = ready.into_iter().map(|(found, bytes, options)| {
                    let load = UiExtension::load_checked(
                        found.extension_id.clone(),
                        found.manifest.clone(),
                        bytes,
                        options,
                        cx,
                    );
                    async move { (found, load.await.map(Some)) }
                });
                let results = futures::future::join_all(loads).await;
                for (found, loaded) in results {
                    match loaded {
                        Ok(Some((extension, pushed))) => {
                            log::info!(
                                "fyx:ui: loaded installed extension {} with {} surface(s)",
                                extension.extension_id(),
                                extension.surfaces().len()
                            );
                            this.update(cx, |this, cx| {
                                if let Err(error) = this.claim_extension_id(
                                    extension.extension_id(),
                                    found.source.clone(),
                                ) {
                                    log::error!("fyx:ui: {error:#}");
                                    this.report_load(
                                        &found.extension_id,
                                        Err(format!("{error:#}")),
                                    );
                                    return;
                                }
                                this.report_load(&found.extension_id, Ok(()));
                                this.installed
                                    .insert(found.extension_id.clone(), found.fingerprint.clone());
                                this.consent
                                    .declare(&found.extension_id, found.capabilities.clone());
                                this.register_extension(extension, pushed, cx);
                                if found.is_dev {
                                    this.watch_component(
                                        found.extension_id.clone(),
                                        found.component.clone(),
                                        Reload::Installed(dir.clone()),
                                        fs.clone(),
                                        cx,
                                    );
                                }
                            })
                            .ok();
                        }
                        Ok(None) => {}
                        Err(error) => {
                            log::error!(
                                "fyx:ui: failed to load installed extension {}: {error:#}",
                                found.dir.display()
                            );
                            this.update(cx, |this, cx| {
                                this.report_load(&found.extension_id, Err(format!("{error:#}")));
                                this.reload_failed(&found.extension_id, &error, cx)
                            })
                            .ok();
                        }
                    }
                }
                let again = this.update(cx, |this, _| {
                    let again = std::mem::take(&mut this.rescan_requested);
                    this.scanning = again;
                    again
                });
                if !matches!(again, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    /// Watches a dev extension's component and loads it again when it changes (once per id).
    fn watch_component(
        &mut self,
        extension_id: Arc<str>,
        component: PathBuf,
        reload: Reload,
        fs: Arc<dyn Fs>,
        cx: &mut Context<Self>,
    ) {
        if self.watches.contains_key(&extension_id) {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            // Through the link of a dev install, to the file that a build rewrites.
            let component = fs.canonicalize(&component).await.unwrap_or(component);
            let Some(dir) = component.parent().map(Path::to_path_buf) else {
                return;
            };
            let (mut events, _watcher) = fs.watch(&dir, Duration::from_millis(50)).await;
            let touches = |events: &[PathEvent]| {
                events.iter().any(|event| {
                    event.path == component || event.path.file_name() == component.file_name()
                })
            };
            while let Some(batch) = events.next().await {
                if !touches(&batch) {
                    continue;
                }
                // Wait until the writes pause.
                loop {
                    let mut pause = cx.background_executor().timer(HOT_RELOAD_DEBOUNCE).fuse();
                    futures::select_biased! {
                        more = events.next().fuse() => {
                            if more.is_none() {
                                return;
                            }
                        }
                        _ = pause => break,
                    }
                }
                log::info!("fyx:ui: {} changed, reloading", component.display());
                let reloaded =
                    this.update(cx, |this, cx| this.reload(reload.clone(), fs.clone(), cx));
                if reloaded.is_err() {
                    return;
                }
            }
        });
        self.watches.insert(extension_id, task);
    }

    fn reload(&mut self, reload: Reload, fs: Arc<dyn Fs>, cx: &mut Context<Self>) {
        match reload {
            // The scan compares the component's fingerprint and loads it again if it changed.
            Reload::Installed(installed_dir) => self.scan_installed(installed_dir, fs, cx),
            Reload::DevPath(path) => {
                let key = path.clone();
                let task = cx.spawn(async move |this, cx| {
                    if let Err(error) = load_dev_extension(path.clone(), cx).await {
                        log::error!(
                            "fyx:ui: failed to reload dev extension {}: {error:#}",
                            path.display()
                        );
                        this.update(cx, |this, cx| {
                            let extension_id = this.sources.iter().find_map(|(id, source)| {
                                matches!(
                                    source,
                                    ExtensionSource::Directory(p) | ExtensionSource::File(p)
                                        if *p == path
                                )
                                .then(|| id.clone())
                            });
                            if let Some(extension_id) = extension_id {
                                this.reload_failed(&extension_id, &error, cx);
                            }
                        })
                        .ok();
                    }
                });
                self.dev_reloads.insert(key, task);
            }
        }
    }

    /// Whether the user consented to `found` as it is now; if not, it waits in
    /// `pending_consent` for an "Enable".
    fn check_consent(
        &mut self,
        found: &Found,
        installed_dir: &Path,
        record: ConsentRecord,
        cx: &mut Context<Self>,
    ) -> bool {
        self.pending_consent
            .retain(|pending| pending.extension_id != found.extension_id);
        let new_capabilities = match self.consent.state(&found.extension_id, &record) {
            ConsentState::Allowed => {
                cx.notify();
                return true;
            }
            ConsentState::NotEnabled => {
                log::info!(
                    "fyx:ui: {} is installed but not enabled; waiting for the user",
                    found.extension_id
                );
                Vec::new()
            }
            ConsentState::NeedsApproval { new_capabilities } => {
                log::info!(
                    "fyx:ui: {} now asks for {new_capabilities:?}; the update waits for approval",
                    found.extension_id
                );
                new_capabilities
            }
        };
        self.pending_consent.push(PendingConsent {
            extension_id: found.extension_id.clone(),
            dir: found.dir.clone(),
            installed_dir: installed_dir.to_path_buf(),
            capabilities: found.capabilities.clone(),
            is_dev: found.is_dev,
            new_capabilities,
            record,
        });
        cx.notify();
        false
    }

    /// Which found extensions need loading; stops the surfaces of uninstalled ones.
    fn plan_scan(
        &mut self,
        found: Vec<Found>,
        unreadable: &[(PathBuf, String)],
        cx: &mut Context<Self>,
    ) -> Vec<Found> {
        // A loaded extension whose folder is there but whose manifest no longer reads is not
        // uninstalled: it keeps running and its surfaces say what is wrong (triage-3 #7).
        let broken: Vec<(Arc<str>, String)> = self
            .installed
            .iter()
            .filter_map(|(id, fingerprint)| {
                unreadable
                    .iter()
                    .find(|(dir, _)| *dir == fingerprint.dir)
                    .map(|(_, error)| (id.clone(), error.clone()))
            })
            .collect();
        for (extension_id, error) in &broken {
            self.reload_failed(
                extension_id,
                &anyhow::anyhow!("its extension.toml cannot be read: {error}"),
                cx,
            );
        }
        self.pending_consent.retain(|pending| {
            found
                .iter()
                .any(|found| found.extension_id == pending.extension_id)
        });
        let gone: Vec<Arc<str>> = self
            .installed
            .keys()
            .filter(|id| !found.iter().any(|found| &found.extension_id == *id))
            .filter(|id| !broken.iter().any(|(broken, _)| broken == *id))
            .cloned()
            .collect();
        for extension_id in gone {
            self.installed.remove(&extension_id);
            self.watches.remove(&extension_id);
            self.release_extension_id(&extension_id);
            log::info!("fyx:ui: {extension_id} was uninstalled");
            self.fail_extension(
                &extension_id,
                "",
                format!("{extension_id} was uninstalled").into(),
                cx,
            );
        }
        found
            .into_iter()
            .filter(|found| self.installed.get(&found.extension_id) != Some(&found.fingerprint))
            .collect()
    }

    /// Stops every surface of an extension except `except_surface` (which stops itself).
    fn fail_extension(
        &mut self,
        extension_id: &str,
        except_surface: &str,
        message: SharedString,
        cx: &mut Context<Self>,
    ) {
        // The instance is gone (trap, disabled, uninstalled): so are its decorations, its
        // notifications, and the registry's handle on it (triage-3, ASYNC 7).
        remove_decorations(extension_id, cx);
        self.close_notifications_of(extension_id, None, cx);
        self.backends.remove(extension_id);
        for surface in self.surfaces_of(extension_id) {
            if surface.surface_id.as_ref() != except_surface {
                surface
                    .state
                    .update(cx, |state, cx| state.fail(message.clone(), false, cx));
            }
        }
    }

    /// Applies one effect of a call (or of instantiation) that the surface that made the call did
    /// not apply itself. Effects come in emission order.
    pub fn apply_effect(&mut self, extension_id: &str, effect: HostEffect, cx: &mut Context<Self>) {
        match effect {
            HostEffect::Push { surface_id, update } => {
                self.apply_pushed(extension_id, &surface_id, update, cx)
            }
            HostEffect::AddSurface(surface) => {
                let Some(backend) = self.backends.get(extension_id).cloned() else {
                    log::error!("fyx:ui: {extension_id} added a surface before it was registered");
                    return;
                };
                // A modal is added closed; panels and status items mount at once.
                self.add_surface(backend, &surface, cx);
                cx.notify();
            }
            HostEffect::RemoveSurface { surface_id } => {
                self.close_modal(extension_id, &surface_id, cx);
                self.remove_surface(extension_id, &surface_id, cx)
            }
            HostEffect::ShowSurface { surface_id }
                if self
                    .surface_of(extension_id, &surface_id)
                    .is_some_and(|surface| surface.state.read(cx).is_modal()) =>
            {
                self.show_modal(extension_id, &surface_id, cx)
            }
            HostEffect::HideSurface { surface_id }
                if self
                    .surface_of(extension_id, &surface_id)
                    .is_some_and(|surface| surface.state.read(cx).is_modal()) =>
            {
                self.close_modal(extension_id, &surface_id, cx)
            }
            HostEffect::ShowSurface { surface_id } => match self
                .surface_of(extension_id, &surface_id)
            {
                Some(surface)
                    if matches!(surface.state.read(cx).surface().kind, SurfaceKind::Panel(_)) =>
                {
                    let key = surface.info.key.clone();
                    cx.emit(RegistryEvent::RevealPanel(key));
                }
                Some(_) => log::debug!(
                    "fyx:ui: {extension_id}: show-surface({surface_id:?}) of a status item does nothing"
                ),
                None => {
                    log::debug!("fyx:ui: {extension_id}: show-surface of unknown {surface_id:?}")
                }
            },
            HostEffect::RequestTick { surface_id, delay } => {
                match self.surface_of(extension_id, &surface_id) {
                    Some(surface) => surface
                        .state
                        .update(cx, |state, cx| state.request_tick(delay, cx)),
                    None => {
                        log::debug!(
                            "fyx:ui: {extension_id}: request-tick of unknown {surface_id:?}"
                        )
                    }
                }
            }
            HostEffect::CancelTick { surface_id } => {
                if let Some(surface) = self.surface_of(extension_id, &surface_id) {
                    surface.state.update(cx, |state, _| state.cancel_tick());
                }
            }
            HostEffect::HideSurface { surface_id } => {
                if let Some(surface) = self.surface_of(extension_id, &surface_id)
                    && matches!(surface.state.read(cx).surface().kind, SurfaceKind::Panel(_))
                {
                    let key = surface.info.key.clone();
                    cx.emit(RegistryEvent::HidePanel(key));
                }
            }
            HostEffect::PlaceDecoration {
                id,
                location,
                nodes,
            } => {
                let Some(backend) = self.backends.get(extension_id).cloned() else {
                    log::error!(
                        "fyx:ui: {extension_id} placed a decoration before it was registered"
                    );
                    return;
                };
                let Some(decorations) = Decorations::global(cx) else {
                    log::debug!(
                        "fyx:ui: {extension_id}: no decorations in this app, dropping {id:?}"
                    );
                    return;
                };
                let link: Rc<dyn SurfaceLink> = Rc::new(RegistryLink {
                    registry: cx.weak_entity(),
                    extension_id: extension_id.into(),
                });
                let placed = decorations.update(cx, |decorations, cx| {
                    decorations.place_extension(
                        extension_id,
                        &id,
                        &location,
                        nodes,
                        backend,
                        Some(link),
                        cx,
                    )
                });
                if let Err(error) = placed {
                    log::error!("fyx:ui: {extension_id}: place-decoration({id:?}): {error:#}");
                }
            }
            HostEffect::RemoveDecoration { id } => {
                let removed = Decorations::global(cx).is_some_and(|decorations| {
                    decorations.update(cx, |decorations, cx| {
                        decorations.remove_extension_decoration(extension_id, &id, cx)
                    })
                });
                if removed {
                    self.close_notifications_of(extension_id, Some(&id), cx);
                } else {
                    log::debug!("fyx:ui: {extension_id} removed unknown decoration {id:?}");
                }
            }
            // Images are 0.3 (plan 4.10, 4.11): no renderer for them yet, and a 0.2 component
            // cannot make these effects.
            HostEffect::AddImage { handle, .. } | HostEffect::RemoveImage { handle } => {
                log::debug!(
                    "fyx:ui: {extension_id}: image {handle} ignored (no image renderer yet)"
                )
            }
            HostEffect::Notify { id, notification } => {
                self.show_notification(extension_id, id, notification, cx)
            }
            HostEffect::DismissNotification { id } => {
                self.dismiss_notification(extension_id, id, cx)
            }
            HostEffect::ProgressStart { id, options } => {
                if let Some(progresses) = Progresses::global(cx) {
                    progresses.update(cx, |progresses, cx| {
                        progresses.start(extension_id, id, options, cx)
                    });
                }
            }
            HostEffect::ProgressUpdate { id, value, message } => {
                if let Some(progresses) = Progresses::global(cx) {
                    progresses.update(cx, |progresses, cx| {
                        progresses.update(extension_id, id, value, message, cx)
                    });
                }
            }
            HostEffect::ProgressEnd { id } => {
                if let Some(progresses) = Progresses::global(cx) {
                    progresses.update(cx, |progresses, cx| progresses.end(extension_id, id, cx));
                }
            }
        }
    }

    fn apply_pushed(
        &mut self,
        extension_id: &str,
        surface_id: &str,
        update: Update,
        cx: &mut Context<Self>,
    ) {
        match self.surface_of(extension_id, surface_id) {
            Some(surface) => surface.state.update(cx, |state, cx| {
                state.apply_pushed_update(update, cx);
            }),
            None => {
                // Decoration ids share the surface namespace (design.md §L).
                let pushed = Decorations::global(cx).is_some_and(|decorations| {
                    decorations.update(cx, |decorations, cx| {
                        decorations.push(extension_id, surface_id, update, cx)
                    })
                });
                if !pushed {
                    log::error!(
                        "fyx:ui: {extension_id} pushed an update for unknown surface {surface_id:?}"
                    );
                }
            }
        }
    }
}

/// The UI extensions in `dir`: subdirectories (or links to them, as dev installs are) whose
/// `extension.toml` has a `[fyx_ui]` table. Others are regular Zed extensions and are skipped.
async fn find_ui_extensions(
    dir: &Path,
    fs: &dyn Fs,
    unreadable: &mut Vec<(PathBuf, String)>,
) -> Vec<Found> {
    let mut entries = match fs.read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) => {
            log::debug!(
                "fyx:ui: no installed extensions at {}: {error:#}",
                dir.display()
            );
            return Vec::new();
        }
    };
    let mut found = Vec::new();
    while let Some(entry) = entries.next().await {
        let Ok(extension_dir) = entry else {
            continue;
        };
        let is_dev = fs
            .metadata(&extension_dir)
            .await
            .ok()
            .flatten()
            .is_some_and(|metadata| metadata.is_symlink);
        let source = ExtensionSource::Directory(extension_dir.clone());
        if let Some(extension) = found_at(extension_dir, is_dev, source, None, fs, unreadable).await
        {
            found.push(extension);
        }
    }
    found
}

/// The active version of every extension in the install store.
async fn find_store_extensions(
    store: &InstallStore,
    fs: &dyn Fs,
    unreadable: &mut Vec<(PathBuf, String)>,
) -> Vec<Found> {
    let active = match store.active(fs).await {
        Ok(active) => active,
        Err(error) => {
            log::error!("fyx:ui: cannot read the install store: {error:#}");
            return Vec::new();
        }
    };
    let mut found = Vec::new();
    for (extension_id, version, dir) in active {
        let cache = store.precompiled_cache(&extension_id, &version);
        match found_at(
            dir,
            false,
            ExtensionSource::Store,
            Some(cache),
            fs,
            unreadable,
        )
        .await
        {
            Some(extension) if extension.extension_id == extension_id => found.push(extension),
            Some(extension) => log::error!(
                "fyx:ui: the install store lists {extension_id}, but its folder holds {}",
                extension.extension_id
            ),
            None => {}
        }
    }
    found
}

/// The UI extension in `extension_dir`, if it is one and its component is built.
async fn found_at(
    extension_dir: PathBuf,
    is_dev: bool,
    source: ExtensionSource,
    cache: Option<PrecompiledCache>,
    fs: &dyn Fs,
    unreadable: &mut Vec<(PathBuf, String)>,
) -> Option<Found> {
    let (extension_id, manifest) = match UiManifest::read(fs, &extension_dir).await {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return None,
        Err(error) => {
            // Also a regular Zed extension with a broken manifest; logged, not shown.
            log::warn!("fyx:ui: cannot read {}: {error:#}", extension_dir.display());
            unreadable.push((extension_dir, format!("{error:#}")));
            return None;
        }
    };
    let component = extension_dir.join(&manifest.component);
    match fs.metadata(&component).await {
        Ok(Some(metadata)) => Some(Found {
            fingerprint: Fingerprint {
                dir: extension_dir.clone(),
                mtime: metadata.mtime,
                len: metadata.len,
            },
            dir: extension_dir,
            extension_id,
            component,
            is_dev,
            capabilities: manifest.capabilities.clone(),
            source,
            cache,
            manifest,
        }),
        Ok(None) | Err(_) => {
            log::error!(
                "fyx:ui: {extension_id}: component {} is missing (build it first)",
                component.display()
            );
            None
        }
    }
}

/// Removes every decoration of an extension (unload, reload, disable).
fn remove_decorations(extension_id: &str, cx: &mut App) {
    if let Some(decorations) = Decorations::global(cx) {
        let removed = decorations.update(cx, |decorations, cx| {
            decorations.remove_extension(extension_id, cx)
        });
        if removed > 0 {
            log::debug!("fyx:ui: removed {removed} decoration(s) of {extension_id}");
        }
    }
}

/// A surface's way back to the registry, for the other surfaces of its extension.
struct RegistryLink {
    registry: WeakEntity<UiExtensionRegistry>,
    extension_id: Arc<str>,
}

impl SurfaceLink for RegistryLink {
    fn route_effect(&self, effect: HostEffect, cx: &mut App) {
        self.registry
            .update(cx, |registry, cx| {
                registry.apply_effect(&self.extension_id, effect, cx)
            })
            .ok();
    }

    fn extension_failed(&self, from_surface: &str, message: SharedString, cx: &mut App) {
        self.registry
            .update(cx, |registry, cx| {
                registry.fail_extension(&self.extension_id, from_surface, message, cx)
            })
            .ok();
    }

    fn reserve_tick(&self, due: std::time::Instant, cx: &mut App) -> std::time::Instant {
        self.registry
            .update(cx, |registry, _| {
                registry
                    .tick_budgets
                    .entry(self.extension_id.clone())
                    .or_default()
                    .reserve(due)
            })
            .unwrap_or(due)
    }

    fn min_tick_delay(&self, cx: &App) -> std::time::Duration {
        self.registry
            .upgrade()
            .and_then(|registry| {
                registry
                    .read(cx)
                    .tick_budgets
                    .get(&self.extension_id)
                    .map(|budget| budget.min_delay(cx.background_executor().now()))
            })
            .unwrap_or(ui_ext_wasm::ticks::MIN_TICK_DELAY)
    }

    fn record_tick(
        &self,
        finished_at: std::time::Instant,
        busy: std::time::Duration,
        cx: &mut App,
    ) {
        self.registry
            .update(cx, |registry, _| {
                let throttled = registry
                    .tick_budgets
                    .entry(self.extension_id.clone())
                    .or_default()
                    .record(finished_at, busy);
                if throttled {
                    log::warn!(
                        "fyx:ui: {} spends more than {}% of its time in ticks; its ticks run at \
                         most once a second for {:?}",
                        self.extension_id,
                        ui_ext_wasm::ticks::TICK_CPU_SHARE_PERCENT,
                        ui_ext_wasm::ticks::TICK_CPU_WINDOW
                    );
                }
            })
            .ok();
    }

    fn restart_extension(&self, cx: &mut App) {
        self.registry
            .update(cx, |registry, cx| {
                registry.restart_extension(&self.extension_id, cx)
            })
            .ok();
    }
}

pub fn init(cx: &mut App) {
    let registry = cx.new(|_| {
        UiExtensionRegistry::new(ConsentStore::default_path())
            .with_store(InstallStore::new(InstallStore::default_root()))
    });
    cx.set_global(GlobalRegistry(registry));

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        attach_to_workspace(workspace, window, cx);
    })
    .detach();

    // FYX_UI_DEV_EXTENSIONS loads without consent, so only dev builds honour it (triage-3).
    if dev_build(cx) {
        load_dev_extensions(cx);
    } else if !dev_extension_paths().is_empty() {
        log::warn!(
            "fyx:ui: {DEV_EXTENSIONS_ENV} is ignored outside dev builds; use \
             fyx_ui: install dev UI extension"
        );
    }
    follow_dev_paths_setting(cx);
    watch_installed_extensions(cx);
}

/// A dev build: the Dev release channel (also when none is set, as in tests).
fn dev_build(cx: &App) -> bool {
    release_channel::ReleaseChannel::try_global(cx)
        .is_none_or(|channel| channel == release_channel::ReleaseChannel::Dev)
}

/// Whether a staged `fyx:ui` version (0.2 before it is frozen) may load (migration.md §2): on Dev
/// and Nightly builds, and for dev extensions. `None` before the release channel is set (tests),
/// which leaves `ui_ext_wasm`'s default.
fn staged_allowed(dev_extension: bool, cx: &App) -> Option<bool> {
    let channel = release_channel::ReleaseChannel::try_global(cx)?;
    let dev_or_nightly = matches!(
        channel,
        release_channel::ReleaseChannel::Dev | release_channel::ReleaseChannel::Nightly
    );
    Some(ui_ext_wasm::staged_api_allowed(
        dev_or_nightly,
        dev_extension,
    ))
}

/// Keeps the registry's dev paths equal to `fyx.extensions.dev_paths`.
fn follow_dev_paths_setting(cx: &mut App) {
    let Some(registry) = UiExtensionRegistry::global(cx) else {
        return;
    };
    let dev_paths = crate::install::dev_paths_setting(cx);
    registry.update(cx, |registry, cx| registry.set_dev_paths(dev_paths, cx));
    if cx.has_global::<settings::SettingsStore>() {
        cx.observe_global::<settings::SettingsStore>(move |cx| {
            let dev_paths = crate::install::dev_paths_setting(cx);
            registry.update(cx, |registry, cx| registry.set_dev_paths(dev_paths, cx));
        })
        .detach();
    }
}

/// Loads the installed UI extensions now and again whenever the extension store changes.
fn watch_installed_extensions(cx: &mut App) {
    let (Some(store), Some(registry)) = (
        ExtensionStore::try_global(cx),
        UiExtensionRegistry::global(cx),
    ) else {
        return;
    };
    let dir = store.read(cx).installed_dir.clone();
    let fs = <dyn Fs>::global(cx);
    let scan = move |registry: &Entity<UiExtensionRegistry>, cx: &mut App| {
        registry.update(cx, |registry, cx| {
            registry.scan_installed(dir.clone(), fs.clone(), cx)
        })
    };
    scan(&registry, cx);
    cx.subscribe(&store, move |_, event, cx| match event {
        extension_host::Event::ExtensionInstalled(_)
        | extension_host::Event::ExtensionUninstalled(_)
        | extension_host::Event::ExtensionsUpdated => scan(&registry, cx),
        extension_host::Event::StartedReloading
        | extension_host::Event::ExtensionFailedToLoad(_) => {}
    })
    .detach();
}

/// The extension containers of one workspace, by surface key.
#[derive(Default)]
struct WorkspaceSurfaces {
    panels: HashMap<SharedString, Box<dyn AnyExtensionPanel>>,
    /// One status bar entry per side, holding that side's extension status items.
    status_groups: HashMap<bool, Entity<ExtensionStatusGroup>>,
    status_items: HashMap<SharedString, (bool, Entity<ExtensionStatusItem>)>,
    /// Extensions this workspace has offered "Enable" for.
    consent_prompts: HashSet<Arc<str>>,
}

fn attach_to_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(registry) = UiExtensionRegistry::global(cx) else {
        return;
    };
    let surfaces = Rc::new(RefCell::new(WorkspaceSurfaces::default()));

    crate::store_actions::register(workspace);
    crate::fyx_extensions_page::register(workspace);
    if crate::inspector::inspector_enabled(cx) {
        let inspector =
            cx.new(|cx| crate::inspector::ExtensionInspector::new(registry.clone(), cx));
        workspace.add_panel(inspector, window, cx);
        workspace.register_action(
            |workspace, _: &crate::ToggleExtensionInspector, window, cx| {
                workspace.toggle_panel_focus::<crate::inspector::ExtensionInspector>(window, cx);
            },
        );
    }
    workspace.register_action(|workspace, _: &crate::InstallDevUiExtension, window, cx| {
        crate::install::prompt_and_install(workspace, window, cx)
    });
    workspace.register_action(|workspace, _: &crate::RemoveDevUiExtension, window, cx| {
        crate::install::prompt_and_remove(workspace, window, cx)
    });
    workspace.register_action({
        let surfaces = surfaces.clone();
        move |workspace, action: &ToggleExtensionPanel, window, cx| {
            let panel = surfaces
                .borrow()
                .panels
                .get(action.key.as_str())
                .and_then(|panel| {
                    Some((
                        panel.panel_id(),
                        panel.panel_focus(cx)?,
                        panel.surface_focus(cx)?,
                    ))
                });
            match panel {
                Some((panel_id, panel_focus, surface_focus)) => toggle_panel_focus(
                    workspace,
                    panel_id,
                    &panel_focus,
                    &surface_focus,
                    window,
                    cx,
                ),
                None => log::debug!("fyx:ui: no extension panel {:?}", action.key),
            }
        }
    });

    cx.observe_in(&registry, window, {
        let surfaces = surfaces.clone();
        move |workspace, registry, window, cx| {
            add_missing_surfaces(workspace, &registry, &surfaces, window, cx);
            offer_consent(workspace, &registry, &surfaces, cx);
        }
    })
    .detach();

    cx.subscribe_in(&registry, window, {
        let surfaces = surfaces.clone();
        move |workspace, registry, event: &RegistryEvent, window, cx| {
            let (key, open) = match event {
                RegistryEvent::ShowModal {
                    key,
                    workspace: target,
                } => {
                    if *target == cx.entity_id() {
                        open_modal(workspace, &registry, key.clone(), window, cx);
                    }
                    return;
                }
                RegistryEvent::ShowNotification {
                    extension_id,
                    id,
                    workspace: target,
                } => {
                    if *target == cx.entity_id() {
                        show_extension_notification(
                            workspace,
                            &registry,
                            extension_id.clone(),
                            *id,
                            window,
                            cx,
                        );
                    }
                    return;
                }
                RegistryEvent::HostNotice {
                    id,
                    title,
                    message,
                    workspace: target,
                } => {
                    if *target == cx.entity_id() {
                        let (title, message) = (title.clone(), message.clone());
                        workspace.show_notification(NotificationId::named(id.clone()), cx, |cx| {
                            cx.new(|cx| {
                                MessageNotification::new(message, cx)
                                    .with_title(title)
                                    .primary_message("Open Fyx Extensions")
                                    .primary_on_click(|window, cx| {
                                        window
                                            .dispatch_action(Box::new(crate::OpenFyxExtensions), cx)
                                    })
                            })
                        });
                    }
                    return;
                }
                RegistryEvent::CloseModal(key) => {
                    if let Some(modal) = workspace.active_modal::<ExtensionModal>(cx)
                        && modal.read(cx).key() == key
                    {
                        modal.update(cx, |modal, cx| modal.close_by_host(cx));
                    }
                    return;
                }
                RegistryEvent::RevealPanel(key) => (key, true),
                RegistryEvent::HidePanel(key) => (key, false),
            };
            // `show-surface` and `hide-surface` of a panel act in the window the user is in.
            if !window.is_window_active() && cx.active_window().is_some() {
                return;
            }
            let Some(panel_id) = surfaces
                .borrow()
                .panels
                .get(key)
                .map(|panel| panel.panel_id())
            else {
                return;
            };
            for dock in workspace.all_docks() {
                let Some(index) = dock.read(cx).panel_index_for_id(panel_id) else {
                    continue;
                };
                dock.update(cx, |dock, cx| {
                    if open {
                        dock.activate_panel(index, window, cx);
                        dock.set_open(true, window, cx);
                    } else if dock.active_panel_index() == Some(index) {
                        dock.set_open(false, window, cx);
                    }
                });
            }
        }
    })
    .detach();

    // A banner while FYX_UI_DEV_EXTENSIONS is in use: those load without consent (triage-3).
    if dev_build(cx) && !dev_extension_paths().is_empty() {
        let message = format!(
            "{DEV_EXTENSIONS_ENV} is set: its UI extensions load without asking for consent.              Unset it to stop."
        );
        workspace.show_notification(
            NotificationId::named("fyx-ui-dev-extensions-env".into()),
            cx,
            move |cx| cx.new(|cx| MessageNotification::new(message, cx)),
        );
    }

    // Extension progress in the status bar (design.md M.2), one item for all extensions.
    if let Some(progresses) = Progresses::global(cx) {
        let item = cx.new(|cx| ExtensionProgressStatus::new(progresses, cx));
        workspace.status_bar().update(cx, |status_bar, cx| {
            status_bar.add_left_item(item, window, cx)
        });
    }

    // Modals open in the window the user was in last.
    let workspace_id = cx.entity_id();
    registry.update(cx, |registry, _| {
        if registry.active_workspace.is_none() {
            registry.set_active_workspace(workspace_id);
        }
    });
    cx.observe_window_activation(window, {
        let registry = registry.clone();
        move |_, window, cx| {
            if window.is_window_active() {
                let workspace_id = cx.entity_id();
                registry.update(cx, |registry, _| {
                    registry.set_active_workspace(workspace_id)
                });
            }
        }
    })
    .detach();

    // A closed workspace lets its containers go at once: the registry's observers hold
    // `surfaces` until its next notify (triage-3, ASYNC 8).
    cx.on_release({
        let surfaces = surfaces.clone();
        let registry = registry.downgrade();
        move |_, cx| {
            let released = std::mem::take(&mut *surfaces.borrow_mut());
            drop(released);
            registry
                .update(cx, |registry, _| {
                    if registry.active_workspace == Some(workspace_id) {
                        registry.active_workspace = None;
                    }
                })
                .ok();
        }
    })
    .detach();

    // Not from inside `observe_new`: the workspace is still being set up there.
    cx.defer_in(window, move |workspace, window, cx| {
        add_missing_surfaces(workspace, &registry, &surfaces, window, cx);
        offer_consent(workspace, &registry, &surfaces, cx);
    });
}

/// Draws an extension notification in this workspace's notification stack (design.md M.1).
fn show_extension_notification(
    workspace: &mut Workspace,
    registry: &Entity<UiExtensionRegistry>,
    extension_id: Arc<str>,
    id: u64,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some((name, notification)) = registry.update(cx, |registry, _| {
        registry.take_pending_notification(&extension_id, id)
    }) else {
        return;
    };
    let view = cx.new(|cx| {
        crate::notifications::ExtensionNotification::new(
            id,
            extension_id.clone(),
            name,
            notification,
            registry.downgrade(),
            window,
            cx,
        )
    });
    registry.update(cx, |registry, _| {
        registry.set_notification_view(&extension_id, id, view.downgrade())
    });
    workspace.show_notification(
        NotificationId::composite::<crate::notifications::ExtensionNotification>(
            SharedString::from(format!("{extension_id}/{id}")),
        ),
        cx,
        |_| view,
    );
}

fn close_notification_view(open: OpenNotification, cx: &mut App) {
    if let Some(view) = open.view.and_then(|view| view.upgrade()) {
        view.update(cx, |view, cx| view.close_by_host(cx));
    }
}

/// One notification per installed extension waiting for consent, once per workspace: what it
/// asks for, with "Enable" and "Not now". "Not now" leaves it unloaded until the next start.
fn offer_consent(
    workspace: &mut Workspace,
    registry: &Entity<UiExtensionRegistry>,
    surfaces: &RefCell<WorkspaceSurfaces>,
    cx: &mut Context<Workspace>,
) {
    let pending: Vec<PendingConsent> = registry.read(cx).pending_consent().to_vec();
    let mut surfaces = surfaces.borrow_mut();
    surfaces
        .consent_prompts
        .retain(|id| pending.iter().any(|pending| &pending.extension_id == id));
    for pending in pending {
        if !surfaces
            .consent_prompts
            .insert(pending.extension_id.clone())
        {
            continue;
        }
        let folder = if pending.is_dev {
            format!(" Folder: {}", pending.dir.display())
        } else {
            String::new()
        };
        let message = if pending.new_capabilities.is_empty() {
            format!(
                "The UI extension {} is installed but not enabled. It asks for: {}.{folder}",
                pending.extension_id,
                crate::install::describe_capability_ids(&pending.capabilities).join("; "),
            )
        } else {
            format!(
                "An update of the UI extension {} needs approval. It now also asks for: {}.{folder}",
                pending.extension_id,
                crate::install::describe_capability_ids(&pending.new_capabilities).join("; "),
            )
        };
        let update = !pending.new_capabilities.is_empty();
        let extension_id = pending.extension_id.clone();
        let registry = registry.downgrade();
        let fs = workspace.app_state().fs.clone();
        workspace.show_notification(
            NotificationId::named(format!("fyx-ui-consent-{extension_id}").into()),
            cx,
            move |cx| {
                cx.new(|cx| {
                    MessageNotification::new(message, cx)
                        .primary_message(if update { "Approve" } else { "Enable" })
                        .primary_on_click(move |_, cx| {
                            registry
                                .update(cx, |registry, cx| {
                                    registry.enable(&extension_id, fs.clone(), cx)
                                })
                                .ok();
                        })
                        .secondary_message("Not now")
                })
            },
        );
    }
}

/// Opens a modal surface in this workspace. The modal layer holds one modal, and toggling one of
/// the same type only closes it (design.md, C, "Same-type trap"), so the active modal is hidden
/// first; it gets `dismissed(replaced)`. Showing the modal already shown does nothing.
fn open_modal(
    workspace: &mut Workspace,
    registry: &Entity<UiExtensionRegistry>,
    key: SharedString,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if workspace
        .active_modal::<ExtensionModal>(cx)
        .is_some_and(|modal| *modal.read(cx).key() == key)
    {
        return;
    }
    let Some(state) = registry
        .read(cx)
        .surface(&key)
        .map(|surface| surface.state.clone())
    else {
        return;
    };
    let weak = cx.weak_entity();
    workspace.hide_modal(window, cx);
    workspace.toggle_modal(window, cx, move |window, cx| {
        ExtensionModal::new(key, state, weak, window, cx)
    });
}

/// Brings the workspace's containers in line with the registry: adds the missing panels and
/// status items, and takes out those whose surface is gone (`remove-surface`).
fn add_missing_surfaces(
    workspace: &mut Workspace,
    registry: &Entity<UiExtensionRegistry>,
    surfaces: &RefCell<WorkspaceSurfaces>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let live: HashSet<SharedString> = registry
        .read(cx)
        .surfaces()
        .iter()
        .map(|surface| surface.info.key.clone())
        .collect();
    let gone_panels: Vec<SharedString> = surfaces
        .borrow()
        .panels
        .keys()
        .filter(|key| !live.contains(*key))
        .cloned()
        .collect();
    for key in gone_panels {
        if let Some(panel) = surfaces.borrow_mut().panels.remove(&key) {
            panel.remove_from(workspace, window, cx);
        }
    }
    let gone_items: Vec<(bool, Entity<ExtensionStatusItem>)> = {
        let mut surfaces = surfaces.borrow_mut();
        let keys: Vec<SharedString> = surfaces
            .status_items
            .keys()
            .filter(|key| !live.contains(*key))
            .cloned()
            .collect();
        keys.into_iter()
            .filter_map(|key| surfaces.status_items.remove(&key))
            .collect()
    };
    for (left, item) in gone_items {
        let group = surfaces.borrow().status_groups.get(&left).cloned();
        if let Some(group) = group {
            group.update(cx, |group, cx| group.remove(&item, cx));
        }
    }
    let loaded: Vec<(Rc<SurfaceInfo>, Entity<SurfaceState>, SurfaceKind)> = registry
        .read(cx)
        .surfaces()
        .iter()
        .map(|surface| {
            (
                surface.info.clone(),
                surface.state.clone(),
                surface.state.read(cx).surface().kind.clone(),
            )
        })
        .collect();
    for (info, state, kind) in loaded {
        match kind {
            // Modals are shown on request (`show-surface`), not docked.
            SurfaceKind::Modal(_) => {}
            SurfaceKind::Panel(_) => {
                if surfaces.borrow().panels.contains_key(&info.key) {
                    continue;
                }
                let key = info.key.clone();
                let panel = add_extension_panel(workspace, info, state, window, cx);
                surfaces.borrow_mut().panels.insert(key, panel);
            }
            SurfaceKind::StatusItem(descriptor) => {
                if surfaces.borrow().status_items.contains_key(&info.key) {
                    continue;
                }
                let item = cx.new(|cx| ExtensionStatusItem::new(state, window, cx));
                let left = descriptor.side == StatusBarSide::Left;
                surfaces
                    .borrow_mut()
                    .status_items
                    .insert(info.key.clone(), (left, item.clone()));
                let existing = surfaces.borrow().status_groups.get(&left).cloned();
                let group = match existing {
                    Some(group) => group,
                    None => {
                        let group = cx.new(|_| ExtensionStatusGroup::new(descriptor.side));
                        workspace.status_bar().update(cx, |status_bar, cx| {
                            if left {
                                status_bar.add_left_item(group.clone(), window, cx)
                            } else {
                                status_bar.add_right_item(group.clone(), window, cx)
                            }
                        });
                        surfaces
                            .borrow_mut()
                            .status_groups
                            .insert(left, group.clone());
                        group
                    }
                };
                group.update(cx, |group, cx| group.add(descriptor.priority, item, cx));
            }
        }
    }
}

/// `Workspace::toggle_panel_focus` for one panel instance (that one picks the first panel of a
/// type, and panels beyond `PANEL_SLOTS` share a type).
fn toggle_panel_focus(
    workspace: &mut Workspace,
    panel_id: gpui::EntityId,
    panel_focus: &gpui::FocusHandle,
    surface_focus: &gpui::FocusHandle,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if panel_focus.contains_focused(window, cx) {
        workspace.focus_center_pane(window, cx);
        if WorkspaceSettings::get_global(cx).close_panel_on_toggle {
            for dock in workspace.all_docks() {
                if dock.read(cx).panel_index_for_id(panel_id).is_some() {
                    dock.update(cx, |dock, cx| dock.set_open(false, window, cx));
                }
            }
        }
        return;
    }
    for dock in workspace.all_docks() {
        if let Some(index) = dock.read(cx).panel_index_for_id(panel_id) {
            dock.update(cx, |dock, cx| {
                dock.activate_panel(index, window, cx);
                dock.set_open(true, window, cx);
            });
            surface_focus.focus(window, cx);
            break;
        }
    }
    cx.notify();
}

fn dev_extension_paths() -> Vec<PathBuf> {
    let Some(value) = std::env::var_os(DEV_EXTENSIONS_ENV) else {
        return Vec::new();
    };
    std::env::split_paths(&value)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

fn load_dev_extensions(cx: &mut App) {
    for path in dev_extension_paths() {
        log::info!("fyx:ui: loading dev extension {}", path.display());
        cx.spawn(async move |cx| {
            if let Err(error) = load_dev_extension(path.clone(), cx).await {
                log::error!(
                    "fyx:ui: failed to load dev extension {}: {error:#}",
                    path.display()
                );
            }
        })
        .detach();
    }
}

async fn load_dev_extension(path: PathBuf, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let fs = cx.update(|cx| <dyn Fs>::global(cx));
    let reload_path = path.clone();
    // The file to watch for hot reload.
    let component = if path.is_dir() {
        UiManifest::read(fs.as_ref(), &path)
            .await
            .ok()
            .flatten()
            .map(|(_, manifest)| path.join(manifest.component))
    } else {
        Some(path.clone())
    };
    let checker = cx.update(|cx| {
        UiExtensionRegistry::global(cx).map(|registry| registry.read(cx).capability_check())
    });
    let options = LoadOptions {
        capability_check: checker,
        // A `FYX_UI_DEV_EXTENSIONS` entry is a dev extension.
        allow_staged: cx.update(|cx| staged_allowed(true, cx)),
        ..LoadOptions::default()
    };
    let mut declared = Vec::new();
    let (extension, pushed) = if path.is_dir() {
        // An extension directory: `extension.toml` with a `[fyx_ui]` table names the component.
        if let Some((_, manifest)) = UiManifest::read(fs.as_ref(), &path).await? {
            declared = manifest.capabilities;
        }
        UiExtension::load_from_dir_with_options(&path, options, fs.clone(), cx)
            .await?
            .with_context(|| format!("{}: extension.toml has no [fyx_ui] table", path.display()))?
    } else {
        // A bare component, for quick iteration: the id is the file name.
        let (extension_id, bytes) = cx
            .background_spawn(async move {
                let extension_id = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .context("dev extension path has no usable name")?
                    .to_lowercase()
                    .replace('_', "-");
                let bytes = std::fs::read(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                anyhow::Ok((extension_id, bytes))
            })
            .await?;
        UiExtension::load_with_options(extension_id.into(), bytes, options, cx).await?
    };
    log::info!(
        "fyx:ui: loaded {} with {} surface(s)",
        extension.extension_id(),
        extension.surfaces().len()
    );
    cx.update(|cx| {
        let registry =
            UiExtensionRegistry::global(cx).context("ui_ext_host::init was not called")?;
        registry.update(cx, |registry, cx| {
            let extension_id = extension.extension_id().clone();
            let source = if reload_path.is_dir() {
                ExtensionSource::Directory(reload_path.clone())
            } else {
                ExtensionSource::File(reload_path.clone())
            };
            registry.claim_extension_id(&extension_id, source)?;
            registry.consent.declare(&extension_id, declared);
            registry.register_extension(extension, pushed, cx);
            if let Some(component) = component {
                registry.watch_component(
                    extension_id,
                    component,
                    Reload::DevPath(reload_path),
                    fs,
                    cx,
                );
            }
            anyhow::Ok(())
        })?;
        anyhow::Ok(())
    })
}
