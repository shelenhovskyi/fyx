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

mod backend;
mod consent;
mod decorations;
mod inspector;
mod install;
pub mod install_store;
mod keyboard;
mod panel;
mod registry;
mod store_actions;
mod surface;
mod view;

#[cfg(test)]
mod decoration_tests;
#[cfg(test)]
mod primitive_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wasm_tests;

use gpui::{Action, App};
use schemars::JsonSchema;
use serde::Deserialize;

pub use backend::UiBackend;
pub use decorations::{
    DEV_DECORATIONS_ENV, DecorationId, DecorationPlacement, DecorationTarget, Decorations,
    text_tree,
};
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
pub use registry::{DEV_EXTENSIONS_ENV, ExtensionSource, LoadedSurface, UiExtensionRegistry};
pub use store_actions::{
    InstallUiExtensionFromFile, RestartUiExtension, RevokeUiExtensionPermission,
    RollBackUiExtension, TogglePinUiExtension, UninstallUiExtension,
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
        .map(|(capability, tier, granted)| match tier {
            Tier::T0 => format!("{capability} · T0"),
            _ if granted => format!("{capability} · {tier} granted"),
            _ => format!("{capability} · {tier} not granted"),
        })
        .collect();
    if labels.is_empty() {
        labels = registry
            .capabilities(extension_id, cx)
            .into_iter()
            .map(|capability| format!("{capability} · T0"))
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
}
