//! Native containers and renderer for `fyx:ui` extensions.
//!
//! - [`SurfaceState`] retains one surface's tree, routes events to the extension one call at a
//!   time and applies the replies and pushed updates.
//! - [`SurfaceView`] draws that tree with GPUI and `ui` elements and theme tokens. It never
//!   calls the extension while rendering.
//! - [`ExtensionPanel`] (a dock [`workspace::Panel`]) and [`ExtensionStatusItem`] (a
//!   [`workspace::StatusItemView`]) host a view.
//! - [`UiExtensionRegistry`] holds the loaded surfaces and adds containers to every workspace.
//! - [`Decorations`] draws trees as editor blocks anchored to buffer lines (layer 4).
//!
//! For the sprint, extensions are loaded from [`DEV_EXTENSIONS_ENV`] only, and decorations from
//! [`DEV_DECORATIONS_ENV`].

mod agent_observe;
pub mod approvals;
mod backend;
mod canvas;
mod canvas_texture;
mod choice;
mod consent;
pub mod consent_dialog;
pub mod consent_model;
#[cfg(test)]
mod consent_ui_tests;
mod decorations;
pub mod fyx_extensions_page;
mod icons;
pub mod images;
mod inspector;
mod install;
pub mod install_store;
mod keyboard;
mod modal;
pub mod notifications;
mod panel;
pub mod progress;
mod registry;
pub mod registry_service;
#[cfg(test)]
mod registry_service_tests;
mod store_actions;
mod surface;
mod view;
mod workspace_edit;
mod worktrees;

#[cfg(test)]
mod decoration_tests;
#[cfg(test)]
mod decoration_wasm_tests;
#[cfg(test)]
mod primitive_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wasm_tests;

use gpui::{Action, App, Window};
use schemars::JsonSchema;
use serde::Deserialize;

pub use backend::UiBackend;
pub use decorations::{
    DEV_DECORATIONS_ENV, DecorationId, DecorationPath, DecorationPlacement, DecorationTarget,
    Decorations, text_tree,
};
pub use fyx_extensions_page::{OpenFyxExtensions, fyx_extensions_button};
pub use inspector::{ExtensionInspector, InspectorSnapshot, ToggleExtensionInspector};
pub use install::{
    FyxExtensionsSettings, InstallDevUiExtension, RemoveDevUiExtension, check_dev_folder,
    describe_capabilities,
};
pub use keyboard::{Activate, FocusNext, FocusPrevious, SelectPageDown, SelectPageUp};
pub use panel::{
    ExtensionPanel, ExtensionStatusGroup, ExtensionStatusItem, PANEL_SLOTS, SHARED_SLOT,
    SurfaceInfo,
};
pub use registry::{
    DEV_EXTENSIONS_ENV, ExtensionSource, LoadedSurface, RegistryEvent, UiExtensionRegistry,
};
pub use store_actions::{
    InstallUiExtensionFromFile, OpenExtensionPanel, RestartUiExtension,
    RevokeUiExtensionPermission, RollBackUiExtension, TogglePinUiExtension, UninstallUiExtension,
};
pub use surface::{Phase, SurfaceState};
pub use view::{SurfaceView, TYPING_DEBOUNCE};

/// Focuses (or unfocuses) the extension panel with this key, `"<extension id>/<surface id>"`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, Action)]
#[action(namespace = fyx_ui)]
#[serde(deny_unknown_fields)]
pub struct ToggleExtensionPanel {
    pub key: String,
}

