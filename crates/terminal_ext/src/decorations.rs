//! The hooks `terminal_view` calls to decorate a terminal's tab, its right-click menu, the
//! terminal panel's "+" menu and the sidebar beside the terminal panel's panes.
//! `terminal_host` provides them; with no provider set (tests, other binaries) both look as they do
//! upstream.

use std::rc::Rc;

use gpui::{AnyElement, AnyView, App, EntityId, Global, SharedString, Window};

use crate::TerminalExt;

pub trait TerminalDecorations: 'static {
    /// An element drawn after the tab's icon, e.g. the status icon with a tooltip naming its source.
    fn tab_indicator(&self, terminal: &TerminalExt, cx: &App) -> Option<AnyElement>;

    /// Entries added after the terminal panel's "+" menu's own (profiles §B.1, accounts §C.6).
    fn new_terminal_menu(&self, _window: &Window, _cx: &App) -> Vec<MenuEntry> {
        Vec::new()
    }

    /// Entries added to a terminal's right-click menu ("Share with <extension>…", §C.3).
    fn terminal_menu(&self, _terminal: &TerminalExt, _cx: &App) -> Vec<MenuEntry> {
        Vec::new()
    }

    /// A view drawn to the right of the terminal panel's panes of the workspace `workspace` (the
    /// agent sessions list). The view decides its own width; it is empty while hidden.
    fn panel_sidebar(&self, _workspace: EntityId, _cx: &App) -> Option<AnyView> {
        None
    }
}

/// An entry of the "+" menu, built by [`TerminalDecorations::new_terminal_menu`].
#[derive(Clone)]
pub enum MenuEntry {
    Separator,
    Header(SharedString),
    Entry {
        label: SharedString,
        on_select: Rc<dyn Fn(&mut Window, &mut App)>,
    },
    Toggle {
        label: SharedString,
        checked: bool,
        on_select: Rc<dyn Fn(&mut Window, &mut App)>,
    },
}

struct GlobalTerminalDecorations(Rc<dyn TerminalDecorations>);

impl Global for GlobalTerminalDecorations {}

pub fn set_decorations(decorations: impl TerminalDecorations, cx: &mut App) {
    cx.set_global(GlobalTerminalDecorations(Rc::new(decorations)));
}

pub fn tab_indicator(terminal: &TerminalExt, cx: &App) -> Option<AnyElement> {
    let decorations = cx.try_global::<GlobalTerminalDecorations>()?.0.clone();
    decorations.tab_indicator(terminal, cx)
}

pub fn new_terminal_menu(window: &Window, cx: &App) -> Vec<MenuEntry> {
    cx.try_global::<GlobalTerminalDecorations>()
        .map(|decorations| decorations.0.clone())
        .map(|decorations| decorations.new_terminal_menu(window, cx))
        .unwrap_or_default()
}

pub fn terminal_menu(terminal: &TerminalExt, cx: &App) -> Vec<MenuEntry> {
    cx.try_global::<GlobalTerminalDecorations>()
        .map(|decorations| decorations.0.clone())
        .map(|decorations| decorations.terminal_menu(terminal, cx))
        .unwrap_or_default()
}

pub fn panel_sidebar(workspace: EntityId, cx: &App) -> Option<AnyView> {
    let decorations = cx.try_global::<GlobalTerminalDecorations>()?.0.clone();
    decorations.panel_sidebar(workspace, cx)
}
