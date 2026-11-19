//! Containers for extension surfaces: a dock panel and a status-bar item.

use std::{
    rc::Rc,
    sync::{LazyLock, Mutex, OnceLock},
};

use collections::HashMap;
use gpui::{
    Action, App, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable, IntoElement,
    Pixels, Render, SharedString, WeakEntity, Window, prelude::*, px,
};
use theme::ActiveTheme as _;
use ui::{IconName, h_flex, v_flex};
use ui_ext::v0_2::{DockPosition as SurfaceDockPosition, StatusBarSide, SurfaceKind};
use workspace::{
    HideStatusItem, ItemHandle, StatusItemView, Workspace,
    dock::{DockPosition, Panel, PanelEvent},
};

use crate::{ToggleExtensionPanel, surface::SurfaceState, view::SurfaceView};

/// Panel types with a name of their own. `Panel::persistent_name` and `Panel::panel_key` are
/// static (the dock calls them without a context), so a name per panel needs a type per panel:
/// `ExtensionPanel<SLOT>`. Each panel surface gets a slot for the whole process, by key, and the
/// slot's name is `fyx-ui:<extension id>/<surface id>`, so sizes and the open panel persist per
/// surface across restarts. Panels beyond this many share `SHARED_SLOT` and its one name.
pub const PANEL_SLOTS: usize = 16;
/// The slot of panels beyond `PANEL_SLOTS`.
pub const SHARED_SLOT: usize = PANEL_SLOTS;

static SLOT_NAMES: [OnceLock<&'static str>; PANEL_SLOTS] = [const { OnceLock::new() }; PANEL_SLOTS];
static SLOT_KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The slot of the panel surface `key`, the same every time for the process. Its name is leaked
/// once per slot (at most `PANEL_SLOTS` strings).
pub(crate) fn panel_slot(key: &str) -> usize {
    let mut keys = SLOT_KEYS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(slot) = keys.iter().position(|known| known == key) {
        return slot;
    }
    if keys.len() >= PANEL_SLOTS {
        return SHARED_SLOT;
    }
    let slot = keys.len();
    keys.push(key.to_string());
    SLOT_NAMES[slot].get_or_init(|| format!("fyx-ui:{key}").leak());
    slot
}

pub(crate) fn slot_name(slot: usize) -> Option<&'static str> {
    SLOT_NAMES.get(slot)?.get().copied()
}

/// The dock's tooltip API wants `&'static str`. Titles are interned: each distinct title is
/// leaked once per process, however often its extension loads or reloads.
fn static_title(title: &str) -> &'static str {
    static TITLES: LazyLock<Mutex<HashMap<String, &'static str>>> =
        LazyLock::new(|| Mutex::new(HashMap::default()));
    let mut titles = TITLES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(title) = titles.get(title) {
        return title;
    }
    let leaked: &'static str = title.to_string().leak();
    titles.insert(title.to_string(), leaked);
    leaked
}

/// What a container needs to know about a surface, fixed when the extension loads.
pub struct SurfaceInfo {
    /// `"<extension id>/<surface id>"`, unique among loaded surfaces.
    pub key: SharedString,
    /// The surface title, interned (see `static_title`).
    pub title: &'static str,
    /// For a panel: its type slot (see `PANEL_SLOTS`).
    pub slot: Option<usize>,
    pub icon: IconName,
    pub position: DockPosition,
    pub default_size: Pixels,
    /// Docks need a distinct priority per panel; assigned in load order, above every built-in.
    pub activation_priority: u32,
}

impl SurfaceInfo {
    pub fn new(
        extension_id: &str,
        surface: &ui_ext::v0_2::Surface,
        activation_priority: u32,
    ) -> Self {
        let (icon, position, default_size) = match &surface.kind {
            SurfaceKind::Panel(panel) => (
                crate::icons::resolve_icon(&panel.icon).static_name(),
                match panel.position {
                    SurfaceDockPosition::Left => DockPosition::Left,
                    SurfaceDockPosition::Right => DockPosition::Right,
                    SurfaceDockPosition::Bottom => DockPosition::Bottom,
                },
                px(panel.default_size.clamp(120, 2000) as f32),
            ),
            SurfaceKind::StatusItem(_) | SurfaceKind::Modal(_) => {
                (IconName::Blocks, DockPosition::Right, px(320.))
            }
        };
        let title = if surface.title.is_empty() {
            &surface.id
        } else {
            &surface.title
        };
        let key = format!("{extension_id}/{}", surface.id);
        let slot = match surface.kind {
            SurfaceKind::Panel(_) => Some(panel_slot(&key)),
            SurfaceKind::StatusItem(_) | SurfaceKind::Modal(_) => None,
        };
        Self {
            key: key.into(),
            title: static_title(title),
            slot,
            icon,
            position,
            default_size,
            activation_priority,
        }
    }
}

