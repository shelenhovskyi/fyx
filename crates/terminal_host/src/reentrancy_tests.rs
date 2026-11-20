//! Regression tests for the re-entrancy crashes of 2026-11-20 (`tasks/reviews/crashes-2026-11-20.md`):
//! each runs its handler the way gpui does, inside the update of a real `MultiWorkspace` window
//! (and of the panel or view whose listener or subscription it is).

use std::path::Path;

use gpui::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, VisualTestContext, WindowHandle,
};
use project::trusted_worktrees::{PathTrust, TrustedWorktrees};
use project::{FakeFs, Project};
use serde_json::json;
use settings::SettingsStore;
use terminal::{Terminal, TerminalBuilder};
use terminal_ext::ExtensionStatus;
use terminal_ext::osc::StatusState;
use terminal_view::TerminalView;
use terminal_view::terminal_panel::TerminalPanel;
use util::paths::PathStyle;
use workspace::dock::Panel as _;
use workspace::{MultiWorkspace, Workspace};

use crate::sessions_panel::AgentSessionsPanel;

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = SettingsStore::test(cx);
        cx.set_global(store);
        // The notification window's options read the release channel.
        let version = "0.0.0"
            .parse()
            .unwrap_or_else(|error| panic!("bad version: {error}"));
        release_channel::init(version, cx);
        theme_settings::init(theme::LoadThemes::JustBase, cx);
        editor::init(cx);
        terminal_view::init(cx);
        crate::init(cx);
    });
}

async fn open_window(
    root: &str,
    fs: &std::sync::Arc<FakeFs>,
    cx: &mut TestAppContext,
) -> (WindowHandle<MultiWorkspace>, Entity<Workspace>) {
    let project = Project::test(fs.clone(), [Path::new(root)], cx).await;
    let window = cx.add_window(|window, cx| MultiWorkspace::test_new(project, window, cx));
    let workspace = window
        .read_with(cx, |multi_workspace, _| multi_workspace.workspace().clone())
        .unwrap_or_else(|error| panic!("no workspace: {error}"));
    (window, workspace)
}

fn display_terminal(cx: &mut TestAppContext) -> Entity<Terminal> {
    cx.update(|cx| {
        let builder = TerminalBuilder::new_display_only(
            Default::default(),
            terminal::terminal_settings::AlternateScroll::On,
            None,
            0,
            cx.background_executor(),
            PathStyle::local(),
        );
        cx.new(|cx| builder.subscribe(cx))
    })
}

/// Opens `terminal` as a tab of `workspace` (its terminal panel when it has one).
fn add_tab(
    window: WindowHandle<MultiWorkspace>,
    workspace: &Entity<Workspace>,
    terminal: &Entity<Terminal>,
    cx: &mut TestAppContext,
) -> Entity<TerminalView> {
    window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                crate::pseudo::add_tab(workspace, terminal.clone(), None, false, window, cx)
            })
        })
        .unwrap_or_else(|error| panic!("window closed: {error}"));
    cx.run_until_parked();
    let id = cx.read(|cx| terminal.read(cx).fyx.id());
    cx.read(|cx| {
        crate::sessions::global(cx)
            .and_then(|sessions| sessions.read(cx).find_terminal(id, cx))
            .map(|(view, _)| view)
    })
    .unwrap_or_else(|| panic!("the tab's view is not an agent session"))
}

