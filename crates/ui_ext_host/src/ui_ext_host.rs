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
mod decorations;
mod install;
mod keyboard;
mod panel;
mod registry;
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

pub fn init(cx: &mut App) {
    registry::init(cx);
    keyboard::intercept_keys(cx);
    decorations::init(cx);
}