/// A dock panel showing one extension surface. `SLOT` gives it a name of its own; see
/// `PANEL_SLOTS`.
pub struct ExtensionPanel<const SLOT: usize> {
    info: Rc<SurfaceInfo>,
    view: Entity<SurfaceView>,
    position: DockPosition,
    focus_handle: FocusHandle,
}

impl<const SLOT: usize> ExtensionPanel<SLOT> {
    pub fn new(
        info: Rc<SurfaceInfo>,
        state: Entity<SurfaceState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = cx.new(|cx| SurfaceView::new(state, false, window, cx));
        // Its progress bar (design.md M.2) is drawn here, outside the cached surface view.
        if let Some(progresses) = crate::progress::Progresses::global(cx) {
            cx.observe(&progresses, |_, _, cx| cx.notify()).detach();
        }
        // A panel that goes away (its workspace closed) no longer shows the surface; without
        // this its ticks would keep running for a window that is gone (triage-3, ASYNC 8).
        let container = cx.entity_id();
        cx.on_release(move |this: &mut Self, cx| {
            let state = this.view.read(cx).state().clone();
            state.update(cx, |state, cx| state.set_visible(container, false, cx));
        })
        .detach();
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

impl<const SLOT: usize> EventEmitter<PanelEvent> for ExtensionPanel<SLOT> {}

impl<const SLOT: usize> Focusable for ExtensionPanel<SLOT> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<const SLOT: usize> Render for ExtensionPanel<SLOT> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context("ExtensionPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().panel_background)
            .children({
                let state = self.view.read(cx).state().read(cx);
                crate::progress::panel_bar(state.extension_id(), &state.surface().id, cx)
            })
            // A cached view: a redraw caused by anything else in the window (a caret blinking in
            // another editor) reuses the surface's last frame instead of rendering its whole
            // tree again (triage-3 #5). The surface's root is `size_full`, as the cache needs.
            .child(
                gpui::AnyView::from(self.view.clone())
                    .cached(gpui::StyleRefinement::default().size_full()),
            )
    }
}

impl<const SLOT: usize> Panel for ExtensionPanel<SLOT> {
    /// The surface view's own focus target, so Tab moves into the surface right away.
    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        self.view.read(cx).focus_handle.clone()
    }

    // Per slot, so per surface (see `PANEL_SLOTS`).
    fn persistent_name() -> &'static str {
        slot_name(SLOT).unwrap_or("Extension Panel")
    }

    fn panel_key() -> &'static str {
        slot_name(SLOT).unwrap_or("FyxExtensionPanel")
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

    /// The dock reports when this panel becomes the shown one (active in an open dock) or stops
    /// being it: the surface's ticks wait while it is hidden everywhere (design.md, B).
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        let container = cx.entity_id();
        let state = self.view.read(cx).state().clone();
        state.update(cx, |state, cx| state.set_visible(container, active, cx));
    }
}

/// An extension panel of any slot, as the registry keeps it.
pub(crate) trait AnyExtensionPanel {
    fn panel_id(&self) -> EntityId;
    /// The panel's root focus handle (containment checks).
    fn panel_focus(&self, cx: &App) -> Option<FocusHandle>;
    /// Where focus goes when the panel is activated: the surface view.
    fn surface_focus(&self, cx: &App) -> Option<FocusHandle>;
    /// Takes the panel out of the workspace's docks (`remove-surface`, uninstall).
    fn remove_from(
        &self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    );
}

impl<const SLOT: usize> AnyExtensionPanel for WeakEntity<ExtensionPanel<SLOT>> {
    fn panel_id(&self) -> EntityId {
        self.entity_id()
    }

    fn panel_focus(&self, cx: &App) -> Option<FocusHandle> {
        Some(self.upgrade()?.focus_handle(cx))
    }

    fn surface_focus(&self, cx: &App) -> Option<FocusHandle> {
        Some(self.upgrade()?.read(cx).view.read(cx).focus_handle.clone())
    }

    fn remove_from(
        &self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if let Some(panel) = self.upgrade() {
            workspace.remove_panel(&panel, window, cx);
        }
    }
}

