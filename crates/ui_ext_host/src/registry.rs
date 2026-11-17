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
use ui_ext::{StatusBarSide, Surface, SurfaceKind, Update};
use ui_ext_wasm::{
    CAPABILITY_PANEL, CAPABILITY_STATUS_BAR, PrecompiledCache, PushedUpdate, UiExtension,
    UiManifest,
};
use workspace::{
    Workspace, WorkspaceSettings,
    notifications::{NotificationId, simple_message_notification::MessageNotification},
};

use crate::{
    ToggleExtensionPanel,
    backend::UiBackend,
    consent::{self, ConsentRecord, ConsentStore},
    install_store::InstallStore,
    panel::{
        AnyExtensionPanel, ExtensionStatusGroup, ExtensionStatusItem, SurfaceInfo,
        add_extension_panel,
    },
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
}

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
        for surface in surfaces {
            let info = SurfaceInfo::new(
                backend.extension_id(),
                surface,
                FIRST_ACTIVATION_PRIORITY + self.next_activation_priority,
            );
            if let Some(loaded) = self.surface_of(backend.extension_id(), &surface.id) {
                log::info!("fyx:ui: reloading surface {}", info.key);
                loaded.state.update(cx, |state, cx| {
                    state.replace_backend(backend.clone(), surface.clone(), cx)
                });
                continue;
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
        cx.notify();
    }

    /// Registers a WASM extension and applies the updates it pushes outside of any call (the
    /// pushes made during a call come back with its reply), batched per wake-up: a burst of
    /// pushes becomes one repaint on the next frame.
    pub fn register_extension(
        &mut self,
        extension: UiExtension,
        mut pushed: mpsc::UnboundedReceiver<PushedUpdate>,
        cx: &mut Context<Self>,
    ) {
        let extension_id = extension.extension_id().clone();
        let surfaces = extension.surfaces().to_vec();
        self.register_backend(Rc::new(extension), &surfaces, cx);
        cx.spawn(async move |this, cx| {
            while let Some(first) = pushed.next().await {
                let mut batch = vec![first];
                while let Ok(next) = pushed.try_recv() {
                    batch.push(next);
                }
                let applied = this.update(cx, |this, cx| {
                    for PushedUpdate { surface_id, update } in batch {
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
                let mut found = find_ui_extensions(&dir, fs.as_ref()).await;
                if let Some(store) = &store {
                    found.extend(find_store_extensions(store, fs.as_ref()).await);
                }
                // Read again on every round: a change during the scan asks for another round.
                let dev_paths = this
                    .update(cx, |this, _| this.dev_paths.clone())
                    .unwrap_or_default();
                for path in dev_paths {
                    let source = ExtensionSource::Directory(path.clone());
                    match found_at(path.clone(), true, source, None, fs.as_ref()).await {
                        Some(extension) => found.push(extension),
                        None => log::error!(
                            "fyx:ui: fyx.extensions.dev_paths: {} is not a built UI extension",
                            path.display()
                        ),
                    }
                }
                let Ok((to_load, consent_file)) = this.update(cx, |this, cx| {
                    (this.plan_scan(found, cx), this.consent.path_to_load())
                }) else {
                    return;
                };
                if let Some(path) = consent_file {
                    let records = consent::read_records(&path, fs.as_ref()).await;
                    this.update(cx, |this, _| this.consent.set_loaded(records))
                        .ok();
                }
                for found in to_load {
                    let trusted = this
                        .update(cx, |this, _| this.trust_installed)
                        .unwrap_or(false);
                    if !trusted {
                        let record = match ConsentRecord::current(
                            &found.dir,
                            &found.capabilities,
                            &found.component,
                            found.is_dev,
                            fs.as_ref(),
                            &cx.background_executor(),
                        )
                        .await
                        {
                            Ok(record) => record,
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
                            continue;
                        }
                    }
                    let loaded = UiExtension::load_from_dir_with_cache(
                        &found.dir,
                        found.cache.clone(),
                        fs.clone(),
                        cx,
                    )
                    .await;
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
                                    return;
                                }
                                this.installed
                                    .insert(found.extension_id.clone(), found.fingerprint.clone());
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
                cx.spawn(async move |this, cx| {
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
                })
                .detach();
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
        if self.consent.allows(&found.extension_id, &record) {
            cx.notify();
            return true;
        }
        log::info!(
            "fyx:ui: {} is installed but not enabled (as it is now); waiting for the user",
            found.extension_id
        );
        self.pending_consent.push(PendingConsent {
            extension_id: found.extension_id.clone(),
            dir: found.dir.clone(),
            installed_dir: installed_dir.to_path_buf(),
            capabilities: found.capabilities.clone(),
            is_dev: found.is_dev,
            record,
        });
        cx.notify();
        false
    }

    /// Which found extensions need loading; stops the surfaces of uninstalled ones.
    fn plan_scan(&mut self, found: Vec<Found>, cx: &mut Context<Self>) -> Vec<Found> {
        self.pending_consent.retain(|pending| {
            found
                .iter()
                .any(|found| found.extension_id == pending.extension_id)
        });
        let gone: Vec<Arc<str>> = self
            .installed
            .keys()
            .filter(|id| !found.iter().any(|found| &found.extension_id == *id))
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
        for surface in self.surfaces_of(extension_id) {
            if surface.surface_id.as_ref() != except_surface {
                surface
                    .state
                    .update(cx, |state, cx| state.fail(message.clone(), false, cx));
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
            None => log::error!(
                "fyx:ui: {extension_id} pushed an update for unknown surface {surface_id:?}"
            ),
        }
    }
}

/// The UI extensions in `dir`: subdirectories (or links to them, as dev installs are) whose
/// `extension.toml` has a `[fyx_ui]` table. Others are regular Zed extensions and are skipped.
async fn find_ui_extensions(dir: &Path, fs: &dyn Fs) -> Vec<Found> {
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
        if let Some(extension) = found_at(extension_dir, is_dev, source, None, fs).await {
            found.push(extension);
        }
    }
    found
}

/// The active version of every extension in the install store.
async fn find_store_extensions(store: &InstallStore, fs: &dyn Fs) -> Vec<Found> {
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
        match found_at(dir, false, ExtensionSource::Store, Some(cache), fs).await {
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
) -> Option<Found> {
    let (extension_id, manifest) = match UiManifest::read(fs, &extension_dir).await {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return None,
        Err(error) => {
            log::debug!("fyx:ui: skipping {}: {error:#}", extension_dir.display());
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

/// A surface's way back to the registry, for the other surfaces of its extension.
struct RegistryLink {
    registry: WeakEntity<UiExtensionRegistry>,
    extension_id: Arc<str>,
}

impl SurfaceLink for RegistryLink {
    fn route_push(&self, push: PushedUpdate, cx: &mut App) {
        self.registry
            .update(cx, |registry, cx| {
                registry.apply_pushed(&self.extension_id, &push.surface_id, push.update, cx)
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

    load_dev_extensions(cx);
    follow_dev_paths_setting(cx);
    watch_installed_extensions(cx);
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
    status_items: HashSet<SharedString>,
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

    // Not from inside `observe_new`: the workspace is still being set up there.
    cx.defer_in(window, move |workspace, window, cx| {
        add_missing_surfaces(workspace, &registry, &surfaces, window, cx);
        offer_consent(workspace, &registry, &surfaces, cx);
    });
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
        let message = format!(
            "The UI extension {} is installed but not enabled. It asks for: {}.{folder}",
            pending.extension_id,
            crate::install::describe_capability_ids(&pending.capabilities).join("; "),
        );
        let extension_id = pending.extension_id.clone();
        let registry = registry.downgrade();
        let fs = workspace.app_state().fs.clone();
        workspace.show_notification(
            NotificationId::named(format!("fyx-ui-consent-{extension_id}").into()),
            cx,
            move |cx| {
                cx.new(|cx| {
                    MessageNotification::new(message, cx)
                        .primary_message("Enable")
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

fn add_missing_surfaces(
    workspace: &mut Workspace,
    registry: &Entity<UiExtensionRegistry>,
    surfaces: &RefCell<WorkspaceSurfaces>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
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
            SurfaceKind::Panel(_) => {
                if surfaces.borrow().panels.contains_key(&info.key) {
                    continue;
                }
                let key = info.key.clone();
                let panel = add_extension_panel(workspace, info, state, window, cx);
                surfaces.borrow_mut().panels.insert(key, panel);
            }
            SurfaceKind::StatusItem(descriptor) => {
                if !surfaces.borrow_mut().status_items.insert(info.key.clone()) {
                    continue;
                }
                let item = cx.new(|cx| ExtensionStatusItem::new(state, window, cx));
                let left = descriptor.side == StatusBarSide::Left;
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
    let (extension, pushed) = if path.is_dir() {
        // An extension directory: `extension.toml` with a `[fyx_ui]` table names the component.
        UiExtension::load_from_dir(&path, fs.clone(), cx)
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
        UiExtension::load(extension_id.into(), bytes, cx).await?
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
