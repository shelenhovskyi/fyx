//! fyx: hook that lets Fyx's Agent Sessions board (plan 7.9, `terminal_host`) bring a thread to the
//! front without depending on the agent panel. `agent_ui` installs the focuser.

use std::rc::Rc;

use gpui::{App, Entity, Global, Window};

use crate::AcpThread;

/// Shows `thread` in its window's agent panel and focuses it.
pub type ThreadFocuser = Rc<dyn Fn(&Entity<AcpThread>, &mut Window, &mut App)>;

struct GlobalThreadFocuser(ThreadFocuser);

impl Global for GlobalThreadFocuser {}

pub fn set_thread_focuser(focuser: ThreadFocuser, cx: &mut App) {
    cx.set_global(GlobalThreadFocuser(focuser));
}

/// Whether a focuser is installed; without one, [`focus_thread`] does nothing.
pub fn can_focus_thread(cx: &App) -> bool {
    cx.has_global::<GlobalThreadFocuser>()
}

pub fn focus_thread(thread: &Entity<AcpThread>, window: &mut Window, cx: &mut App) {
    let Some(focuser) = cx
        .try_global::<GlobalThreadFocuser>()
        .map(|global| global.0.clone())
    else {
        return;
    };
    focuser(thread, window, cx);
}
