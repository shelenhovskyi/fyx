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
use extension_host::{
    ExtensionStore,
    wasm_host::fyx_ui::{PushedUpdate, UiExtension, UiManifest},
};
use fs::{Fs, MTime, PathEvent};
use futures::{FutureExt as _, StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, Focusable as _, Global, SharedString, Task,
    WeakEntity, Window,
};
use settings::Settings as _;
use ui_ext::{StatusBarSide, Surface, SurfaceKind, Update};
use workspace::{Workspace, WorkspaceSettings};

use crate::{
    ToggleExtensionPanel,
    backend::UiBackend,
    panel::{ExtensionPanel, ExtensionStatusItem, SurfaceInfo},
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
    pub info: Rc<SurfaceInfo>,
    pub state: Entity<SurfaceState>,
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
}

/// How a watched dev extension is loaded again.
#[derive(Clone, Debug)]
enum Reload {
    /// Linked into this installed-extensions directory: rescan it.
    Installed(PathBuf),
    /// A `FYX_UI_DEV_EXTENSIONS` entry: load it again.
    DevPath(PathBuf),
}

/// What tells a reinstalled component from an unchanged one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint {
    mtime: MTime,
    len: u64,
}

/// A UI extension found in the installed-extensions directory.
struct Found {
    dir: PathBuf,
    extension_id: Arc<str>,
    fingerprint: Fingerprint,
    component: PathBuf,
    /// A link (Zed's dev install): its component is watched for changes.
    is_dev: bool,
}

struct GlobalRegistry(Entity<UiExtensionRegistry>);

impl Global for GlobalRegistry {}