/// Chips for the extensions page: the `fyx:ui` capabilities of a loaded UI extension, next to the
/// card's other feature chips (the one `fyx:` hook in `extensions_ui`). `None` for an extension
/// that is not a loaded UI extension.
pub fn capability_chips(extension_id: &str, cx: &App) -> Option<gpui::AnyElement> {
    use gpui::{IntoElement as _, ParentElement as _, Styled as _};
    use ui_ext_wasm::permissions::Tier;
    let registry = UiExtensionRegistry::global(cx)?;
    let registry = registry.read(cx);
    // What the manifest declared, with the tier; extensions loaded without one (a bare component)
    // show what their surfaces use.
    let mut labels: Vec<String> = registry
        .declared_capabilities(extension_id)
        .into_iter()
        .map(|(capability, tier, granted)| {
            let label = install::capability_chip_label(&capability);
            match tier {
                Tier::T0 => label,
                _ if granted => format!("{label} (approved)"),
                _ => format!("{label} (not approved)"),
            }
        })
        .collect();
    if labels.is_empty() {
        labels = registry
            .capabilities(extension_id, cx)
            .into_iter()
            .map(install::capability_chip_label)
            .collect();
    }
    if labels.is_empty() {
        return None;
    }
    Some(
        ui::h_flex()
            .gap_1()
            .children(labels.into_iter().map(ui::Chip::new))
            .into_any_element(),
    )
}

pub fn init(cx: &mut App) {
    registry::init(cx);
    keyboard::intercept_keys(cx);
    decorations::init(cx);
    progress::init(cx);
    images::init(cx);
    fyx_extensions_page::init(cx);
    // A `.fyxext` dropped on a pane opens the install dialog.
    cx.set_global(workspace::FyxPackageDrop(
        store_actions::install_dropped_package,
    ));
}

/// Draws a 0.4 `terminal` node (§B.2, plan 5.9): the view of the pseudo-terminal that
/// extension `ext` opened in its panel under `handle`, or `None` when that handle is not one of
/// its in-panel terminals. W1's `terminal_host::extension_terminal` resolves the handle; the
/// caller wraps it in a terminal view.
pub type TerminalNodeViews = std::rc::Rc<
    dyn Fn(&str, ui_ext::terminal::TerminalHandle, &mut Window, &mut App) -> Option<gpui::AnyView>,
>;

struct GlobalTerminalNodeViews(TerminalNodeViews);

impl gpui::Global for GlobalTerminalNodeViews {}

/// Sets how panels draw their `terminal` nodes (plan 5.9). Set from `crates/zed/src/main.rs`
/// next to [`set_terminal_host`]; without it a terminal node draws a placeholder.
pub fn set_terminal_node_views(views: TerminalNodeViews, cx: &mut App) {
    cx.set_global(GlobalTerminalNodeViews(views));
}

/// The view for a `terminal` node, if [`set_terminal_node_views`] was called and the handle
/// is one of `extension_id`'s in-panel terminals.
pub(crate) fn terminal_node_view(
    extension_id: &str,
    handle: ui_ext::terminal::TerminalHandle,
    window: &mut Window,
    cx: &mut App,
) -> Option<gpui::AnyView> {
    let views = cx.try_global::<GlobalTerminalNodeViews>()?.0.clone();
    views(extension_id, handle, window, cx)
}

/// The terminal host the runtime gets for 0.4's `terminals` imports (W1's `terminal_host`).
struct GlobalTerminalHost(std::sync::Arc<dyn ui_ext::terminal::TerminalHost>);

impl gpui::Global for GlobalTerminalHost {}

/// Sets the terminal host passed to every extension loaded from now on, as
/// `LoadOptions::terminals`. Call it before the first registry scan (W1 does, from
/// `crates/zed/src/main.rs`); a later call applies to later loads only.
pub fn set_terminal_host(host: std::sync::Arc<dyn ui_ext::terminal::TerminalHost>, cx: &mut App) {
    cx.set_global(GlobalTerminalHost(host));
}

/// The terminal host set by [`set_terminal_host`], if any.
pub(crate) fn terminal_host(
    cx: &App,
) -> Option<std::sync::Arc<dyn ui_ext::terminal::TerminalHost>> {
    cx.try_global::<GlobalTerminalHost>()
        .map(|host| host.0.clone())
}
