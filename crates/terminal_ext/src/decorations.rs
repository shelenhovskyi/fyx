//! The hooks `terminal_view` calls to decorate a terminal's tab and the terminal panel's "+" menu.
//! `terminal_host` provides them; with no provider set (tests, other binaries) both look as they do
//! upstream.

use std::rc::Rc;

use gpui::{AnyElement, App, Global, SharedString, Window};

use crate::TerminalExt;

pub trait TerminalDecorations: 'static {
    /// An element drawn after the tab's icon, e.g. the status icon with a tooltip naming its source.
    fn tab_indicator(&self, terminal: &TerminalExt, cx: &App) -> Option<AnyElement>;

    /// Entries added after the terminal panel's "+" menu's own (profiles §B.1, accounts §C.6).
    fn new_terminal_menu(&self, _window: &Window, _cx: &App) -> Vec<MenuEntry> {
        Vec::new()
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
