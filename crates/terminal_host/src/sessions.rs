//! Every terminal view with a status ("agent sessions"), how to jump to one, and pop-up
//! notifications when one becomes blocked or done while its window is in the background.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EntityId, Global, WeakEntity, Window,
    WindowHandle,
};
use terminal_ext::osc::StatusState;
use terminal::Terminal;
use terminal_ext::{TerminalId, TerminalStatus};
use terminal_view::TerminalView;
use terminal_view::terminal_panel::TerminalPanel;
use workspace::{MultiWorkspace, Pane, Workspace};

use crate::notification::SessionNotification;
use crate::tab_status::tooltip_text;

/// A terminal pops up at most once in this interval.
const MIN_POPUP_INTERVAL: Duration = Duration::from_secs(10);
/// Pop-ups open at once; the oldest closes first.
const MAX_POPUPS: usize = 3;

struct Entry {
    view: WeakEntity<TerminalView>,
    window: AnyWindowHandle,
    state: Option<StatusState>,
    updated: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Sessions {
    entries: Vec<Entry>,
    last_popup: HashMap<EntityId, Instant>,
    popups: Vec<WindowHandle<SessionNotification>>,
}

struct GlobalSessions(Entity<Sessions>);

impl Global for GlobalSessions {}

pub(crate) fn init(cx: &mut App) {
    let sessions = cx.new(|_| Sessions::default());
    cx.set_global(GlobalSessions(sessions));
}

pub(crate) fn global(cx: &App) -> Option<Entity<Sessions>> {
    cx.try_global::<GlobalSessions>()
        .map(|sessions| sessions.0.clone())
}

/// Whether a change from `previous` to `current` is worth a pop-up.
pub(crate) fn should_notify(previous: Option<StatusState>, current: StatusState) -> bool {
    matches!(current, StatusState::Blocked | StatusState::Done) && previous != Some(current)
}

/// Sort key for the list: needs attention first, then blocked, error, working, done, idle, then
/// the most recent.
pub(crate) fn sort_key(status: &TerminalStatus) -> (bool, u8, Instant) {
    let rank = match status.state {
        StatusState::Blocked => 4,
        StatusState::Error => 3,
        StatusState::Working => 2,
        StatusState::Done => 1,
        StatusState::Idle | StatusState::Clear => 0,
    };
    (status.attention, rank, status.updated)
}

impl Sessions {
    pub(crate) fn register(&mut self, view: WeakEntity<TerminalView>, window: AnyWindowHandle) {
        self.entries.retain(|entry| entry.view.is_upgradable());
        self.entries.push(Entry {
            view,
            window,
            state: None,
            updated: None,
        });
    }

    /// Called on a terminal's `Wakeup`. Notifies the list when the status changed, and pops up when
    /// the terminal became blocked or done while its window is not active.
    /// `view` is being updated (this runs from its terminal's subscription), so its title comes
    /// from `terminal` and `view` is never read.
    pub(crate) fn status_maybe_changed(
        &mut self,
        view: &Entity<TerminalView>,
        terminal: &Entity<Terminal>,
        status: Option<TerminalStatus>,
        window_active: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.view.entity_id() == view.entity_id())
        else {
            return;
        };
        let state = status.as_ref().map(|status| status.state);
        let updated = status.as_ref().map(|status| status.updated);
        if (entry.state, entry.updated) == (state, updated) {
            return;
        }
        let previous = std::mem::replace(&mut entry.state, state);
        entry.updated = updated;
        let main_window = entry.window;
        cx.notify();

        let Some(status) = status else {
            return;
        };
        if window_active
            || !should_notify(previous, status.state)
            || !crate::settings::notifications_enabled(cx)
        {
            return;
        }
        let now = Instant::now();
        if self
            .last_popup
            .get(&view.entity_id())
            .is_some_and(|last| now.duration_since(*last) < MIN_POPUP_INTERVAL)
        {
            return;
        }
        self.last_popup.insert(view.entity_id(), now);
        let terminal_title = terminal.read(cx).title(true);
        self.pop_up(&status, terminal_title, view.downgrade(), main_window, cx);
    }