/// Crash 1: a "+" menu entry runs inside its window's update; reading that window through its
/// handle panicked ("attempted to read a window that is already on the stack").
#[gpui::test]
async fn trusted_worktree_check_inside_the_clicked_window(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/trusted", json!({ "a.txt": "" })).await;
    fs.insert_tree("/other", json!({ "b.txt": "" })).await;
    cx.update(|cx| project::trusted_worktrees::init(Default::default(), cx));
    let (clicked, workspace) = open_window("/trusted", &fs, cx).await;
    cx.update(|cx| {
        let project = workspace.read(cx).project().clone();
        let store = project.read(cx).worktree_store();
        let worktrees = project
            .read(cx)
            .visible_worktrees(cx)
            .map(|worktree| PathTrust::Worktree(worktree.read(cx).id()))
            .collect();
        if let Some(trust) = TrustedWorktrees::try_get_global(cx) {
            trust.update(cx, |trust, cx| trust.trust(&store, worktrees, cx));
        }
    });
    // A menu click updates the window, not its root view: the handler reads the `MultiWorkspace`.
    let clicked: AnyWindowHandle = clicked.into();
    // Another window is read through its handle, the clicked one through `window`.
    let (_other, _) = open_window("/other", &fs, cx).await;
    cx.run_until_parked();

    let trusted = clicked
        .update(cx, |_, window, cx| {
            crate::created::in_trusted_worktree(Path::new("/trusted/a.txt"), Some(window), cx)
        })
        .unwrap_or_else(|error| panic!("window closed: {error}"));
    assert!(trusted, "the clicked window's trusted worktree counts");
    let untrusted = clicked
        .update(cx, |_, window, cx| {
            crate::created::in_trusted_worktree(Path::new("/other/b.txt"), Some(window), cx)
        })
        .unwrap_or_else(|error| panic!("window closed: {error}"));
    assert!(!untrusted, "the other window's worktree is not trusted");
}

/// Crash 2: a terminal turning blocked in an inactive window. `tab_status`'s `subscribe_in` runs
/// inside the view's (and window's) update, and the pop-up read the view.
#[gpui::test]
async fn blocked_terminal_in_an_inactive_window_pops_up(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root", json!({})).await;
    let (window, workspace) = open_window("/root", &fs, cx).await;
    let terminal = display_terminal(cx);
    add_tab(window, &workspace, &terminal, cx);
    VisualTestContext::from_window(window.into(), cx).deactivate_window();

    let windows_before = cx.update(|cx| cx.windows().len());
    cx.update(|cx| {
        terminal.read(cx).fyx.status_handle().set_extension_status(
            "acme",
            Some(ExtensionStatus {
                state: StatusState::Blocked,
                label: "Needs approval".into(),
                detail: None,
                progress: None,
                attention: true,
            }),
        );
        terminal.update(cx, |_, cx| cx.emit(terminal::Event::Wakeup));
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| cx.windows().len()),
        windows_before + 1,
        "a notification window opened"
    );
}

/// Crash 3: a row's click listener runs inside the sessions panel's update; revealing the
/// terminal in the same dock deactivated that panel synchronously.
#[gpui::test]
async fn session_row_click_reveals_the_terminal_in_the_same_dock(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root", json!({})).await;
    let (window, workspace) = open_window("/root", &fs, cx).await;
    let sessions_panel = window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let terminal_panel = cx.new(|cx| TerminalPanel::new(workspace, window, cx));
                let position = terminal_panel.read(cx).position(window, cx);
                workspace.add_panel(terminal_panel, window, cx);
                let sessions = crate::sessions::global(cx)
                    .unwrap_or_else(|| panic!("sessions are not initialised"));
                let workspace_handle = cx.weak_entity();
                let panel = cx.new(|cx| AgentSessionsPanel::new(workspace_handle, sessions, cx));
                panel.update(cx, |panel, cx| panel.set_position(position, window, cx));
                workspace.add_panel(panel.clone(), window, cx);
                panel
            })
        })
        .unwrap_or_else(|error| panic!("window closed: {error}"));
    let terminal = display_terminal(cx);
    let view = add_tab(window, &workspace, &terminal, cx);
    window
        .update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.focus_panel::<AgentSessionsPanel>(window, cx);
            })
        })
        .unwrap_or_else(|error| panic!("window closed: {error}"));
    cx.run_until_parked();

    window
        .update(cx, |_, window, cx| {
            sessions_panel.update(cx, |panel, cx| panel.jump_to(&view, window, cx))
        })
        .unwrap_or_else(|error| panic!("window closed: {error}"));
    cx.run_until_parked();

    let active_is_terminal_panel = cx.read(|cx| {
        let workspace = workspace.read(cx);
        let terminal_panel = workspace
            .panel::<TerminalPanel>(cx)
            .unwrap_or_else(|| panic!("no terminal panel"));
        workspace.all_docks().iter().any(|dock| {
            dock.read(cx)
                .active_panel()
                .is_some_and(|panel| panel.panel_id() == terminal_panel.entity_id())
        })
    });
    assert!(
        active_is_terminal_panel,
        "the click revealed the terminal panel"
    );
}
