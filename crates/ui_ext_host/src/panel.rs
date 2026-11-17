//! Containers for extension surfaces: a dock panel and a status-bar item.

use std::rc::Rc;

use gpui::{
    Action, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Pixels,
    Render, SharedString, Window, prelude::*, px,
};
use theme::ActiveTheme as _;
use ui::{IconName, v_flex};
use ui_ext::{DockPosition as SurfaceDockPosition, SurfaceKind};
use workspace::{
    HideStatusItem, ItemHandle, StatusItemView,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::{
    ToggleExtensionPanel,
    surface::SurfaceState,
    view::{SurfaceView, icon_name},
};

/// What a container needs to know about a surface, fixed when the extension loads.
pub struct SurfaceInfo {
    /// `"<extension id>/<surface id>"`, unique among loaded surfaces.
    pub key: SharedString,
    /// The surface title; the dock's tooltip API wants `&'static str`, so it is leaked once per
    /// loaded surface (a handful per session).
    pub title: &'static str,
    pub icon: IconName,
    pub position: DockPosition,
    pub default_size: Pixels,
    /// Docks need a distinct priority per panel; assigned in load order, above every built-in.
    pub activation_priority: u32,
}

impl SurfaceInfo {
    pub fn new(extension_id: &str, surface: &ui_ext::Surface, activation_priority: u32) -> Self {
        let (icon, position, default_size) = match &surface.kind {
            SurfaceKind::Panel(panel) => (
                icon_name(&panel.icon),
                match panel.position {
                    SurfaceDockPosition::Left => DockPosition::Left,
                    SurfaceDockPosition::Right => DockPosition::Right,
                    SurfaceDockPosition::Bottom => DockPosition::Bottom,
                },
                px(panel.default_size.clamp(120, 2000) as f32),
            ),
            SurfaceKind::StatusItem(_) => (IconName::Blocks, DockPosition::Right, px(320.)),
        };
        let title = if surface.title.is_empty() {
            surface.id.clone()
        } else {
            surface.title.clone()
        };
        Self {
            key: format!("{extension_id}/{}", surface.id).into(),
            title: title.leak(),
            icon,
            position,
            default_size,
            activation_priority,
        }
    }
}

/// A dock panel showing one extension surface.
pub struct ExtensionPanel {
    info: Rc<SurfaceInfo>,
    view: Entity<SurfaceView>,
    position: DockPosition,
    focus_handle: FocusHandle,
}

impl ExtensionPanel {
    pub fn new(
        info: Rc<SurfaceInfo>,
        state: Entity<SurfaceState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.new(|cx| SurfaceView::new(state, false, window, cx));
        Self {
            position: info.position,
            info,
            view,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn key(&self) -> &SharedString {
        &self.info.key
    }

    pub fn view(&self) -> &Entity<SurfaceView> {
        &self.view
    }
}

impl EventEmitter<PanelEvent> for ExtensionPanel {}

impl Focusable for ExtensionPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExtensionPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("ExtensionPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().panel_background)
            .child(self.view.clone())
    }
}

impl Panel for ExtensionPanel {
    // Every extension panel shares these, because the trait makes them static. Consequences:
    // panel sizes persist per workspace for all extension panels together, and a restored dock
    // reopens the first extension panel. A per-instance key would need a larger workspace change.
    fn persistent_name() -> &'static str {
        "Extension Panel"
    }

    fn panel_key() -> &'static str {
        "FyxExtensionPanel"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        self.position
    }

    fn position_is_valid(&self, _: DockPosition) -> bool {
        true
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        // Kept for this session only; the extension's descriptor decides the next time.
        self.position = position;
        cx.notify();
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        self.info.default_size
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(self.info.icon)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<&'static str> {
        Some(self.info.title)
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleExtensionPanel {
            key: self.info.key.to_string(),
        })
    }

    fn activation_priority(&self) -> u32 {
        self.info.activation_priority
    }
}

/// A status-bar item showing one extension surface inline.
pub struct ExtensionStatusItem {
    view: Entity<SurfaceView>,
}

impl ExtensionStatusItem {
    pub fn new(state: Entity<SurfaceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            view: cx.new(|cx| SurfaceView::new(state, true, window, cx)),
        }
    }
}

impl Render for ExtensionStatusItem {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.view.clone()
    }
}

impl StatusItemView for ExtensionStatusItem {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// Present only while its extension is loaded, which the user controls.
    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}