impl UiExtensionRegistry {
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalRegistry>()
            .map(|global| global.0.clone())
    }

    pub fn surfaces(&self) -> &[LoadedSurface] {
        &self.surfaces
    }

    pub fn surface(&self, key: &str) -> Option<&LoadedSurface> {
        self.surfaces.iter().find(|surface| surface.info.key == key)
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
        let prefix = format!("{}/", backend.extension_id());
        for loaded in &self.surfaces {
            let Some(surface_id) = loaded.info.key.strip_prefix(prefix.as_str()) else {
                continue;
            };
            if !surfaces.iter().any(|surface| surface.id == surface_id) {
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
            if let Some(loaded) = self.surface(&info.key) {
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
        if self.scanning {
            self.rescan_requested = true;
            return;
        }
        self.scanning = true;
        cx.spawn(async move |this, cx| {
            loop {
                let found = find_ui_extensions(&dir, fs.as_ref()).await;
                let Ok(to_load) = this.update(cx, |this, cx| this.plan_scan(found, cx)) else {
                    return;
                };
                for found in to_load {
                    let loaded = UiExtension::load_from_dir(&found.dir, fs.clone(), cx).await;
                    match loaded {
                        Ok(Some((extension, pushed))) => {
                            log::info!(
                                "fyx:ui: loaded installed extension {} with {} surface(s)",
                                extension.extension_id(),
                                extension.surfaces().len()
                            );
                            this.update(cx, |this, cx| {
                                this.installed
                                    .insert(found.extension_id.clone(), found.fingerprint);
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
                        Err(error) => log::error!(
                            "fyx:ui: failed to load installed extension {}: {error:#}",
                            found.dir.display()
                        ),
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
                cx.spawn(async move |_, cx| {
                    if let Err(error) = load_dev_extension(path.clone(), cx).await {
                        log::error!(
                            "fyx:ui: failed to reload dev extension {}: {error:#}",
                            path.display()
                        );
                    }
                })
                .detach();
            }
        }
    }

    /// Which found extensions need loading; stops the surfaces of uninstalled ones.
    fn plan_scan(&mut self, found: Vec<Found>, cx: &mut Context<Self>) -> Vec<Found> {
        let gone: Vec<Arc<str>> = self
            .installed
            .keys()
            .filter(|id| !found.iter().any(|found| &found.extension_id == *id))
            .cloned()
            .collect();
        for extension_id in gone {
            self.installed.remove(&extension_id);
            self.watches.remove(&extension_id);
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
        let prefix = format!("{extension_id}/");
        for surface in &self.surfaces {
            let Some(surface_id) = surface.info.key.strip_prefix(prefix.as_str()) else {
                continue;
            };
            if surface_id != except_surface {
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
        let key = format!("{extension_id}/{surface_id}");
        match self.surface(&key) {
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
        let manifest = match UiManifest::read(fs, &extension_dir).await {
            Ok(Some(manifest)) => manifest,
            Ok(None) => continue,
            Err(error) => {
                log::debug!("fyx:ui: skipping {}: {error:#}", extension_dir.display());
                continue;
            }
        };
        let (extension_id, manifest) = manifest;
        let component = extension_dir.join(&manifest.component);
        let is_dev = fs
            .metadata(&extension_dir)
            .await
            .ok()
            .flatten()
            .is_some_and(|metadata| metadata.is_symlink);
        match fs.metadata(&component).await {
            Ok(Some(metadata)) => found.push(Found {
                dir: extension_dir,
                extension_id,
                fingerprint: Fingerprint {
                    mtime: metadata.mtime,
                    len: metadata.len,
                },
                component,
                is_dev,
            }),
            Ok(None) | Err(_) => log::error!(
                "fyx:ui: {extension_id}: component {} is missing (build it first)",
                component.display()
            ),
        }
    }
    found
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
}

pub fn init(cx: &mut App) {
    let registry = cx.new(|_| UiExtensionRegistry::default());
    cx.set_global(GlobalRegistry(registry));

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        attach_to_workspace(workspace, window, cx);
    })
    .detach();

    load_dev_extensions(cx);
    watch_installed_extensions(cx);
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
    panels: HashMap<SharedString, WeakEntity<ExtensionPanel>>,
    status_items: HashSet<SharedString>,
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

    workspace.register_action(|workspace, _: &crate::InstallDevUiExtension, window, cx| {
        crate::install::prompt_and_install(workspace, window, cx)
    });
    workspace.register_action({
        let surfaces = surfaces.clone();
        move |workspace, action: &ToggleExtensionPanel, window, cx| {
            let panel = surfaces
                .borrow()
                .panels
                .get(action.key.as_str())
                .and_then(WeakEntity::upgrade);
            match panel {
                Some(panel) => toggle_panel_focus(workspace, &panel, window, cx),
                None => log::debug!("fyx:ui: no extension panel {:?}", action.key),
            }
        }
    });

    cx.observe_in(&registry, window, {
        let surfaces = surfaces.clone();
        move |workspace, registry, window, cx| {
            add_missing_surfaces(workspace, &registry, &surfaces, window, cx)
        }
    })
    .detach();

    // Not from inside `observe_new`: the workspace is still being set up there.
    cx.defer_in(window, move |workspace, window, cx| {
        add_missing_surfaces(workspace, &registry, &surfaces, window, cx)
    });
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
                let panel = cx.new(|cx| ExtensionPanel::new(info, state, window, cx));
                surfaces.borrow_mut().panels.insert(key, panel.downgrade());
                workspace.add_panel(panel, window, cx);
            }
            SurfaceKind::StatusItem(descriptor) => {
                if !surfaces.borrow_mut().status_items.insert(info.key.clone()) {
                    continue;
                }
                // The status bar keeps insertion order; `priority` is not honoured yet.
                let item = cx.new(|cx| ExtensionStatusItem::new(state, window, cx));
                workspace
                    .status_bar()
                    .update(cx, |status_bar, cx| match descriptor.side {
                        StatusBarSide::Left => status_bar.add_left_item(item, window, cx),
                        StatusBarSide::Right => status_bar.add_right_item(item, window, cx),
                    });
            }
        }
    }
}

/// `Workspace::toggle_panel_focus` for one panel instance (that one picks the first panel of a
/// type, and every extension panel has the same type).
fn toggle_panel_focus(
    workspace: &mut Workspace,
    panel: &Entity<ExtensionPanel>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let focus_handle = panel.focus_handle(cx);
    if focus_handle.contains_focused(window, cx) {
        workspace.focus_center_pane(window, cx);
        if WorkspaceSettings::get_global(cx).close_panel_on_toggle {
            for dock in workspace.all_docks() {
                if dock
                    .read(cx)
                    .panel_index_for_id(panel.entity_id())
                    .is_some()
                {
                    dock.update(cx, |dock, cx| dock.set_open(false, window, cx));
                }
            }
        }
        return;
    }
    for dock in workspace.all_docks() {
        if let Some(index) = dock.read(cx).panel_index_for_id(panel.entity_id()) {
            dock.update(cx, |dock, cx| {
                dock.activate_panel(index, window, cx);
                dock.set_open(true, window, cx);
            });
            panel
                .read(cx)
                .view()
                .read(cx)
                .focus_handle
                .clone()
                .focus(window, cx);
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
        });
        anyhow::Ok(())
    })
}
