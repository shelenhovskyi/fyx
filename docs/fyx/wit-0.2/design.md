# `fyx:ui@0.2.0`: design

Status: **decided 2026-11-17**; see [Decisions](#decisions-2026-11-17). Ready for implementation,
not frozen: the world stays staged (Stable and Preview capped at 0.1) until the commit that
declares it frozen. The world is in
[`crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit`](../../../crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit),
next to the frozen 0.1 world, and its data model is in `ui_ext::v0_2` (platform plan 2.1; what
landed is in [`migration.md`](migration.md), "Status"). The host and the renderer are not changed
yet (plan 2.2, 2.3). The WIT parses with
`wasm-tools component wit` (1.263.0). A dummy component embedded from it validates with
`wasm-tools component embed --dummy` + `component new` + `validate`. How 0.1 and 0.2 components
coexist is in [`migration.md`](migration.md).

Sources: the backlog [`tasks/wit-0.2.md`](../../../tasks/wit-0.2.md) (rows #1–#10), the judge
reviews [`integration-1`](../../../tasks/reviews/integration-1.md) (findings 1–13, F1–F2),
[`integration-2`](../../../tasks/reviews/integration-2.md) (findings 1–8, W8-1…W8-10) and
[`w3-deep-1`](../../../tasks/reviews/w3-deep-1.md) (findings 1–8), and
[`docs/vscode-coverage.md`](../../vscode-coverage.md) ("Schema gaps"). Line references are to
`integration` at `2e91298`. Revised after the judge review
[`wit-0.2-design`](../../../tasks/reviews/wit-0.2-design.md) (on `claude/fyx-w7-judge-cd66a5`):
§L added (finding 1), one cap for `replace` and `mount` (finding 2), SDK world selection without
Cargo features (finding 3, migration.md §4), and the small points of finding 5.

## Summary

| # | Change | Source | Decision |
|---|---|---|---|
| A | Effects of host imports are ordered, checked at the call, and applied after it | integration-1 #3, integration-2 #7 | **accepted**: the base for B–F |
| B | `request-tick` / `cancel-tick` imports, `tick` export | backlog #2, integration-1 #9 | **accepted (priority)** |
| C | `surface-kind::modal`, `show-surface` / `hide-surface`, list `selected` + `row-selected` | backlog #5, W8 | **accepted (priority)**, keyboard selection included; a modal opens only after a click or Enter (decided 2026-11-17) |
| D | `ui-event::dismissed(dismiss-reason)` | backlog #7, W8 | **accepted**, for modals only |
| E | `add-surface` / `remove-surface` | backlog #6, W8 | **accepted** |
| F | `push-update` returns `result<_, host-error>` | backlog #9, W8-3 (integration-2), integration-1 #2 | **accepted** |
| G | `invalidate-rows` patch | backlog #1, integration-1 #5 | **accepted**, with an optional range |
| H | Row-local ids documented in the WIT | backlog #3, integration-1 #4 | **accepted** (docs only) |
| I | Frozen `icon` enum instead of upstream `IconName` strings | backlog #4, integration-1 #12 | **accepted**: 101 icons (decided 2026-11-17) |
| J | `list-row.indent`, `button-props.tooltip` | backlog #8, W8 | **accepted**: indent on the row, not on flex |
| K | `input-props.secret` | integration-2 W8-5 | **accepted** |
| L | Editor decorations: `fyx:ui/decorations` with `place-decoration` / `remove-decoration` | backlog #10, W6 | **accepted**, in 0.2 (decided 2026-11-17) |
| M | Notifications and progress: `fyx:ui/notifications` with `notify` / `dismiss-notification`, `progress-start` / `-update` / `-end`, three owner events | [platform spec](../platform/spec.md) decision 22 | **drafted 2026-11-18, for review**; WIT diff in M.6, not yet in the landed WIT, which still has its `TODO` |
| — | Panel visibility events, surface rename, commands, raw colours, in-row node clicks, `ignoreFocusOut` | various | **rejected or deferred**; see [Rejected and deferred](#rejected-and-deferred) |

Not changed: the flat tree, theme tokens only, the `mount` / `handle-event` / `list-rows`
exports, `log`, and the caps of 2048 patches per update, 16384 nodes per tree and 2048 nodes per
row. One cap changes: a `replace` and a `mount` may carry a whole tree, 16384 nodes, where the 0.1
host stopped at 2048 (integration-1 #11). Otherwise a push that passes the check in F could never
be applied, and its remount would fail the same way (judge wit-0.2-design #2).

## A. Effects of host imports: checked at the call, applied after it

**Motivation.** Integration-1 #3 fixed the order of pushes against the reply by returning them
with it (`Reply<T> { pushed, value }`, `fyx_ui.rs:233-241`). Integration-2 #7 then had to carry
pushes inside `ReplyError` too. 0.2 adds six more imports with side effects (`request-tick`,
`cancel-tick`, `add-surface`, `remove-surface`, `show-surface`, `hide-surface`), and L adds two
(`place-decoration`, `remove-decoration`). Each needs the same ordering guarantee, so they share
one rule. `log` has no effect on the UI and is not part of the effect list: it is written at
once, so a log line from a call that then traps is still kept.

**Semantics** (doc comment on `interface host`):

1. Every effectful import is checked **when it is called**. The guest gets the result
   synchronously (`result<_, host-error>`).
2. The effect is recorded in the call's **effect list**, in call order, and applied **after the
   export returns**: first every effect in order, then the export's own return value. This
   generalizes today's `Reply.pushed`.
3. If the export returns `Err` or its value fails validation, the effects still apply (as pushes do
   now via `ReplyError`). The guest already counts them as done.
4. A trap or an exceeded time budget drops the effect list and disables the extension (unchanged).
5. Effects made outside any export call happen only during instantiation and `surfaces()`. They go
   to the existing channel (`fyx_ui.rs:498-502`) and are applied after the static surfaces are
   registered.
6. The checks run against the extension's **live surface set as of that point in the call**. That
   set is kept in `UiState` and changed by `add-surface` / `remove-surface` as they are called. So
   `remove-surface("x")` followed by `push-update("x", …)` in the same call fails with
   `unknown-surface`. Only the extension adds or removes surfaces, so this set never races with
   the main thread.

`host-error` is a record of a closed `host-error-code` enum and a message, so the SDK can branch on
the code. The codes: `unknown-surface`, `limit-exceeded`, `invalid-argument`, `not-permitted`,
`unavailable`.

**Rejected:** a separate result type per import (five near-identical enums). Also rejected:
returning effects as values of the exports. That would mean `handle-event -> (update,
list<effect>)`, so every SDK call site would build effect lists by hand, and the ordering against
pushes would have to be spelled out again.

## B. Ticks: `request-tick`, `cancel-tick`, `tick`

**Motivation.** Backlog #2 and integration-1 #9. A component runs only inside a host call, so the
"timers" that the 0.1 WIT and the SDK promise for `push-update` (`fyx-ui.wit:166`,
`ui_ext_sdk.rs:109`) cannot exist. A clock, a progress status item, or "refresh every 30 s" needs
the host to call the extension back.

**Shape.** Ticks are **per surface**:
`request-tick(surface-id, delay-ms)`, `cancel-tick(surface-id)` and the export
`tick(surface-id) -> result<update, string>`. The update applies to that surface, like an
event's reply. Pushes to other surfaces go through `push-update` as usual.

Per surface rather than per extension, because the host knows visibility per surface and
throttles hidden ones (below). It also makes the reply a plain `update` with the same apply,
validate and remount path as `handle-event`.

**Semantics.**

- **One-shot, fixed delay.** One pending tick per surface. A new `request-tick` replaces the
  pending one, which lets the guest move a deadline earlier or later. A periodic timer re-requests
  inside `tick`. The delay counts from when the requesting call's effects are applied (see A), not
  from the import call. Ticks never catch up: if a tick runs late, the next one is due `delay`
  after the late one returns. Ticks are never queued behind each other.
- **Ordering.** A due tick enters the surface's event queue (`surface.rs:235-293`) behind
  already-queued user events, as a `QueuedItem::Tick`. It never overtakes an event. It does not
  flush pending input debounces, because it is not user input (w3-deep-1 #2 applies to user
  events only). A tick reply that applies does **not** reset `remounts_without_input`
  (`surface.rs:350-351`), so a tick loop cannot keep a broken surface remounting.
- **When ticks are not delivered.**
  - Never while the surface is `Mounting` or `Failed`. A pending tick survives a remount: the
    guest's timer state did not change, and the SDK's mirror was reset by the mount.
  - A `Failed` surface drops its tick.
  - For a modal, the tick is dropped when the modal closes. `request-tick` for a modal that is
    not shown returns `invalid-argument`.
- **Rate limits** (host constants in `fyx_ui.rs`; the WIT states only the 100 ms floor):

  | Rule | Value | Why |
  |---|---|---|
  | Minimum delay; smaller values are clamped up, not rejected | 100 ms | A tick is a full call with a 200 ms budget; 10 Hz is enough for progress text. Spinners need no ticks (the `spinner` icon is animated by the host; see I). |
  | Panel not visible in any window (dock closed or another panel active) | deferred until it is visible, then delivered at once if due | Nobody sees it. A status item is always visible. |
  | No Fyx window is active (app in the background) | minimum 1000 ms | Battery. |
  | Ticks per extension, across all its surfaces | 20 per second, token bucket | Bounds the queue load of many surfaces. Excess ticks are delayed, not dropped. |
  | Tick CPU share per extension | more than 10 % of wall time over 10 s clamps every tick of that extension to at least 1000 ms for the next 10 s, with one `warn` log | A slow ticker must not starve event handling on the shared queue (`fyx_ui.rs:509-516`). |
  | Consecutive `tick` errors on one surface | after 3, ticks for that surface stop until the next user event on it | A broken guest that re-requests inside a failing tick would log at 10 Hz forever. |

- `request-tick` on an unknown surface: `unknown-surface`. A delay above 24 h: `invalid-argument`.
  That keeps `Instant` arithmetic safe and catches unit mix-ups (seconds passed as ms are
  harmless; µs are not).
- A trap or budget overrun in `tick` disables the extension, as for every call.

**Changes.**

- Host, `crates/extension_host/src/wasm_host/fyx_ui.rs`:
  - `UiState` records `HostEffect::RequestTick { surface_id, delay }` and `CancelTick`.
  - New `UiExtension::tick(surface_id) -> Result<Reply<Update>>` beside `handle_event`.
  - The constants above.
- Renderer, `crates/ui_ext_host/src/surface.rs`:
  - `SurfaceState` gets `tick: Option<PendingTick { due: Instant, _timer: Task<()> }>`, using a
    GPUI executor timer, which also makes it testable with `run_until_parked`.
  - The queue item becomes `enum QueuedItem { Event(QueuedEvent), Tick }`.
  - A `visible` flag is fed by the containers.
  - The error counter.
- Renderer, `crates/ui_ext_host/src/panel.rs`: `ExtensionPanel` reports visibility, from
  `Panel::set_active` and the dock open state, to its `SurfaceState`.
- Renderer, `crates/ui_ext_host/src/registry.rs`: the per-extension token bucket and CPU share.
  Keep them in a small `TickBudget` per extension id; it is per extension, not per surface, and
  the registry is the only place that sees every surface of an extension.
- Renderer, `crates/ui_ext_host/src/backend.rs`: `UiBackend::tick`.
- SDK, `crates/ui_ext_sdk/src/ui_ext_sdk.rs`:
  - `Extension::tick(&mut self, surface_id) -> Result<Update, String>`, defaulting to
    `Ok(Update::Unchanged)`.
  - `request_tick(surface_id, Duration) -> Result<(), HostError>` and `cancel_tick`.
  - `register_extension!` exports `tick`.
  - Off wasm, both are recorded for `take_host_effects()` (see F).
  - Fix the `push_update` doc that promises timers.
- Sidecar, `sidecars/vscode-host/src/host.ts`:
  - New request `tick {surface-id}`.
  - Requests `request-tick` / `cancel-tick` from the sidecar to the host.
  - Use them for `withProgress` status text and `StatusBarItem` updates driven by `setInterval`.
    Today `setInterval` runs freely in Node and pushes, which works because the sidecar is a
    process, but it bypasses the host's rate limits.
- Demo: `extensions/fyx_demo_counter` gets a clock status item (one tick per second), which is also
  the manual test.

**Rejected:**
- An extension-wide `tick()` with no surface: no visibility throttling, and the reply has no
  surface to apply to.
- Fixed-rate periodic timers (`set-interval`): catch-up bursts after a stall, and cancellation
  becomes mandatory.
- Passing `now` to `tick`: the guest has `wasi:clocks` (linked at `fyx_ui.rs:340`).
- A host-driven animation API: out of scope, and the spinner icon covers the common case.

## C. Modal surfaces

**Motivation.** Backlog #5. W8 maps QuickPick and InputBox to a bottom-dock panel
(`sidecars/vscode-host/src/ui/quickpick.ts:1-8`). Native pickers (branch switcher, "select
model") are modals in Zed.

**Shape.** A new case `surface-kind::modal(modal-descriptor { width: modal-width })`.
`modal-width` has three classes (`small`, `medium`, `large`), mapped to the widths of Zed's own
pickers. The title is drawn as a header when it is not empty. The capability is `ui:modal` in
`[fyx_ui].capabilities`. Two new imports, `show-surface(surface-id)` and
`hide-surface(surface-id)`, work for modals and panels.

**Lifecycle.**

| Step | What happens |
|---|---|
| declared (static or `add-surface`) | Nothing is drawn or mounted. Phase `Closed` (new). |
| `show-surface` | The check needs a user gesture (below). After the call, the host opens the modal in the **active workspace window** and calls `mount`. Until the tree arrives it shows an empty modal frame of the declared width, so focus moves at once and no keystroke is lost. |
| shown | Same as a panel: events, `list-rows`, ticks, pushes. |
| user closes it (Escape, click outside, focus elsewhere, another modal opens, window closes) | The tree is dropped first. Then `handle-event(dismissed(reason))` is queued behind earlier events. Its reply must be `unchanged`; anything else is ignored with a debug log. Its effects for other surfaces still apply. Pending tick dropped; queued events for the modal after the dismissal dropped. |
| `hide-surface` | Closed, tree dropped, **no** `dismissed` event: the extension asked. |
| `show-surface` again | A fresh `mount`. Modals keep no tree between showings (VS Code's `hide()`/`show()` also re-renders). |

**Rules.**

- **User gesture** (decided 2026-11-17). A modal steals focus, so `show-surface` on a modal
  returns `not-permitted` unless the extension has a *transient activation*. Only a click or
  Enter grants one: `handle-event` for `click`, `row-click` (a mouse click on a row, or Enter on
  a focused list) or `input-submitted` (Enter in an input). `input-changed` does not, so typing in
  an extension's own input never opens a focus-stealing modal (judge wit-0.2-design #4). Neither do
  `row-selected` (arrow keys) and `dismissed`. The activation lasts until 5 s after that call
  returns, or until one successful `show-surface` consumes it. The 5 s window covers "click →
  fetch → show the picker from a tick". The rule follows browsers' popup blocking. Revealing a
  panel needs no gesture: it does not move focus.
- **One modal per window.** Zed's `ModalLayer` holds one. Opening any modal, an extension's or
  Zed's own, closes the active one, and the closed modal gets `dismissed(replaced)`. An extension
  modal can never veto its dismissal: `on_before_dismiss` always returns `Dismiss(true)`, because
  the host cannot ask the guest synchronously.
- **Same-type trap.** `ModalLayer::toggle_modal` closes the active modal **and returns without
  opening** when both have the same type (`modal_layer.rs:161-166`). Every extension modal shares
  one type, `ExtensionModal`. So the host calls `workspace.hide_modal` first, then
  `toggle_modal`. Showing the modal that is already shown in that window does nothing (checked
  with `workspace.active_modal::<ExtensionModal>()` and its key).
- **Dismiss reason.** `escape` comes from the modal's own `menu::Cancel` handler, which records
  the reason before emitting `DismissEvent`. Any other dismissal without a recorded reason is
  decided one frame later: `replaced` if `workspace.has_active_modal()`, otherwise `focus-lost`.
  `window-closed` comes from `cx.on_release` of the `ExtensionModal` while it is still marked as
  shown.
- **Focus.**
  - On open, focus goes to the first `input` node in pre-order. If there is none, it goes to the
    modal itself, so Escape still works.
  - Tab and Shift-Tab move through inputs and enabled buttons in tree order.
  - On close, `ModalLayer` restores the previous focus (`modal_layer.rs:233`).
- **Keyboard selection** (ships with 0.2, decided 2026-11-17). Without it, a picker cannot be
  driven by keyboard. This is the one addition beyond the backlog row:
  - `list-props.selected: option<u32>` is the highlighted row. It is drawn in every surface
    kind, and scrolled into view (`UniformListScrollHandle::scroll_to_item`) when the extension
    sets it. That also gives the sidecar's `TreeView.reveal` the scroll it lacks
    (`vscode-coverage.md`, "Partial APIs").
  - In a modal, Up, Down, PageUp and PageDown move the selection of the modal's **first list
    node in pre-order**, from anywhere in the modal. The host updates `selected` in its own tree
    first, as it does for typed text (integration-1 #6, `surface.rs:324-339`), then sends
    `row-selected(list, index)`.
  - Consecutive `row-selected` events for one list are coalesced in the queue, as
    `input-changed` is (`surface.rs:244-253`).
  - Enter in an input still sends `input-submitted`; the extension accepts its tracked
    selection, which is what VS Code's QuickPick does with `activeItems`. Enter while the list
    itself is focused sends `row-click(list, selected)`.
  - Changing only `selected` fetches no rows (see G).
- **Sizing.** The width comes from the class; the height follows the content up to 60 % of the
  window height. A `list` inside a modal grows to fill the remaining height.

**Changes.**

- `crates/ui_ext/src/types.rs`:
  - `SurfaceKind::Modal(ModalDescriptor)`, `ModalWidth`.
  - `ListProps::selected`.
  - `UiEvent::RowSelected` and `UiEvent::Dismissed(DismissReason)`.
  - `UiEvent::target()` becomes `Option<NodeId>` (`dismissed` has no node), and its callers in
    `surface.rs:239` and `:303` handle `None`.
- Host, `fyx_ui.rs`:
  - `CAPABILITY_MODAL = "ui:modal"`, added to `check_surfaces` (`fyx_ui.rs:188-203`).
  - `HostEffect::{Show, Hide}`.
  - The transient-activation timestamp in `UiState`, set by `handle_event` for `click`,
    `row-click` and `input-submitted` only.
- Renderer, new file `crates/ui_ext_host/src/modal.rs`: `ExtensionModal: ModalView` wraps a
  `SurfaceView`, handles `menu::Cancel`, the dismiss reason, focus and the selection keys. This is
  a new logical component, so a new file.
- Renderer, `surface.rs`:
  - `Phase::Closed`.
  - `SurfaceState::show` and `hide`, which mount on show and drop the tree on hide.
  - Dismissed queueing.
  - The local echo of `selected`.
- Renderer, `view.rs`:
  - Selected-row highlight and `scroll_to_item` in `virtualized` (`view.rs:625-680`).
  - The modal header.
- Renderer, `registry.rs`:
  - Applies `Show` / `Hide` in the active window. It tracks the last active workspace window,
    from the workspace observer it already has (`registry.rs:195-205`).
  - `show-surface` for a panel reuses `toggle_panel_focus`'s dock activation without the focus
    step (`registry.rs:281-317`).
  - No workspace window: the effect is dropped with a log. The check at import time cannot know
    about windows, so `unavailable` is returned only by the sidecar path, where the check is
    asynchronous anyway.
- SDK:
  - `Surface::modal(id, title, ModalWidth)`, `show_surface`, `hide_surface`.
  - `ListProps::selected(Option<u32>)`.
  - Doc example: an input-plus-list picker that tracks `row-selected`.
- Sidecar:
  - `ui/quickpick.ts`: `modal` kind instead of `panel` at the bottom; `show()` maps to
    `show-surface`, `hide()` to `hide-surface`; `activeItems` follows `row-selected`, and the
    active item is sent as `selected`.
  - `ui/treeview.ts`: `reveal` sets `selected`.
  - `ui/output.ts`: `OutputChannel.show()` maps to `show-surface` on the output panel.
  - `docs/vscode-coverage.md`: move QuickPick and InputBox out of "panel at the bottom".

**Rejected:**
- A modal that keeps its tree while hidden: it is cheaper to remount than to keep stale state
  consistent.
- `close-on-focus-loss: false` (VS Code `ignoreFocusOut`): `ModalLayer` hides on any click outside
  (`modal_layer.rs` render, background `on_mouse_down`). The only way to resist is returning
  `Dismiss(false)` from `on_before_dismiss`, which would also block every other modal from opening
  (`toggle_modal` returns when `!did_close`). It is recorded as `partial` in the coverage report.
- Showing a modal without a gesture: focus stealing from ticks or background work.
- A separate `open-modal` import: `show-surface` also covers revealing panels (VS Code
  `TreeView.reveal`, `OutputChannel.show`).

## D. `ui-event::dismissed`

**Motivation.** Backlog #7: the sidecar invented `fyx/surfaceClosed` (`host.ts:327`).

**Semantics.** Only for modals, with a `dismiss-reason` (see C). It is never sent for a close the
extension asked for (`hide-surface`, `remove-surface`). Every user-initiated close of a modal gets
exactly one `dismissed`. The host drops the tree before it sends the event, so the reply must be
`unchanged`.

Panels are not "dismissed": closing a dock only hides the panel, and its tree is kept
(w3-deep-1, "Panel lifecycle"). Status items cannot be closed by the user.

**Changes:** as in C. Sidecar: drop the `fyx/surfaceClosed` handler (`host.ts:327`) and map
`dismissed` in `SurfaceRegistry.handleEvent` to `onDidHide`. VS Code fires `onDidHide` for
`hide()` too, so the sidecar fires it itself on `hide-surface`.

**Rejected:**
- `visibility-changed` for panels (VS Code `onDidChangeVisibility`): no consumer in the
  coverage run, and ticks already get the host-side visibility throttling. Deferred.
- A reason `extension`: the extension knows when it closed a surface.

## E. Runtime surfaces: `add-surface`, `remove-surface`

**Motivation.** Backlog #6: VS Code creates QuickPicks, output channels and status bar items after
activation. The sidecar invented `fyx/surfacesChanged` (`host.ts:176`).

**Lifecycle and rules.**

- `add-surface(surface)` checks, in this order:
  - the id: 1–64 bytes of `[a-z0-9._-]`, otherwise `invalid-argument`. 0.1 does not restrict
    ids; 0.2 does, so they are safe in the `"<extension>/<surface>"` key and in action payloads.
  - not already live, otherwise `invalid-argument`.
  - the manifest has the kind's capability, otherwise `not-permitted`.
  - the extension has fewer than **32 live surfaces**, static ones included, otherwise
    `limit-exceeded`.
- After the call:
  - A panel is added to every open workspace and to new ones (`registry.rs:244-279`). It gets the
    next activation priority (`registry.rs:73-77`) and is mounted.
  - A status item is added to every status bar and mounted.
  - A modal is registered `Closed`.
  - Static surfaces from `surfaces()` follow the same path, so there is one code path.
- `remove-surface(id)` checks that the id is live. After the call:
  - Containers close in every window, with `Workspace::remove_panel` for panels, or by closing
    the modal.
  - The `SurfaceState` is dropped. With it go the tree, the row caches, the pending tick and the
    queued events. Replies still in flight for it are discarded when they arrive, because
    `this.update` fails on a released entity and the workers already stop on that
    (`surface.rs:286-291`).
  - No event is sent. The id becomes free, and adding it again creates a new surface with a new
    mount.
- Static surfaces can be removed too. They come back the next time the extension loads.
- Dock size and position are not persisted per runtime panel. All extension panels already share
  one `persistent_name` (`panel.rs:120-131`).

**Upstream hook.** `StatusBar` can only remove by position, and `position_of_item::<T>` finds the
first item of a type (`status_bar.rs:374-419`). Every extension status item has the same type. It
needs `StatusBar::remove_item_by_id(EntityId)`, about 8 lines marked `fyx:`, the same pattern as
`Dock::panel_index_for_id` (`dock.rs:550-555`). Decided 2026-11-17: the hook is accepted. The
alternative, keeping a removed item and rendering nothing, would leak one empty view per removal.
The hook goes into the upstream-diff table of the next integration review when it lands.

**Changes.**

- Host, `fyx_ui.rs`:
  - `UiState.live_surfaces: BTreeMap<String, SurfaceKind>` and `HostEffect::{AddSurface,
    RemoveSurface}`.
  - The manifest's capabilities move into `UiState`, so `add-surface` can check them;
    `load_from_dir` passes them to `load`.
  - `UiExtension::surfaces()` keeps returning the *static* set.
- Renderer, `registry.rs`:
  - `apply_effect` handles add and remove.
  - `register_backend` is split into `add_surface`.
  - The per-workspace `WorkspaceSurfaces` (`registry.rs:209-213`) learns to remove.
- Renderer, `surface.rs`: a removed state stops its workers, which they already do on release.
- SDK: `add_surface(&Surface)` and `remove_surface(id)`, both `-> Result<(), HostError>`.
- Sidecar:
  - `ui/surfaces.ts`: transient surfaces call `add-surface` / `remove-surface`, now requests to
    the host, instead of notifying `fyx/surfacesChanged`.
  - The status items and output channels that are folded into static surfaces today
    (`vscode-coverage.md:100`) can become one surface each.
  - `mock-host.ts:261`: handle the two requests and enforce the id rule, the cap and the
    capability check.

**Rejected:**
- `update-surface` to rename a surface or move its dock (TreeView `title`): remove plus add does
  it, at the cost of a remount. The sidecar draws a header row today. Deferred until someone needs
  it in place.
- Unlimited surfaces: each panel is a dock entry in every window.

## F. `push-update` returns a result

**Motivation.**
- W8-3 (integration-2): the sidecar's mirror drifts after a rejected update, so the sidecar
  already made `push-update` a request and resends a `replace` when it fails
  (`host.ts:171-174`, `ui/surfaces.ts:5`).
- Integration-1 #2: a WASM guest has no way to learn that a push failed.
- Integration-1 #10: pushes are unbounded. Today the 65th push of a call traps
  (`fyx_ui.rs:279-281`).

**Semantics.**
- `ok` means the update passed every check the host can make **at once**:
  - the surface is live (as of this point in the call, see A);
  - a `patches` update has at most 2048 patches;
  - a `replace` has at most 16384 nodes, the same cap as the tree and as `mount` (see Summary),
    so "resend whole" after an `err` always fits;
  - this is at most the 64th push of the call.
- It does **not** mean the update fits the tree. That is only known when it is applied on the main
  thread. A misfit still remounts the surface (`surface.rs:438-462`), and the remount resets the
  SDK mirror.
- On `err` nothing was queued. The 65th and later pushes return `limit-exceeded` instead of
  trapping, and a guest that loops anyway is stopped by the time budget.
- A push to a modal that is not shown passes the check and is dropped when applied. The user may
  have closed it a moment ago; the `dismissed` event follows.

**Changes.**
- Host, `fyx_ui.rs:278-287`: return the error instead of trapping, and add the size checks (they
  are `TreeLimits` from `ui_ext`).
- SDK:
  - `push_update(...) -> Result<(), HostError>`.
  - New `SurfaceTree::push(&mut self, surface_id) -> Result<(), HostError>`. It takes the pending
    update and pushes it. On `err` it marks the tree *unsynced*, so the next `take_update`
    returns a full `Replace` of the mirror. This is the WASM form of the sidecar's fallback.
  - `take_pushed_updates()` becomes `take_host_effects() -> Vec<HostEffect>` off wasm, so
    extension unit tests see ticks and surface changes too.
- Sidecar: no protocol change, since it is already a request. `mock-host.ts` returns the same
  error codes as the WASM host (JSON-RPC error `data.code` = the `host-error-code` name).

**Rejected:**
- Validating the tree fit synchronously. It would need a second copy of every surface tree inside
  `extension_host`, next to the renderer's: a fifth copy of the patch rules (integration-2 #8
  counts four).
- An async result (`future<result>`): WASI p3 is not available to this host.

## G. `invalidate-rows` patch

**Motivation.**
- Backlog #1 and integration-1 #5: rows change while the list's props do not.
- Today the extension re-keys the list (remove plus insert). That loses the scroll position and
  burns ids. The todo demo now rebuilds only when its visible set changes
  (`fyx_demo_todo.rs:311-342`), but still re-keys then.
- The sidecar re-keys too (`vscode-coverage.md:101`).

**Semantics.**
- `invalidate-rows({target, rows: option<row-range>})`. The target must be a `list` or `table`
  node of the surface tree, otherwise the patch fails with `TreeError::NotAList`, a new variant,
  and the whole update is rejected.
- `rows: none` means all rows. Ranges are clamped to `row-count`.
- The host bumps the generation of the affected rows only. Old rows stay drawn until their
  replacements arrive, as today (`surface.rs:39-50`). Only the visible part is fetched: one
  `list-rows` call per list per update, however many invalidations it carries. Several ranges
  merge into their hull.
- It counts as touching the list in `apply_update`'s result (`tree.rs:152-160`).

**Refetch rule in 0.2.** Rows are fetched again when:
- `invalidate-rows` names them;
- `row-count`, `row-height` or `columns` changes in a `set-props`;
- the list is inserted or replaced.

A `set-props` that changes only `selected` fetches nothing. **0.1 components keep today's rule**:
any touched list refetches everything (`surface.rs:418-425`). See `migration.md`, "Behaviour by
version".

**Changes.**
- `crates/ui_ext/src/tree.rs`:
  - `Patch::InvalidateRows`.
  - `apply_update` returns `Touched { nodes: BTreeSet<NodeId>, rows: BTreeMap<NodeId,
    RowInvalidation> }` instead of a bare set, so the renderer can tell a props change from a row
    invalidation.
  - A `set-props` on a list records `RowInvalidation::All` only if the refetch fields changed.
- Renderer, `surface.rs`: `RowCache` gets range-aware invalidation. Index entries already carry a
  generation (`surface.rs:52-58`), so invalidation bumps the generation of the indexes in range
  only.
- SDK:
  - `SurfaceTree::invalidate_rows(list, Option<Range<u32>>)` and `PatchSet::invalidate_rows`.
  - The todo demo stops rebuilding its list on filter changes.
- Sidecar:
  - `schema.ts`: the `Patch` union gets `{ "invalidate-rows": { target, rows } }`.
  - `ui/treeview.ts` and `ui/quickpick.ts` stop re-keying the list.

**Rejected:**
- A list of indexes: a range covers "these rows changed" and "everything from here shifted".
- An implicit refetch on every `set-props` of the list (0.1's rule): it costs a fetch for every
  selection move.

## H. Row-local ids, documented

**Motivation.**
- Backlog #3 and integration-1 #4: the 0.1 WIT says ids are "unique within one surface"
  (`fyx-ui.wit:5-6`).
- The SDK numbers every row from `ROW_NODE_ID_BASE` (`ui_ext_sdk.rs:98`).
- The host never routes row ids: a click in a row is `row-click`, and buttons in rows get no
  handler (`view.rs:532-540`). This is pinned by a test
  (`extension_host/src/wasm_host/fyx_ui/tests.rs:23-24`).

**Change.** The doc comments on `node-id`, `list-row.nodes` and `row-click` say so. Ids inside a
row are row-local, and every row may reuse them. There is no ABI change.

The sidecar uses its own base, 1,000,000 (W8-10). Keep that or align it with `0x8000_0000`; either
is legal under the documented rule.

**Rejected (deferred):** `row-node-click(list, index, node-id)`, so that a button inside a row
could be told apart (VS Code inline tree-item actions). It needs hit-testing of nested handlers
inside `uniform_list` rows. The coverage run has no consumer of inline row buttons: QuickPick item
buttons and TreeView inline menus are both reported as partial. Revisit in 0.3.

## I. Frozen icon list

**Motivation.**
- Backlog #4 and integration-1 #12: 0.1 passes upstream `IconName` snake_case strings, parsed with
  `IconName::from_str` and falling back to `Blocks` (`view.rs:849-851`).
- Any upstream rename silently changes extension icons.
- The sidecar already sends names that are not `IconName`s: `output`, `search`, `blank`
  (`ui/output.ts:96`, `ui/quickpick.ts:66`, `:279`).

**Semantics.** `enum icon` has 101 generic cases. There are no brand, AI-provider or Zed-specific
icons, so extensions cannot impersonate host UI. It is used by `button-props.icon`,
`icon-props.name` and `panel-descriptor.icon`. Two cases have host behaviour:
- `spinner` is drawn rotating (`with_rotate_animation`, the pattern in
  `ui/src/components/collab/update_button.rs:182`);
- `blank` draws nothing at icon size.

Adding an icon needs a new world version. Each per-version bindings module maps its own enum, so
old components keep working.

**Mapping.** It lives in `view.rs` as an exhaustive `match` with no fallback arm. If upstream
removes an `IconName`, the host stops compiling, rather than an icon changing silently.

| `icon` | `IconName` | `icon` | `IconName` | `icon` | `IconName` |
|---|---|---|---|---|---|
| arrow-up/-down/-left/-right | `Arrow*` | chevron-up/-down/-left/-right | `Chevron*` | expand | `ExpandVertical` |
| collapse | `ListCollapse` | menu | `Menu` | ellipsis | `Ellipsis` |
| list-tree | `ListTree` | list-todo | `ListTodo` | table | `Table` |
| file-tree | `FileTree` | plus | `Plus` | dash | `Dash` |
| check | `Check` | check-double | `CheckDouble` | close | `Close` |
| search | `MagnifyingGlass` | filter | `Filter` | refresh | `RotateCw` |
| undo | `Undo` | copy | `Copy` | pencil | `Pencil` |
| trash | `Trash` | replace | `Replace` | download | `Download` |
| send | `Send` | share | `Share` | link | `Link` |
| pin | `Pin` | unpin | `Unpin` | play | `PlayFilled` |
| pause | `DebugPause` | stop | `Stop` | maximize | `Maximize` |
| minimize | `Minimize` | split | `Split` | file | `File` |
| file-code | `FileCode` | file-diff | `FileDiff` | file-text | `FileTextOutlined` |
| folder | `Folder` | folder-open | `FolderOpen` | code | `Code` |
| terminal | `Terminal` | diff | `Diff` | regex | `Regex` |
| case-sensitive | `CaseSensitive` | whole-word | `WholeWord` | hash | `Hash` |
| at-sign | `AtSign` | git-branch | `GitBranch` | git-commit | `GitCommit` |
| pull-request | `PullRequest` | info | `Info` | warning | `Warning` |
| error | `XCircle` | help | `CircleHelp` | bell | `Bell` |
| bell-dot | `BellDot` | circle | `Circle` | dot | `Indicator` |
| star | `Star` | star-filled | `StarFilled` | bookmark | `Bookmark` |
| thumbs-up | `ThumbsUp` | thumbs-down | `ThumbsDown` | todo-pending | `TodoPending` |
| todo-progress | `TodoProgress` | todo-complete | `TodoComplete` | spinner | `LoadCircle`, rotating |
| blank | (empty box) | settings | `Settings` | clock | `Clock` |
| debug | `Debug` | eye | `Eye` | eye-off | `EyeOff` |
| lock | `Lock` | unlock | `LockOff` | person | `Person` |
| people | `UserGroup` | chat | `Chat` | envelope | `Envelope` |
| book | `Book` | notepad | `Notepad` | image | `Image` |
| globe | `Public` | server | `Server` | database | `DatabaseZap` |
| keyboard | `Keyboard` | box | `Box` | archive | `Archive` |
| sparkle | `Sparkle` | bolt | `BoltFilled` | flame | `Flame` |
| power | `Power` | blocks | `Blocks` | | |

Every name on the right was checked against `crates/icons/src/icons.rs` at `2e91298`. The four
demo icons outside this list map as follows: `rotate_cw` becomes `refresh`, `sparkle`, `hash` and
`dash` keep their names, and `list_todo` becomes `list-todo`.

**Changes.**
- `crates/ui_ext/src/types.rs`:
  - `enum Icon`.
  - `IconProps.name`, `ButtonProps.icon` and `PanelDescriptor.icon` become `IconRef`, an
    `enum IconRef { Frozen(Icon), Upstream(String) }`. The 0.1 conversion produces `Upstream`,
    which keeps 0.1's `from_str` behaviour; 0.2 produces `Frozen`.
- Renderer: `view.rs` `icon_name` takes `&IconRef`; `panel.rs:41` follows.
- SDK: the generated `Icon` replaces `&str` in the `ButtonProps::icon` and `IconProps` builders,
  and in `Surface::panel`.
- Sidecar:
  - `schema.ts`: an `Icon` string-literal union.
  - `ui/surfaces.ts` `splitCodicons` maps codicon names to `Icon`, falling back to `blocks` and
    reporting the fallback in the coverage `partial` notes.

**Rejected:**
- Keeping strings with a documented list: typos fail silently, and the ABI still depends on
  upstream names.
- Extension-supplied SVG icons: they need an asset pipeline and sandboxing of the images.
  Deferred.

## J. `indent` on rows, `tooltip` on buttons

**Motivation.**
- Backlog #8: the sidecar shows tree depth as flex padding, which also pads vertically
  (`ui/treeview.ts:5`, `:217`).
- W8-8: a deep enough tree overflows the `u16` padding and fails the whole reply.
- Tooltips are dropped (`vscode-coverage.md:104`).

**Semantics.**
- `list-row.indent: u8` is a depth, in levels. The host multiplies it by its own indent step,
  matching the project panel's indent setting. Levels above 32 are drawn at 32. A `u8` cannot
  overflow the way pixel padding did.
- The indent is on the row, not on `flex-props`, because only tree rows need it, and a row has
  exactly one root.
- `button-props.tooltip: option<string>` holds at most 1024 bytes and is truncated beyond that.
  It is shown with Zed's `Tooltip::text`.
- For a status item, its root button's tooltip is the item tooltip.

**Changes.**
- `types.rs`: the fields.
- Renderer: `view.rs` `row` (`view.rs:682-726`) indents with `pl(indent × step)`; `button`
  (`view.rs:526-556`) adds the tooltip.
- SDK: `list_row(index, indent, build)`, a new argument; `ButtonProps::tooltip`.
- Sidecar: `ui/treeview.ts` sends `indent: depth` instead of padding; `ui/statusbar.ts` and
  QuickPick buttons send `tooltip`.

**Rejected:** tooltips on every node kind: only buttons are interactive enough to need one. Text
can be truncated, but that is a renderer concern.

## K. `input-props.secret`

**Motivation.** Integration-2 W8-5: `InputBox.password` would show secrets in plain text, so the
sidecar throws `NotImplementedInFyx` (`vscode-coverage.md:173`).

**Semantics.**
- The host masks the text with `Editor::set_masked(true)` (`editor.rs:9074`).
- It never writes the value of a secret input to logs, including the debug logs of dropped events
  (`surface.rs:240`, `:313`).
- It does not keep the value in undo history beyond the editor's own.
- The value still reaches the extension.

**Changes.**
- `types.rs`: the field.
- Renderer: `view.rs` `input` / `sync_input` (`view.rs:173-241`, `:558-589`) sets masking;
  `surface.rs` redacts in logs.
- SDK: `InputProps::secret()`.
- Sidecar: `ui/quickpick.ts:355-362` maps `password` to `secret`, and the `NotImplementedInFyx` throw
  is removed.

## L. Editor decorations (`fyx:ui/decorations`)

**Status: accepted, in 0.2** (decided 2026-11-17; judge wit-0.2-design, decision 1). W6's dev
data source (`FYX_UI_DEV_DECORATIONS`) stays as a host-side test aid.

**Motivation.** Backlog #10, W6. Layer 4 of `ARCHITECTURE.md`: blocks above or below buffer
lines that survive edits, such as test results, blame or diagnostics. W6 built the host side.
`ui_ext_host::decorations::Decorations` draws a retained `fyx:ui` tree as an editor block,
anchored by a CRDT anchor per buffer and shared by every editor on that buffer
(`tasks/progress/W6.md`). No extension can place one yet.

**Shape.** W6's two functions, adapted to A, in an interface of their own that the world imports:

- `place-decoration(id, location, nodes) -> result<_, host-error>`;
- `remove-decoration(id) -> result<_, host-error>`.

An interface rather than more `host` functions, so the capability boundary shows in the WIT, and
a 0.3 change to decorations (inlays, buffer handles) leaves `host` alone. Imports rather than a
`surface-kind`, for W6's reason: surfaces are few and long-lived, while decorations come and go
with analysis results.

The host links `decorations` for every 0.2 component and checks the capability per call. The
judge suggested linking it only for `ui:editor-decorations`. But then a component that imports it
would fail to instantiate wherever the capability is not granted, with a link error instead of an
error it can handle. Checked per call, the same component works either way.

**Semantics.**

- **Checks at the call** (A), in this order:
  - the manifest has `ui:editor-decorations`, otherwise `not-permitted`;
  - the id follows E's rule (1–64 bytes of `[a-z0-9._-]`), otherwise `invalid-argument`;
  - the id is not a live surface id, otherwise `invalid-argument`. Ids share one namespace, so
    `push-update`, `handle-event`, `list-rows` and `mount` need no second id parameter;
  - the path is relative, `/`-separated, at most 1024 bytes, with no empty, `.` or `..`
    component, otherwise `invalid-argument`;
  - `nodes` has at most 16384 nodes, otherwise `limit-exceeded`;
  - a new id while 512 decorations are live, otherwise `limit-exceeded`. The cap is separate
    from E's 32 surfaces, because a linter places one decoration per finding;
  - at most 512 decoration effects per export call, otherwise `limit-exceeded`.
- Whether the tree fits (parents first, children only under `flex`) is checked when it is
  applied, as for pushes (F). On a misfit the host calls `mount(id)`; when that fails too, the
  decoration is removed with one `warn` log. It uses the same remount counter as surfaces.
- **After the call** the registry hands the tree to `Decorations` (W6), keyed
  `"<extension>/<id>"`, with a backend that routes the block's events to the extension as
  `handle-event(id, …)` and its rows as `list-rows(id, …)`. Rendering is retained and uses theme
  tokens only, so there are no WASM calls in `render()`.
- **Location.** The path is matched against every worktree of every open workspace. Buffers
  opened later get the anchor when they open, as in W6. A path outside every worktree never
  matches. The host reads no file for the extension: `line` is anchored in buffers that are
  already open or opened later, and rows past the end clamp to the last row. If the anchored
  line is deleted, the block stays where the anchor collapses to. W6 also accepts absolute paths
  and component suffixes; 0.2 narrows that to worktree-relative paths, so extensions cannot probe
  for files outside the project.
- **Placing a live id again** moves the decoration (a new anchor from `line`) and replaces its
  tree, like a fresh mount. Events still queued for the old tree are dropped.
- **Removal.** `remove-decoration`, or the extension being disabled or unloaded. No event is
  sent, as for `remove-surface`.
- **Not for decorations:** `request-tick`, `show-surface`, `hide-surface` and `remove-surface`
  return `invalid-argument`. An extension that refreshes decorations periodically ticks one of
  its surfaces and pushes to the decorations from there. That keeps the tick budget (B) per
  surface and bounded.
- **Deferred to 0.3:** a `decoration-orphaned` event, anchoring to a buffer handle (layer 3)
  instead of a path, and inline (inlay) decorations.
- 0.1 components have no decorations.

**Changes.**

- WIT (W0): `decoration-placement`, `decoration-location`, `interface decorations`,
  `import decorations`.
- Model, `types.rs`: `DecorationPlacement` and `DecorationLocation`.
- Host, `fyx_ui.rs`:
  - link `decorations` for 0.2;
  - `UiState.live_decorations: BTreeSet<String>`, checked together with `live_surfaces`;
  - `HostEffect::{PlaceDecoration { id, location, nodes }, RemoveDecoration(id)}`;
  - the capability, the two caps and the path rule.
- Renderer:
  - `registry.rs`: `apply_effect` for both effects, a per-decoration backend for events and
    rows, the remount counter, removal on unload;
  - `decorations.rs`: worktree-relative path matching for extension decorations.
- SDK:
  - `place_decoration(id, &DecorationLocation, nodes)` and `remove_decoration(id)`, both
    `-> Result<(), HostError>`;
  - `SurfaceTree::place(id, location, build)`, which mounts the mirror and places it;
  - `take_host_effects` records both.
- Sidecar: no VS Code mapping in 0.2, since `TextEditorDecorationType` styles text rather than
  drawing trees. `mock-host.ts` returns `not-permitted`.
- Tests:
  - host: `not-permitted` without the capability, a clash with a surface id, the 513th
    decoration, a `..` path, `push-update(id)` after placing, the `mount(id)` resync on a misfit,
    and that removal drops queued events;
  - renderer: W6's decoration tests with a WASM backend, where a click in the block reaches
    `handle-event(id)`;
  - SDK: both effects in `take_host_effects`.

## M. Notifications and progress (`fyx:ui/notifications`)

**Status: drafted 2026-11-18, for review.** 0.2 is staged, not frozen, so M can join it. The WIT
change below is for W2c (host) and W3 (renderer) to land. The landed WIT is not edited by this
draft: its `TODO` marks the place.

**Motivation.** Spec decision 22, `docs/vscode-coverage.md` (`showInformationMessage`,
`withProgress`). Extensions need these cases, and today each one needs a panel:
- "Build finished" or "3 problems found" after work an extension ran on a tick;
- a short choice such as "Reload now?" with two buttons;
- progress for long work.

**Shape.** One interface, `notifications`, imported by the world and checked per call, for L's
reason: a component that imports it still instantiates without the capability, and gets
`not-permitted`.

- `notify(notification) -> result<u64, host-error>` and `dismiss-notification(id)`.
- `progress-start(options) -> result<u64, host-error>`,
  `progress-update(id, value, message) -> result<_, host-error>` and `progress-end(id)`.
- Three new `ui-event` cases, delivered to the **owner**: `notification-action`,
  `notification-closed` and `progress-cancel-requested`.

**Owner.** Every notification and progress names an owner: a live surface or decoration id of the
extension.
- **Why:** user actions must come back as a `ui-event` (`handle-event(surface-id, event)`), and
  the reply needs a tree to apply to. The reply is an `update` for the owner, like any event's.
- **Lifetime.** Removing the owner (`remove-surface`, `remove-decoration`, a reload, a disable)
  closes its notifications and ends its progresses, without events. Nothing outlives the
  surface that can react to it.
- **Rejected:** a separate export `notification-event(id, event)`. It would return no update, so
  every action would need a `push-update`, and it would add an export to a world whose exports
  are otherwise all per surface.

### M.1 Notifications

| Field | Rule (checked at the call, A) |
|---|---|
| `owner` | a live surface or decoration id, otherwise `unknown-surface` |
| `level` | `info`, `success`, `warning`, `error`; the host picks the icon and colour |
| `title` | 1–80 bytes, one line |
| `body` | at most 500 bytes, at most 4 lines |
| `actions` | at most 2; ids `[a-z0-9._-]{1,64}`, unique; labels 1–24 bytes, one line |
| `auto-dismiss` | `true`: host-chosen time (6 s for info and success, 10 s for warning and error); `false`: stays until closed |

- **Text rules.** All text is plain: no markdown, links or icons. It may not contain control
  characters (Cc), format characters (Cf: bidi overrides, zero-width) or U+2028/U+2029, the
  registry's rules for `description` (registry-format.md). A violation is `invalid-argument`.
- **Where.** The notification appears in the active workspace window when the effect is applied,
  in Zed's notification stack. It never takes focus and never opens a modal. With no workspace
  window it is dropped, with one log line and `notification-closed(id, window-closed)` to the
  owner.
- **Time.**
  - The extension cannot choose a duration, so it cannot flash a notification for 50 ms or make
    one look like host UI by its timing.
  - The timer pauses while the pointer is over the notification and while the window is in the
    background, so an expired notification was on screen for its full time.
  - `auto-dismiss: false` is meant for questions, so it is limited by the open cap below.
- **Events** (to `handle-event(owner, …)`):
  - **`notification-action({notification, action})`** when a button is clicked. The notification
    closes, and no `notification-closed` follows.
  - **`notification-closed(id, reason)`**, with reason `user`, `expired` or `window-closed`.
  - `dismiss-notification` sends no event (D's rule for `hide-surface`). Removing the owner sends
    none either.
- **Caps** (spec decision 22, made exact):
  - **Rate:** at most **5 accepted `notify` calls per rolling 60 s** per extension. The 6th returns
    `limit-exceeded` and writes the title to the log at `info`, under `fyx_ui`, with the
    extension id. That is the spec's "extra ones go to the log".
  - **Open:** at most **3 open notifications** per extension. A 4th returns `limit-exceeded`. The
    host does not close older ones to make room, so an extension cannot rotate its notifications
    to stay on screen.
  - **Per call:** at most 4 notification effects per export call.
- **Click guard.** For **750 ms** after a notification appears, its buttons ignore clicks: it may
  have popped up under the pointer just as the user clicked something else (classic
  click-jacking). The close button works at once.

### M.2 Progress

| Field | Rule |
|---|---|
| `owner` | a live surface or decoration id; a **panel** for `location: panel` (otherwise `invalid-argument`) |
| `title` | 1–80 bytes, one line, same text rules |
| `location` | `status-bar` or `panel` |
| `cancellable` | draws a Cancel button that sends `progress-cancel-requested(id)` to the owner |
| `progress-update.value` | `indeterminate` or `percent(0..=100)`; more than 100 is `invalid-argument` |
| `progress-update.message` | at most 120 bytes, one line, same text rules; `none` keeps the previous message |

- **`status-bar`.** One host status-bar item per extension that has a running status-bar progress.
  It is not an extension surface and does not count against the 32 surfaces. It shows a spinner
  or the percentage, the extension's name and the newest progress's title. Its tooltip lists
  every running progress of the extension with its message. Clicking it reveals the owner if the
  owner is a panel.
- **`panel`.** A thin bar under the owner panel's header, with the title and message on hover.
  Several progresses on one panel stack, at most 4.
- **Drawing.** The host redraws a progress at most 10 times a second and keeps only the last
  value. A guest can send updates on every tick (B), bounded by B's 100 ms floor, and on every
  event. Updates for an ended or unknown id return `invalid-argument`, so a stale handle shows up
  in the SDK.
- **End.** `progress-end`, removing the owner, or the extension stopping (a trap, the time
  budget, a disable, a reload or an uninstall) ends the progress, without an event. There is no
  timeout: progress for long work is legitimate, and the user sees which extension it is.
- **Cancel.**
  - `progress-cancel-requested` is a request. The progress stays, with its Cancel button disabled,
    until the extension calls `progress-end`. The extension owns the work, so only it knows when
    the work has stopped.
  - The event does not grant an activation, so Cancel can never open a modal.
- **Caps:** at most **4 running progresses** per extension and 30 `progress-start` calls per
  rolling 60 s (`limit-exceeded`), and at most 64 `progress-update` calls per export call (as for
  pushes, F).

### M.3 Interaction with the modal gesture rule (C)

| Source | Grants the click-or-Enter activation? |
|---|---|
| `notification-action` | **Yes**: a real click on a button of this extension, after the 750 ms guard. "Pick a branch" can open the extension's picker modal. |
| `notification-closed` (any reason) | No. Closing a notification is not a request for more UI. |
| `progress-cancel-requested` | No (M.2). |
| `notify`, `progress-*` themselves | Need no gesture: they do not take focus. |

`notify` from inside a `handle-event` for a click does not pass the activation on to the
notification. The notification's own button click is a new gesture.

### M.4 Anti-spoofing

The risk: a notification that looks like Fyx's own, such as "Fyx: review rules changed, Apply"
or "Enable this extension", which users have learned to click.

1. **Provenance line.** The host draws a first line on every extension notification and on the
   status-bar progress item: a puzzle-piece icon, then the extension's name, then its id in
   muted text, for example `🧩 Todo · todo`.
   - The name comes from the installed manifest, the id from `extension.toml`. The host applies
     the registry's `name` rule to every name, including dev and from-file installs: letters,
     digits and plain punctuation, no bidi controls, at most 64 characters. A name that fails it
     is replaced by the id.
   - The extension has no field that changes this line, its icon, its colour or its position.
   - Registry ids such as `fyx` or `fyx-*` are reserved (registry-repo.md), so the id cannot say
     "Fyx".
2. **No host styling.** The extension picks only the `level`. Fyx's own notifications (review
   rules, consent, security) never carry the provenance line, and consent and security prompts
   are modals or notifications that only the host creates.
3. **Plain text** (M.1). No links to click, no icons to imitate host buttons, and no bidi tricks
   that reorder the name.
4. **Click guard** (M.1), and at most 2 buttons, labelled with plain text.
5. **Owner panels only for `panel` progress**, so a progress bar cannot appear on another
   extension's or a host panel.

### M.5 Capability

`ui:notifications`, tier **T0**, already named in spec decision 21 and in
`ui_ext_wasm::permissions` and `fyx-ext check`'s known capabilities. It covers notifications and
progress. It is T0 because it reads nothing, and the rate and open caps bound the annoyance. The
extensions page lists it as "shows notifications and progress". Users who still find an
extension too noisy can disable it. A per-extension mute is a later, additive host setting.

### M.6 WIT change (for W2c and W3)

It applies to the landed `crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit` and replaces its `TODO`. The
result parses with `wasm-tools component wit` (1.263.0), and a dummy component embedded from it
validates (`component embed --dummy` + `component new` + `validate`).

```diff
@@ -305,6 +305,32 @@
         row-selected(tuple<node-id, u32>),
         /// The host closed a modal surface; its tree is gone. The reply must be `unchanged`.
         dismissed(dismiss-reason),
+        /// §M. The user clicked an action button of a notification this surface owns. Grants the
+        /// click-or-Enter activation, like `click`.
+        notification-action(notification-action-event),
+        /// §M. A notification this surface owns closed without the extension asking (an
+        /// extension's own `dismiss-notification` sends nothing).
+        notification-closed(tuple<u64, notification-close-reason>),
+        /// §M. The user pressed Cancel on a cancellable progress this surface owns. The progress
+        /// stays until the extension calls `progress-end`.
+        progress-cancel-requested(u64),
+    }
+
+    /// §M. Which action of which notification was clicked.
+    record notification-action-event {
+        notification: u64,
+        /// The `id` the extension gave the action.
+        action: string,
+    }
+
+    /// §M. Why a notification closed when the extension did not close it.
+    enum notification-close-reason {
+        /// The user closed it (its close button, or Escape while it had focus).
+        user,
+        /// It was `auto-dismiss` and its time ran out.
+        expired,
+        /// The window that showed it closed.
+        window-closed,
     }
 
     enum dock-position { left, right, bottom }
@@ -459,18 +485,97 @@
     remove-decoration: func(id: string) -> result<_, host-error>;
 }
 
-// TODO(design.md section M, platform plan 2.1): notifications and progress (platform spec
-// decision 22: `notify(level, text, actions)`, `progress-start` / `progress-update` /
-// `progress-end`, at most 5 notifications per minute per extension). design.md has only a
-// placeholder row for M: the action and progress types, how an action reaches the extension,
-// the ids, the capability and the error codes are not specified yet. Add the interface here,
-// before 0.2 is frozen, once M is drafted.
+/// Notifications and progress (design.md, "M"). Needs the `ui:notifications` capability (tier
+/// T0); without it every call returns `not-permitted`. Every function follows the effect rule of
+/// `host`: checked at the call, applied after the export returns, dropped by a trap.
+///
+/// Every notification and progress has an **owner**: a live surface or decoration id of this
+/// extension. Its events arrive as `handle-event(owner, …)`, and removing the owner closes them
+/// without an event. The host always draws the extension's name and id above a notification
+/// and next to a progress; the extension cannot style, hide or replace that line.
+interface notifications {
+    use host.{host-error};
+
+    enum notification-level { info, success, warning, error }
+
+    record notification-action {
+        /// 1 to 64 bytes of `[a-z0-9._-]`, unique within the notification. Sent back in
+        /// `notification-action-event`.
+        id: string,
+        /// The button text: 1 to 24 bytes of plain text on one line.
+        label: string,
+    }
+
+    record notification {
+        /// The surface or decoration whose `handle-event` gets this notification's events.
+        owner: string,
+        level: notification-level,
+        /// 1 to 80 bytes of plain text on one line.
+        title: string,
+        /// At most 500 bytes of plain text, at most 4 lines. No markdown, links or icons.
+        body: option<string>,
+        /// At most 2. Drawn as buttons in list order; the first is the primary one.
+        actions: list<notification-action>,
+        /// `true`: the host closes it after a time it chooses by level (6 s for `info` and
+        /// `success`, 10 s for `warning` and `error`), paused while the pointer is over it or the
+        /// window is in the background. `false`: it stays until the user or the extension closes
+        /// it.
+        auto-dismiss: bool,
+    }
+
+    /// Shows a notification in the active workspace window, without moving focus, and returns its
+    /// id. Caps: 5 accepted calls per rolling 60 s and 3 open notifications per extension;
+    /// past either, `limit-exceeded`, and the title is written to the log instead.
+    notify: func(notification: notification) -> result<u64, host-error>;
+
+    /// Closes a notification of this extension; no event is sent. Unknown or closed ids are
+    /// ignored.
+    dismiss-notification: func(id: u64);
+
+    enum progress-location {
+        /// One host status-bar item per extension, showing its newest running progress; its
+        /// tooltip lists all of them.
+        status-bar,
+        /// A bar under the header of the owner, which must be a panel.
+        panel,
+    }
+
+    record progress-options {
+        /// The surface or decoration whose `handle-event` gets `progress-cancel-requested`; a
+        /// panel for `location: panel`.
+        owner: string,
+        /// 1 to 80 bytes of plain text on one line.
+        title: string,
+        location: progress-location,
+        /// Draws a Cancel button that sends `progress-cancel-requested`.
+        cancellable: bool,
+    }
+
+    variant progress-value {
+        indeterminate,
+        /// 0 to 100.
+        percent(u8),
+    }
+
+    /// Starts a progress, indeterminate until the first `progress-update`, and returns its id.
+    /// At most 4 running progresses per extension and 30 starts per rolling 60 s.
+    progress-start: func(options: progress-options) -> result<u64, host-error>;
+
+    /// Sets the value and, when given, a one-line message (at most 120 bytes). The host redraws
+    /// at most 10 times a second and keeps the last value. At most 64 updates per export call.
+    progress-update: func(id: u64, value: progress-value, message: option<string>) -> result<_, host-error>;
+
+    /// Ends a progress. Unknown or ended ids are ignored. Disabling, reloading or unloading the
+    /// extension, or removing the owner, ends its progresses too.
+    progress-end: func(id: u64);
+}
 
 world ui-extension {
     use types.{surface, node, node-id, ui-event, update, list-row};
 
     import host;
     import decorations;
+    import notifications;
 
     /// Called once after instantiation; declares the static surfaces. More can be added with
     /// `add-surface`.
```

Ids are `u64`, chosen by the host, increasing, never reused within an instance, and separate for
notifications and progresses (A's id rule for host-chosen ids, 0.3 §0.5).

### M.7 Changes by component

- **WIT (W0, after review):** the diff above.
- **Model (W2c), `crates/ui_ext/src/v0_2.rs`:**
  - `Notification`, `NotificationAction`, `NotificationLevel`, `ProgressOptions`,
    `ProgressLocation`, `ProgressValue`;
  - `UiEvent::{NotificationAction, NotificationClosed, ProgressCancelRequested}`;
  - the text-rule check, shared with `fyx_registry`'s rules where possible.
- **Host (W2c), `crates/ui_ext_wasm`:**
  - link `notifications` for 0.2;
  - `HostEffect::{Notify, DismissNotification, ProgressStart, ProgressUpdate, ProgressEnd}`;
  - in `UiState`: the rolling-window counters, open and running counts, id counters, and the
    owner check against live surfaces and decorations;
  - the 0.2 conversions.
- **Renderer (W3), `crates/ui_ext_host`:**
  - **new `notifications.rs`:** a `MessageNotification`-style view with the provenance line,
    level icon, up to two buttons with the 750 ms guard, the auto-dismiss timer with pause on
    hover and in the background, and close-reason tracking;
  - **new `progress.rs`:** the per-extension status-bar item, the panel bar in `panel.rs`'s
    header, and redraw coalescing;
  - **`registry.rs`:** apply the effects, route events to the owner, and close or end on owner
    removal and on unload.
  - **No upstream hook expected:** `Workspace::show_notification` and `dismiss_notification` are
    public, and status items come from W3's existing registration.
- **SDK (W4), `crates/ui_ext_sdk`:**
  - `notify(Notification) -> Result<NotificationId, HostError>`, `dismiss_notification`;
  - a `Progress` guard whose `Drop` calls `progress-end`;
  - builders, and the three events in `Extension::handle_event`;
  - `take_host_effects` records the five effects;
  - the harness (`fyx_ext_harness`) gets the caps and the events.
- **Sidecar (W8):** maps `showInformationMessage`, `showWarningMessage` and `showErrorMessage`
  (up to two items) and `withProgress` (`ProgressLocation.Notification` maps to `status-bar`,
  `Window` to `status-bar`, `SourceControl` and views to `panel`).

### M.8 Tests

- **Host:**
  - `not-permitted` without the capability, and an unknown owner;
  - each text rule (a newline in the title, a bidi override, 81 bytes, a third action, a
    duplicate action id);
  - the 6th `notify` in 60 s (`limit-exceeded`, a log line) and the 4th open one;
  - `percent(101)`, a `panel` progress owned by a status item, the 5th running progress, the
    65th update in one call;
  - a trap after `notify` shows nothing (A).
- **Renderer:**
  - the provenance line is present and cannot be altered by any field;
  - the button click guard (a click at 500 ms is ignored, one at 800 ms is sent);
  - auto-dismiss pauses on hover and in the background;
  - `notification-action` reaches `handle-event(owner)` and grants an activation that a
    following `show-surface` of a modal consumes;
  - `notification-closed(expired)` and `(user)`; no event on `dismiss-notification`;
  - owner removal closes and ends without events;
  - status-bar progress shows the newest and lists all; updates coalesce to 10 per second;
  - Cancel is disabled after the request, and the progress stays until `progress-end`.
- **SDK and harness:** the `Progress` guard ends on drop; the effects are recorded; the demo
  gallery gets a "Notify" and a "Run job" button.

## Rejected and deferred

| Proposal | Source | Why not in 0.2 |
|---|---|---|
| Raw colours or a palette index | `vscode-coverage.md:105` | "Theme tokens only" is an architecture rule. Decorations (L) use the same tokens. |
| `visibility-changed` event for panels | VS Code `onDidChangeVisibility` | No consumer; the host already throttles ticks by visibility. |
| `update-surface` (rename, move) | `TreeView.title` | Remove plus add covers it; deferred. |
| Command contributions (palette entries that open a modal) | W8 keybindings (`fyx_vscode::RunCommand`) | A separate host feature (actions, keymap) with its own capability; 0.2 modals open from buttons, rows and status items. |
| `row-node-click` | integration-1 #4 | See H; deferred to 0.3. |
| `ignoreFocusOut` | QuickPick | See C; would block other modals. |
| Extension SVG icons | — | See I. |
| Synchronous tree validation of pushes | W8-3 | See F. |
| Webviews (`ui:web`) | `vscode-coverage.md` API gaps | Layer 6, out of scope by design. |

## Changes by component (file level)

| Component | Files | Changes |
|---|---|---|
| WIT (W0) | `crates/ui_ext/wit/fyx-ui.wit` stays where it is (0.1, unchanged); new `crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit` (this design's WIT). **Landed** (plan 2.1) | migration.md §1 |
| Model | `crates/ui_ext/src/types.rs` | `Icon`, `IconRef`, `ModalDescriptor`, `ModalWidth`, `DismissReason`, `RowRange`; new fields `ButtonProps.tooltip`, `InputProps.secret`, `ListProps.selected`, `ListRow.indent`; `Patch::InvalidateRows`; `UiEvent::{RowSelected, Dismissed}`; `UiEvent::target() -> Option<NodeId>`; `SurfaceKind::Modal` (B, C, G, I, J, K) |
| Model | `crates/ui_ext/src/tree.rs` | `InvalidateRows` validation, `TreeError::NotAList`, `Touched` result with row invalidations (G); `replace` checks `max_nodes` (16384) instead of `max_nodes_per_update`, for both versions, since 0.1 only gains (Summary) |
| Host | `crates/extension_host/src/wasm_host/fyx_ui.rs` | version dispatch, `HostEffect` list replacing `pushes`, `Reply { effects, value }`, `ReplyError { effects, .. }`, live surface set, activation timestamp, `tick()`, `ui:modal`, `MAX_VERSION = 0.2.0` (A–F) |
| Host | new `crates/extension_host/src/wasm_host/fyx_ui/since_v0_1_0.rs`, `since_v0_2_0.rs` | one `bindgen!` and its conversions each; `fyx_ui.rs` keeps the shared runtime (migration.md) |
| Host tests | `crates/extension_host/src/wasm_host/fyx_ui/tests.rs`, `fixtures/probe` | see migration.md, "Test plan" |
| Renderer | `crates/ui_ext_host/src/backend.rs` | `UiBackend::{tick, api_version}`; `PushedUpdate` → `HostEffect` |
| Renderer | `crates/ui_ext_host/src/surface.rs` | `Phase::Closed`, show/hide, tick scheduling and error counter, `QueuedItem`, `selected` echo, range invalidation, version-dependent refetch rule, secret redaction (B, C, D, G, K) |
| Renderer | `crates/ui_ext_host/src/registry.rs` | `apply_effect` (add, remove, show, hide), active-window tracking, per-extension `TickBudget` (B, C, E) |
| Renderer | `crates/ui_ext_host/src/panel.rs` | visibility reporting, `IconRef` (B, I) |
| Renderer | new `crates/ui_ext_host/src/modal.rs` | `ExtensionModal: ModalView` (C, D) |
| Renderer | `crates/ui_ext_host/src/view.rs` | icon match, spinner, blank, tooltip, indent, selected highlight and scroll, masked input, modal header (C, I, J, K) |
| Upstream hook | `crates/workspace/src/status_bar.rs` | `remove_item_by_id`, about 8 lines, marked `fyx:` (E) |
| SDK | `crates/ui_ext_sdk/src/ui_ext_sdk.rs`, `host_model.rs`, `Cargo.toml` | bindings for both versions in every build, the world chosen in `register_extension!`, `Extension::tick`, host-function wrappers returning `HostError`, `SurfaceTree::push` and the unsynced flag, `SurfaceTree::mount` capped at `max_nodes`, `invalidate_rows`, builders for the new fields, `place_decoration`, `take_host_effects` (A–L); migration.md, "SDK" |
| Demos | `extensions/fyx_demo_counter`, `fyx_demo_todo`, `fyx_demo_gallery` | clock status item (ticks), a picker modal, `invalidate-rows` instead of re-keying, frozen icons |
| Sidecar | `sidecars/vscode-host/src/schema.ts`, `host.ts`, `mock-host.ts`, `ui/surfaces.ts`, `ui/quickpick.ts`, `ui/treeview.ts`, `ui/output.ts`, `ui/statusbar.ts`, `test/*.test.ts` | the JSON mirror of every type change, version negotiation in `initialize`, `fyx/surfacesChanged` and `fyx/surfaceClosed` replaced by WIT-equivalent requests (C–K) |
| M (drafted) | see M.7 | `notifications` interface, three `ui-event` cases, notification and progress views, caps |
| Docs | `docs/fyx/sdk-reference.md`, `extension-authoring.md`, `limits-and-gotchas.md`, `docs/vscode-coverage.md`, `tasks/wit-0.2.md` | new APIs, the limits table from B, C, E and M, coverage updates, close the backlog rows |

## Decisions (2026-11-17)

The owner decided the judge's three owner questions (wit-0.2-design, "Decisions for the user"),
and the integrator took the judge's recommendations for the rest. They are applied in the sections
above.

| Question | Decision | Where |
|---|---|---|
| Decorations in 0.2? | **In.** `fyx:ui/decorations` as drafted. | L, WIT |
| Frozen icon enum or strings? | **The frozen 101-icon enum.** A new icon needs a new world version. | I |
| May a modal open after a gesture, and what is a gesture? | **Within 5 s after a click or Enter only**: `click`, `row-click`, `input-submitted`. Typing (`input-changed`) is not a gesture, nor are `row-selected` and `dismissed`. | C, WIT `show-surface` |
| Keyboard selection in pickers? | **Ships with 0.2.** | C |
| Tick floor | **100 ms.** | B, WIT `request-tick` |
| `StatusBar::remove_item_by_id` hook | **Accepted**, about 8 lines marked `fyx:`. | E |
| `replace` / `mount` cap | **16384 nodes**, the tree cap. | Summary, F, WIT |
| Staging | **Stable and Preview capped at 0.1** until the commit that declares 0.2 frozen; Dev and Nightly accept 0.2. | migration.md §2 |
