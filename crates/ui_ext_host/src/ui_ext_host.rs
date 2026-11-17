//! Native containers and renderer for `fyx:ui` extensions.
//!
//! - [`SurfaceState`] retains one surface's tree, routes events to the extension one call at a
//!   time and applies the replies and pushed updates.
//! - [`SurfaceView`] draws that tree with GPUI and `ui` elements and theme tokens. It never
//!   calls the extension while rendering.
//! - [`ExtensionPanel`] (a dock [`workspace::Panel`]) and [`ExtensionStatusItem`] (a
//!   [`workspace::StatusItemView`]) host a view.
//! - [`UiExtensionRegistry`] holds the loaded surfaces and adds containers to every workspace.
//!
//! For the sprint, extensions are loaded from [`DEV_EXTENSIONS_ENV`] only.

mod backend;
mod install;
mod keyboard;
mod panel;
mod registry;
mod surface;
mod view;

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
pub use install::{InstallDevUiExtension, describe_capabilities, install_dev_ui_extension};
pub use keyboard::{Activate, FocusNext, FocusPrevious, SelectPageDown, SelectPageUp};
pub use panel::{
    ExtensionPanel, ExtensionStatusGroup, ExtensionStatusItem, PANEL_SLOTS, SHARED_SLOT,
    SurfaceInfo,
};
pub use registry::{DEV_EXTENSIONS_ENV, LoadedSurface, UiExtensionRegistry};
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
    let registry = UiExtensionRegistry::global(cx)?;
    let capabilities = registry.read(cx).capabilities(extension_id, cx);
    if capabilities.is_empty() {
        return None;
    }
    Some(
        ui::h_flex()
            .gap_1()
            .children(capabilities.into_iter().map(ui::Chip::new))
            .into_any_element(),
    )
}

pub fn init(cx: &mut App) {
    registry::init(cx);
    keyboard::intercept_keys(cx);
}