    fn pop_up(
        &mut self,
        status: &TerminalStatus,
        terminal_title: String,
        view: WeakEntity<TerminalView>,
        main_window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(screen) = cx.primary_display() else {
            return;
        };
        while self.popups.len() >= MAX_POPUPS {
            let oldest = self.popups.remove(0);
            oldest
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        self.popups
            .retain(|popup| popup.update(cx, |_, _, _| ()).is_ok());
        let (title, meta) = tooltip_text(status);
        let caption = format!("{terminal_title} · {meta}");
        let state = status.state;
        let options = SessionNotification::window_options(screen, cx);
        match cx.open_window(options, |window, cx| {
            cx.new(|cx| {
                SessionNotification::new(title, caption, state, view, main_window, window, cx)
            })
        }) {
            Ok(popup) => self.popups.push(popup),
            Err(error) => log::error!("fyx: cannot open a terminal notification: {error}"),
        }
        main_window
            .update(cx, |_, window, _| window.request_attention())
            .ok();
    }

    /// The live view of a terminal and its window.
    pub(crate) fn find_terminal(
        &self,
        id: TerminalId,
        cx: &App,
    ) -> Option<(Entity<TerminalView>, AnyWindowHandle)> {
        self.entries.iter().find_map(|entry| {
            let view = entry.view.upgrade()?;
            (view.read(cx).terminal().read(cx).fyx.id() == id).then_some((view, entry.window))
        })
    }

    /// Live terminal views with a status, sorted for the list.
    pub(crate) fn list(&self, cx: &App) -> Vec<(Entity<TerminalView>, TerminalStatus)> {
        let mut list: Vec<_> = self
            .entries
            .iter()
            .filter_map(|entry| {
                let view = entry.view.upgrade()?;
                let status = view.read(cx).terminal().read(cx).fyx.status()?;
                Some((view, status))
            })
            .collect();
        list.sort_by_key(|(_, status)| std::cmp::Reverse(sort_key(status)));
        list
    }
}

enum Location {
    Center,
    TerminalPanel(Entity<Pane>, usize),
}

fn locate(workspace: &Workspace, view: &Entity<TerminalView>, cx: &App) -> Option<Location> {
    if workspace
        .panes()
        .iter()
        .any(|pane| pane.read(cx).index_for_item(view).is_some())
    {
        return Some(Location::Center);
    }
    let panel = workspace.panel::<TerminalPanel>(cx)?;
    panel.read(cx).panes().into_iter().find_map(|pane| {
        let index = pane.read(cx).index_for_item(view)?;
        Some(Location::TerminalPanel(pane.clone(), index))
    })
}

/// The pane in `workspace` (center or terminal panel) that holds `view`.
pub(crate) fn pane_of(
    workspace: &Workspace,
    view: &Entity<TerminalView>,
    cx: &App,
) -> Option<Entity<Pane>> {
    match locate(workspace, view, cx)? {
        Location::Center => workspace
            .panes()
            .iter()
            .find(|pane| pane.read(cx).index_for_item(view).is_some())
            .cloned(),
        Location::TerminalPanel(pane, _) => Some(pane),
    }
}

pub(crate) fn workspace_contains(
    workspace: &Workspace,
    view: &Entity<TerminalView>,
    cx: &App,
) -> bool {
    locate(workspace, view, cx).is_some()
}

/// Shows and focuses `view` in `workspace`: activates its tab in the center or opens the terminal
/// panel on it.
pub(crate) fn reveal(
    workspace: &mut Workspace,
    view: &Entity<TerminalView>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    match locate(workspace, view, cx) {
        Some(Location::Center) => {
            workspace.activate_item(view, true, true, window, cx);
        }
        Some(Location::TerminalPanel(pane, index)) => {
            workspace.focus_panel::<TerminalPanel>(window, cx);
            pane.update(cx, |pane, cx| {
                pane.activate_item(index, true, true, window, cx)
            });
        }
        None => {}
    }
}

/// Activates `window` and reveals `view` in whichever of its workspaces holds it.
pub(crate) fn reveal_in_window(
    main_window: AnyWindowHandle,
    view: WeakEntity<TerminalView>,
    cx: &mut App,
) {
    let Some(view) = view.upgrade() else {
        return;
    };
    main_window
        .update(cx, |_, window, cx| {
            window.activate_window();
            let Some(Some(multi_workspace)) = window.root::<MultiWorkspace>() else {
                return;
            };
            let Some(workspace) = multi_workspace
                .read(cx)
                .workspaces()
                .find(|workspace| workspace_contains(workspace.read(cx), &view, cx))
                .cloned()
            else {
                return;
            };
            multi_workspace.update(cx, |multi_workspace, cx| {
                multi_workspace.activate(workspace.clone(), None, window, cx)
            });
            workspace.update(cx, |workspace, cx| reveal(workspace, &view, window, cx));
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use terminal_ext::StatusSource;
    use terminal_ext::osc::Protocol;

    use super::*;

    fn status(state: StatusState, attention: bool, age: u64) -> TerminalStatus {
        TerminalStatus {
            state,
            kind: None,
            label: None,
            detail: None,
            progress: None,
            attention,
            source: StatusSource::Program {
                protocol: Protocol::Osc7501,
                app: None,
            },
            updated: Instant::now() - Duration::from_secs(age),
        }
    }

    #[test]
    fn attention_first_then_urgency_then_recency() {
        let mut list = [
            ("idle", status(StatusState::Idle, false, 0)),
            ("working", status(StatusState::Working, false, 0)),
            ("old blocked", status(StatusState::Blocked, false, 9)),
            ("new blocked", status(StatusState::Blocked, false, 1)),
            ("done, unseen", status(StatusState::Done, true, 5)),
        ];
        list.sort_by_key(|(_, status)| std::cmp::Reverse(sort_key(status)));
        let order: Vec<_> = list.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            order,
            [
                "done, unseen",
                "new blocked",
                "old blocked",
                "working",
                "idle"
            ]
        );
    }

    #[test]
    fn pop_ups_only_on_entering_blocked_or_done() {
        assert!(should_notify(
            Some(StatusState::Working),
            StatusState::Blocked
        ));
        assert!(should_notify(Some(StatusState::Working), StatusState::Done));
        assert!(should_notify(None, StatusState::Done));
        assert!(!should_notify(Some(StatusState::Done), StatusState::Done));
        assert!(!should_notify(
            Some(StatusState::Idle),
            StatusState::Working
        ));
        assert!(!should_notify(
            Some(StatusState::Working),
            StatusState::Error
        ));
    }
}