/// Creates the panel of `info`'s slot type and adds it to the workspace's dock.
pub(crate) fn add_extension_panel(
    workspace: &mut Workspace,
    info: Rc<SurfaceInfo>,
    state: Entity<SurfaceState>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Box<dyn AnyExtensionPanel> {
    fn add<const SLOT: usize>(
        workspace: &mut Workspace,
        info: Rc<SurfaceInfo>,
        state: Entity<SurfaceState>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Box<dyn AnyExtensionPanel> {
        let panel = cx.new(|cx| ExtensionPanel::<SLOT>::new(info, state, window, cx));
        workspace.add_panel(panel.clone(), window, cx);
        Box::new(panel.downgrade())
    }
    macro_rules! by_slot {
        ($($slot:literal)*) => {
            match info.slot.unwrap_or(SHARED_SLOT) {
                $($slot => add::<$slot>(workspace, info, state, window, cx),)*
                _ => add::<SHARED_SLOT>(workspace, info, state, window, cx),
            }
        };
    }
    by_slot!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15)
}

/// The extension status items of one side of the status bar, in one status bar entry, ordered by
/// their `priority`: higher sits closer to the edge (leftmost on the left, rightmost on the
/// right); equal priorities keep load order.
pub struct ExtensionStatusGroup {
    side: StatusBarSide,
    items: Vec<(i32, Entity<ExtensionStatusItem>)>,
    /// Each item notifies when it is hidden or shown again, which changes the group's layout.
    observers: HashMap<EntityId, gpui::Subscription>,
}

impl ExtensionStatusGroup {
    pub fn new(side: StatusBarSide) -> Self {
        Self {
            side,
            items: Vec::new(),
            observers: HashMap::default(),
        }
    }

    pub fn add(
        &mut self,
        priority: i32,
        item: Entity<ExtensionStatusItem>,
        cx: &mut Context<Self>,
    ) {
        let index = match self.side {
            StatusBarSide::Left => self.items.partition_point(|(other, _)| *other >= priority),
            StatusBarSide::Right => self.items.partition_point(|(other, _)| *other <= priority),
        };
        self.observers
            .insert(item.entity_id(), cx.observe(&item, |_, _, cx| cx.notify()));
        self.items.insert(index, (priority, item));
        cx.notify();
    }

    pub fn remove(&mut self, item: &Entity<ExtensionStatusItem>, cx: &mut Context<Self>) {
        self.items.retain(|(_, existing)| existing != item);
        self.observers.remove(&item.entity_id());
        cx.notify();
    }

    /// The priorities in display order, left to right.
    pub fn priorities(&self) -> Vec<i32> {
        self.items.iter().map(|(priority, _)| *priority).collect()
    }
}

impl Render for ExtensionStatusGroup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex().gap_2().children(
            self.items
                .iter()
                .filter(|(_, item)| !item.read(cx).is_hidden(cx))
                .map(|(_, item)| item.clone()),
        )
    }
}

impl StatusItemView for ExtensionStatusGroup {
    fn set_active_pane_item(
        &mut self,
        _: Option<&dyn ItemHandle>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    /// Present only while extensions with status items are loaded, which the user controls.
    fn hide_setting(&self, _: &App) -> Option<HideStatusItem> {
        None
    }
}

/// A status-bar item showing one extension surface inline.
pub struct ExtensionStatusItem {
    view: Entity<SurfaceView>,
    /// `"<extension id>/<surface id>"`, as `fyx.extensions.hidden_status_items` names it.
    key: SharedString,
    _settings: gpui::Subscription,
}

impl ExtensionStatusItem {
    pub fn new(state: Entity<SurfaceState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let key = {
            let state = state.read(cx);
            format!("{}/{}", state.extension_id(), state.surface().id).into()
        };
        Self {
            view: cx.new(|cx| SurfaceView::new(state, true, window, cx)),
            key,
            _settings: cx.observe_global::<settings::SettingsStore>(|_, cx| cx.notify()),
        }
    }

    pub fn is_hidden(&self, cx: &App) -> bool {
        crate::install::status_item_hidden(&self.key, cx)
    }
}

impl Render for ExtensionStatusItem {
    /// Right-click offers "Hide Button", as built-in status items do (triage-3, UX F22).
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_hidden(cx) {
            return gpui::Empty.into_any_element();
        }
        let view = self.view.clone();
        let key = self.key.clone();
        ui::right_click_menu(SharedString::from(format!("fyx-ui-status-menu-{key}")))
            .trigger(move |_, _, _| view)
            .menu(move |window, cx| {
                let key = key.clone();
                ui::ContextMenu::build(window, cx, move |menu, _, _| {
                    workspace::add_hide_button_entry(
                        menu,
                        HideStatusItem::new(move |settings| {
                            let hidden = settings
                                .fyx
                                .get_or_insert_default()
                                .extensions
                                .get_or_insert_default()
                                .hidden_status_items
                                .get_or_insert_default();
                            if !hidden.iter().any(|hidden| *hidden == *key) {
                                hidden.push(key.to_string());
                            }
                        }),
                    )
                })
            })
            .into_any_element()
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
