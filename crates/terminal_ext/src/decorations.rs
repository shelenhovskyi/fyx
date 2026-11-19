//! The hook `terminal_view` calls to decorate a terminal's tab. `terminal_host` provides it; with no
//! provider set (tests, other binaries) the tab looks as it does upstream.

use std::rc::Rc;

use gpui::{AnyElement, App, Global};

use crate::TerminalExt;

pub trait TerminalDecorations: 'static {
    /// An element drawn after the tab's icon, e.g. the status icon with a tooltip naming its source.
    fn tab_indicator(&self, terminal: &TerminalExt, cx: &App) -> Option<AnyElement>;
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
