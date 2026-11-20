# Re-entrancy audit 1 (2026-11-20)

Follow-up to `crashes-2026-11-20.md`: the Fyx crates searched for the same class of bug, an entity
or window read or updated while gpui is already updating it. Base: `integration` 91f052daa2 plus
`fyx/profile-menu-crash` (1b599378fd was not in `integration` yet).

gpui behaviour this rests on (`crates/gpui/src/app.rs`): while a window is being updated it is out
of `App::windows`. `WindowHandle::read`/`read_with` then panic ("attempted to read a window that is
already on the stack"); `update` returns `Err` and does nothing. `Entity::read`/`update` of an
entity being updated panic. Subscriptions and observers run from `flush_effects`, after the
emitter's update; `subscribe_in`/`observe_in`/`cx.listener` callbacks run inside their own
window's (and entity's) update.

## Findings

| # | Where | Kind | Status |
| - | ----- | ---- | ------ |
| 1 | `terminal_host::sessions::Sessions::pop_up` | `main_window.update(request_attention)` from the view's `subscribe_in`: `main_window` is the window on the stack, so the update always failed and an inactive window never asked for attention. No crash. | Fixed: `status_maybe_changed` takes the `&mut Window` and calls `request_attention` on it. |
| 2 | `terminal_host::pseudo::target_workspace` | `read_with` on `cx.active_window()` and every window. Callers today run from the host's command loop and `cx.spawn`, outside window updates, so no crash yet; any future caller from a listener would panic. | Hardened: `update` instead of `read_with`, skipping a window on the stack. |
| 3 | `terminal_host::created::in_trusted_worktree` | Crash 1. | Fixed in 4468bdb9da/2fc3f624ee. |
| 4 | `terminal_host::sessions::status_maybe_changed` | Crash 2. | Fixed in 820524c9c4. |
| 5 | `terminal_host::sessions_panel::AgentSessionsPanel::jump_to` | Crash 3. | Fixed in 1b599378fd. |

## Checked, no change

- `terminal_host`: `run` (host commands: pseudo/created open and close, t3 read/write, repaint)
  runs from a `cx.spawn` loop. `notification.rs` "View" defers `reveal_in_window`.
  `profiles::launch` reads the window through the `Window` it was given and builds the tab in a
  spawned task. `accounts::menu_entries` and `t3::menu_entries` only read globals, the workspace
  and its project; their `on_select`s write settings or call the share bridge. `tab_status`
  focus handlers read only the `Terminal`. `usage_meters` notices go through
  `show_app_notification` (deferred) and cancel threads from plain subscriptions.
  `Sessions::list`/`find_terminal` read other views, called from render and the command loop.
- `ui_ext_host`: extension host calls reach the main thread through `MainThread` jobs (a
  `cx.spawn` loop), so `settings.open`, `workspace.open` and edits never run inside a surface's
  update. `RegistryEvent::RevealPanel`/`HidePanel` dock changes run in a subscription after the
  registry's update. Native buttons on the Fyx Extensions page are plain `on_click` closures (not
  `cx.listener`), and `window.dispatch_action` is deferred by gpui.
- `agent_review`: `notify_project_with_action` uses `update` (no panic) from plain subscriptions
  and spawned tasks; `WindowApprover` runs from a spawned task; toasts' "Review"/"Show Review
  Rules" update the workspace, not the toast; `open_turn_pane` reads `ReviewPane`s only from
  actions and toasts.
- `terminal_ext`: no entity or window access.
- `fyx:` hooks: `terminal_panel` "+" menu and the terminal context menu (above);
  `agent_ui::fyx::account_entries` updates the workspace and `AgentPanel` from a `ContextMenu`
  entry, like the upstream entries next to it; `project_panel::fyx_decorations` reads only its
  global.

## Rule

Added to `.claude/CLAUDE.md` (Fyx rules) rather than upstream's `.rules`.
