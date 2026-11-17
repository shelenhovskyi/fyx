//! Loaded UI extensions and their surfaces, and how they reach every workspace.
//!
//! Panels are registered dynamically: when an extension finishes loading, each open workspace
//! gets a panel (or status item) per surface, and new workspaces get them on creation. Nothing
//! is listed statically in `crates/zed`.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use anyhow::Context as _;
use collections::{HashMap, HashSet};
use extension_host::wasm_host::fyx_ui::{PushedUpdate, UiExtension};
use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, AsyncApp, Context, Entity, Focusable as _, Global, SharedString,
    WeakEntity, Window,
};
use settings::Settings as _;
use ui_ext::{StatusBarSide, Surface, SurfaceKind, Update};
use workspace::{Workspace, WorkspaceSettings};

use crate::{
    ToggleExtensionPanel,
    backend::UiBackend,
    panel::{ExtensionPanel, ExtensionStatusItem, SurfaceInfo},
    surface::SurfaceState,
};

/// Opt-in for the sprint: a list of `fyx:ui` components to load at startup, separated like
/// `PATH` (`;` on Windows, `:` elsewhere). Each entry is a `.wasm` file, or a directory holding
/// `fyx_ui.wasm` or `extension.wasm`. The extension id is the file (or directory) name.
pub const DEV_EXTENSIONS_ENV: &str = "FYX_UI_DEV_EXTENSIONS";

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
    pub fn register_backend(
        &mut self,
        backend: Rc<dyn UiBackend>,
        surfaces: &[Surface],
        cx: &mut Context<Self>,
    ) {
        for surface in surfaces {
            let info = SurfaceInfo::new(
                backend.extension_id(),
                surface,
                FIRST_ACTIVATION_PRIORITY + self.next_activation_priority,
            );
            if self.surface(&info.key).is_some() {
                log::error!("fyx:ui: surface {} is already loaded, skipping", info.key);
                continue;
            }
            self.next_activation_priority += 1;
            let state = cx.new(|cx| SurfaceState::new(backend.clone(), surface.clone(), cx));
            self.surfaces.push(LoadedSurface {
                info: Rc::new(info),
                state,
            });
        }
        cx.notify();
    }

    /// Registers a WASM extension and applies the updates it pushes, batched per wake-up: a
    /// burst of pushes becomes one repaint on the next frame.
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
                state.apply_update(update, cx);
            }),
            None => log::error!(
                "fyx:ui: {extension_id} pushed an update for unknown surface {surface_id:?}"
            ),
        }
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
            focus_handle.focus(window, cx);
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
    let (extension_id, bytes) = cx
        .background_spawn(async move {
            let wasm_path = if path.is_dir() {
                ["fyx_ui.wasm", "extension.wasm"]
                    .iter()
                    .map(|name| path.join(name))
                    .find(|candidate| candidate.is_file())
                    .with_context(|| {
                        format!("{} has no fyx_ui.wasm or extension.wasm", path.display())
                    })?
            } else {
                path.clone()
            };
            let extension_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .context("dev extension path has no usable name")?
                .replace('_', "-");
            let bytes = std::fs::read(&wasm_path)
                .with_context(|| format!("failed to read {}", wasm_path.display()))?;
            anyhow::Ok((extension_id, bytes))
        })
        .await?;
    let (extension, pushed) = UiExtension::load(extension_id.into(), bytes, cx).await?;
    log::info!(
        "fyx:ui: loaded {} with {} surface(s)",
        extension.extension_id(),
        extension.surfaces().len()
    );
    cx.update(|cx| {
        let registry =
            UiExtensionRegistry::global(cx).context("ui_ext_host::init was not called")?;
        registry.update(cx, |registry, cx| {
            registry.register_extension(extension, pushed, cx)
        });
        anyhow::Ok(())
    })
}
