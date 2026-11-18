# Limits and gotchas

What the `fyx:ui@0.1.0` host enforces, and the places where the prototype behaves in ways you
would not guess. Every rule cites the code it comes from. If you find a mismatch, the code wins:
please report it.

Contents:

- [Time budget](#time-budget)
- [Node and patch caps](#node-and-patch-caps)
- [Errors and recovery](#errors-and-recovery)
- [Rows: row-local ids, `row-click` and refresh](#rows-row-local-ids-row-click-and-refresh)
- [Input text ownership](#input-text-ownership)
- [Icon names and fallback](#icon-names-and-fallback)
- [`push-update` only during a call](#push-update-only-during-a-call)
- [Containers](#containers)
- [Rendering limits](#rendering-limits)
- [Sandbox](#sandbox)
- [Versions](#versions)
- [What changes in 0.2](#what-changes-in-02)

## Time budget

| Call | Budget |
|---|---|
| `handle-event`, `list-rows`, and `mount` after a surface's first | **200 ms** each |
| the first `mount` of each surface | **2 s** |
| instantiation, then `surfaces` (your `Extension::new` runs inside `surfaces`) | **2 s** each |

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:63-65, 476-495, 710-715; crates/ui_ext_sdk/src/ui_ext_sdk.rs:225-238 -->

- The budget is **wall-clock and includes time blocked in WASI**. A guest that keeps executing is
  interrupted by an epoch tick (every 10 ms) once it passes the budget. A guest blocked in a host
  call (`std::thread::sleep`, which ends up in `wasi:io/poll`) gets no epoch callback, so the call
  is also raced against a deadline of budget + 50 ms, and dropped when that passes.
- **Over budget means disabled.** A call that is interrupted or dropped, traps, or panics (a Rust
  panic on `wasm32-wasip2` is a trap) disables the extension. Every later call fails at once with
  `disabled after an earlier failure: <reason>`. **Restart extension** on the stopped surface (or
  `fyx_ui: restart UI extension`) loads it again on a fresh instance; a dev extension also
  restarts by itself on its next build.
- Calls of one extension are serialized on its own queue. A slow call delays that extension's
  next event, but never Fyx itself or other extensions.
- Every 10 ms tick, a running call also yields to the executor.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:5-9, 62, 66-68, 378-389, 466-474, 693-734; tasks/progress/W2.md:47-48 -->

What to do: keep every call well under 200 ms, including debug builds, which are much slower
than release builds. Do heavy setup lazily, in `new`/`surfaces` or in a surface's first `mount`
(2 s each); a remount (after **Retry**, say) gets 200 ms. Never sleep or spin
waiting for something: there is nothing to wait for, since no other code runs while your call
does.

## Node and patch caps

`TreeLimits::default()`:

| Cap | Value | Host applies it to | `SurfaceTree` applies it to |
|---|---|---|---|
| `max_nodes_per_update` | **2048** | patches in one `patches` update; nodes in one list row | queued patches (an inserted subtree counts one per node) |
| `max_nodes` | **16384** | nodes in the whole tree, so also in a `mount` or `replace` | the mirror after any edit |
| `max_depth` | **64** | levels from a root to its deepest node (a root is level 1), in the tree and in each list row; a deeper insert fails as `TooDeep` and the whole update is rejected | the same, on the mirror |

<!-- crates/ui_ext/src/tree.rs:9-25, 96-104, 168-173, 190-191, 255-258; crates/ui_ext_sdk/src/ui_ext_sdk.rs:605-618, 685, 717-718, 752-753 -->

A `mount` or `replace` through `SurfaceTree` is capped at `max_nodes` (16384), as in the host,
so a tree grown past 2048 nodes with `insert` across several events can always be mounted again.
Large collections still belong in virtualized lists.

Other caps:

| Cap | Value |
|---|---|
| pushes per call | 64. The 65th traps the call and disables the extension. |
| rows cached per list on the host | 1024, then pruned around the latest request (invisible to you, except as extra `list-rows` calls) |

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:69-71, 277-287; crates/ui_ext_host/src/surface.rs:17-18, 428-432 -->

Exceeding a host cap in an update is not fatal: the update is rejected as a whole (next section).

## Errors and recovery

| What happens | Host reaction | Extension |
|---|---|---|
| `mount` returns `Err`, or a tree the host rejects | The panel shows the error text instead of the tree. Not retried. | stays enabled |
| `handle-event` returns `Err` | Logged; the message shows in the panel footer (click to dismiss). The tree is unchanged. | stays enabled |
| an update (reply or push) that does not apply to the retained tree | Logged; `Invalid update from the extension: …` in the footer; the surface is mounted again, at most one remount in flight at a time | stays enabled |
| `list-rows` answer with an invalid row or an index outside `start..start+count` | The whole answer is rejected and the error shown. That list is not asked again until it changes. | stays enabled |
| trap, panic, time budget, push cap | The call fails. | **stopped** until Restart extension (or, for a dev extension, its next build) |

<!-- crates/ui_ext_host/src/surface.rs:130-161, 240-263, 287-343, 404-443; crates/ui_ext_host/src/view.rs:289-346; crates/ui_ext_wasm/src/ui_ext_wasm.rs:579-592, 640-660, 663-689, 716-721 -->

Gotchas:

- **The extension is never told an update was rejected.** It only sees the next `mount`.
  `SurfaceTree` exists to make that rare: it checks the same rules before queueing.
- **Handle `mount` being called more than once.** Rebuild from your state, and store the new node
  ids every time. The old ids are dead. The counter demo is written this way.
- **Status items have no error footer.** Their errors only reach the log.

<!-- crates/ui_ext_host/src/view.rs:346 (.when(!compact, …)) -->

## Rows: row-local ids, `row-click` and refresh

A `list` or `table` node has no children. The host pulls its rows with
`list-rows(surface, list-node, start, count)` as they scroll into view: the visible range plus 8
rows of overscan, clamped to `row-count`. While a fetch for a list is in flight, only the latest
newer range is kept and asked for next.

<!-- crates/ui_ext/wit/fyx-ui.wit:57-62, 120-124, 189; crates/ui_ext_host/src/surface.rs:20, 355-402, 470-477; crates/ui_ext_host/src/view.rs:254-277 -->

**Row-local ids.** Each `list-row` is a separate subtree whose single root has `parent: none`. Its
ids are **local to that row**: the host stores rows per `(list node, index)` and never merges them
into the surface's id space. The same ids may (and with the SDK, do) appear in every row. The SDK
starts row ids at `ROW_NODE_ID_BASE` (`0x8000_0000`), so a row id can never be confused with a
main-tree id. The WIT itself does not say this yet (0.2 item 3).

<!-- crates/ui_ext_host/src/surface.rs:28-29; crates/ui_ext_sdk/src/ui_ext_sdk.rs:93-98, 376-378; tasks/wit-0.2.md:10 -->

**`row-click`, not `click`.** Because row ids are ambiguous, nothing inside a row sends its own
event. Buttons in rows are inert, inputs in rows render as read-only labels, and markdown renders
as plain text. A click anywhere on the row sends `row-click(list node, row index)`, which is the
same for tables (with the table's id).

<!-- crates/ui_ext_host/src/view.rs:444-451, 472-485, 585-605, 638-652 -->

**When the host asks for rows again.** There is no "rows changed" patch in 0.1. The host drops
its cached rows for a list or table, and re-requests the visible range once, when an applied
update **touches** the list node: a `set-props` on it, its insertion, or a `replace`. A typical
case is the `row-count` changing. Old rows stay on screen until the new ones arrive, so there is
no blank flash.

<!-- crates/ui_ext/src/tree.rs:156-165; crates/ui_ext_host/src/surface.rs:300-311, 455-460 -->

Gotchas:

- **Rows that change while the list's props do not are not refreshed.** For example, a filter
  might keep the same number of matches but different items. The host does not compare props, so
  a `set-props` that repeats the list's current props also counts as a touch. But
  `SurfaceTree::set_props` drops an unchanged `set-props` before it is sent. The workaround the
  todo demo uses is `SurfaceTree::rebuild(list, …)`. That removes the list and inserts a new list
  node with a **new id**, so the host requests every visible row. Two costs follow:
  - the scroll position resets, since the scroll handle belongs to the old id;
  - `list-rows` and `row-click` for the old id can still arrive, so ignore them (match on the
    current list id).

  To keep the scroll position, rebuild only when the visible rows really changed, as the todo
  demo does.

  <!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:681-683, 730-757; extensions/fyx_demo_todo/src/fyx_demo_todo.rs:309-350, 477-493; crates/ui_ext_host/src/view.rs:146-149 (scroll handles keyed by node id) -->
- `rebuild` fails with `IsRoot` on a root node: put the list inside a flex node.
- **Rows you leave out are not asked for again** until the list changes. That covers indices you
  did not return and rows `list_rows` dropped as invalid. The host draws them as empty rows. A
  failed `list-rows` answer is not retried either.
- `row-height` is fixed for every row of a list, in pixels; 0 means 24. Table rows are always
  24 px. Content taller than the row is clipped.
- Lists and tables inside list rows are not supported (they render a notice).
- In a table, if a row's root is a flex node, its children become equal-width cells. Nothing
  matches them to `columns`, so give the row one child per column. Otherwise the root fills the
  row.

<!-- crates/ui_ext_host/src/surface.rs:36-43, 362-376, 434-442; crates/ui_ext_host/src/view.rs:509-627 -->

## Input text ownership

The user owns an input's text while typing. The rules, in order:

1. Typing is sent as `input-changed(id, text)` once the user pauses for **150 ms**. A newer queued
   `input-changed` for the same input replaces an older one that has not gone out yet.
2. **Enter** sends `input-submitted(id, text)` and cancels the pending `input-changed`, since the
   submit carries the text.
3. The host writes what the user typed into its own copy of the tree immediately, so its tree
   matches the screen. That write is not an extension change.
4. Whenever an applied update **sets** an input's value (`set-props` on it, an insert, a
   `replace`), the host writes the value into the editor if it differs from the editor's text,
   **even while the field is focused**.

<!-- crates/ui_ext_host/src/view.rs:30, 198-252; crates/ui_ext_host/src/surface.rs:181-189, 221-238, 312-313; tasks/progress/W3.md:52 (integrator decision 6) -->

So, from the extension side:

- **Do not echo the typed value back with `set_props`.** By the time your reply lands, the user
  may have typed more, and rule 4 overwrites the newer text with your older value. Record the
  value with `SurfaceTree::set_props_from_host(id, InputProps::new(text) …)` instead. That updates
  your mirror without sending anything.
- **Clearing after submit needs the submitted text recorded first.** If your mirror still holds
  `""` (because you never recorded the typing), `set_props(id, InputProps::new(""))` is "no change"
  and is dropped, so the field keeps the text. Call `set_props_from_host` with the submitted text,
  then `set_props` with the empty value. The input example in the
  [authoring guide](extension-authoring.md#an-input) does this.
- Keep the placeholder identical in both calls, or the `set_props` also sends a placeholder change.
- Setting a value deliberately (a "clear" button, a programmatic edit) is fine: rule 4 makes it
  win.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs:677-703; extensions/fyx_demo_todo/src/fyx_demo_todo.rs:443-455 -->

## Icon names and fallback

Icons (button `icon`, `icon` nodes, a panel's dock icon) are **strings parsed as upstream Zed's
`IconName` in snake_case**: `plus`, `dash`, `hash`, `list_todo`, `rotate_cw`, and so on. The list
is whatever `crates/icons/src/icons.rs` contains in the Fyx build the user runs. It follows
upstream, so a name can disappear on a rebase. An unknown name does not fail: it renders the
`blocks` icon.

<!-- crates/ui_ext_host/src/view.rs:749-752; crates/ui_ext_host/src/panel.rs:39-49; crates/icons/src/icons.rs:6-10 (strum snake_case) -->

- A button with an empty label and an icon renders as an icon-only button.
- Status items have no icon of their own.
- 0.2 plans a frozen icon list, so the ABI stops following upstream (item 4 below).

## `push-update` only during a call

`host.push-update(surface-id, update)` lets an extension change a surface outside a reply, and the
WIT describes it as being for "timers, background work". In 0.1 it **cannot do that**. A
component only runs while the host is calling it, and there is no tick or timer export, so there
is no moment at which a timer could fire. Pushes are useful only for updating a *different*
surface of the same extension during a call (and the same surface works too).

<!-- crates/ui_ext/wit/fyx-ui.wit:166-168; tasks/wit-0.2.md:9 -->

What happens to pushes:

| Pushed during | Fate |
|---|---|
| `handle-event`, `list-rows` (call succeeds) | Applied in order, **before** the call's reply. A push for another surface goes to that surface. |
| `mount` | Dropped for the surface being mounted (the returned tree supersedes it). Applied for other surfaces. |
| instantiation, `surfaces` | Sent to the registry. The surfaces are normally still mounting then, and an update that arrives before a surface's mount finishes is dropped. |
| a call that returns `Err` or fails validation | The runtime returns them inside `ReplyError`, but the current renderer **drops them**. |
| a call that traps or runs out of time | Discarded; the extension is disabled. |

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:206-241, 498-502, 716-722; crates/ui_ext_host/src/surface.rs:137-145, 240-285, 291-298, 412-417; crates/ui_ext_host/src/registry.rs:109-156 -->

Gotcha: **do not push and then return `Err` in the same call.** The runtime keeps those pushes for
the renderer, as `tasks/progress/W2.md` specifies, but `ui_ext_host` does not apply them yet. If
you built the push with `SurfaceTree::take_update`, your mirror now has changes the host never
saw, and later patches may be rejected until the next remount resyncs you.

Other rules: at most 64 pushes per call (more disables the extension), and a push for an unknown
surface id is logged and dropped.

## Containers

**Panels**

- Dock `left`, `right` or `bottom`. `default-size` is in pixels, clamped to 120..=2000. An empty
  title falls back to the surface id.
- Extension panels sort after every built-in panel, in load order.
- The user can move a panel to another dock for the session only. The descriptor decides again on
  the next start.
- Every extension panel shares one persistent name and panel key. Sizes persist together across
  all extension panels, and dock restore reopens the first one. Panels added after dock
  restoration do not restore their open state.
- The dock button's tooltip is the title, and its string is leaked once per loaded surface.

<!-- crates/ui_ext_host/src/panel.rs:24-60, 124-145; crates/ui_ext_host/src/registry.rs:35-36; tasks/progress/W3.md:157-163 -->

**Status items**

- Side `left` or `right`. **`priority` is ignored**: the status bar keeps insertion order.
- Laid out compactly in a row, with no error footer.

<!-- crates/ui_ext_host/src/registry.rs:251-262; crates/ui_ext_host/src/view.rs:284-288, 346 -->

There are no modals, editor tabs, or surfaces added after `surfaces()` in 0.1, and no event tells
you that a surface was closed (0.2 items 5–7).

## Rendering limits

- Layout is flex only. Flex has direction, gap, padding (pixels), grow, cross-axis align and a
  background token. There is no justify, size, border, scroll container, tooltip or indent.
- Text is a single `Label` with optional bold, monospace and colour token.
- Only theme tokens: `text`, `text-muted`, `text-accent`, `error`, `warning`, `success` map to
  `ui::Color` for text and icons, and `border`, `surface`, `elevated-surface` map to the theme's
  colours.
- `diff-view` is a placeholder: shared leading and trailing lines as context, the rest as removed
  then added lines, in the buffer font. `language` is unused. It is not Zed's diff editor.
- `markdown` uses Zed's markdown element outside rows, and plain text inside rows.

<!-- crates/ui_ext_host/src/view.rs:401-438, 638-690, 763-794; tasks/progress/W3.md:164 -->

## Sandbox

- All of WASI p2 is linked (Rust's std imports parts of it), with only stderr inherited (Rust
  `eprintln!` reaches Fyx's stderr). There are no preopened directories, environment variables or
  arguments.
- No network: TCP, UDP and name lookup are denied and no address passes the check, so `connect`
  fails with "access denied" inside the component.
- Store caps: 256 MiB of linear memory (room for a JS engine), 64 Ki table elements, 32 core
  instances, 32 tables, 8 memories. Growing past one traps the call and disables the extension;
  other extensions are not affected. The pushes of one call may carry 4096 patches plus nodes
  together, and at most 64 pushes. The strings passed to `push-update` and `log` in one call may
  total 16 MiB; a `log` message is cut at 16 KiB.
- An installed extension loads only after the user enables it, for the capabilities and the
  component (or linked folder) it has then; see "Installing as a dev extension" in
  `extension-authoring.md`.
- No access to the worktree, buffers or settings. The todo demo serves a mocked worktree for this
  reason.
- Fyx's own host functions: `push-update` and `log`.
- Links in `markdown` nodes open only if they are `http` or `https`, and only after the user
  confirms the address; other schemes (`file:`, `javascript:`, custom ones, relative paths) do
  nothing.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:452-453; extensions/fyx_demo_todo/src/fyx_demo_todo.rs:3-4; crates/ui_ext/wit/fyx-ui.wit:163-173 -->

## Versions

- **This build supports exactly `fyx:ui` 0.1.0** (`MIN_VERSION = MAX_VERSION = 0.1.0`), on every
  release channel. Zed's stable/preview cap on its own extension API does not apply.
- The component declares its version in the custom section `fyx:ui-api-version`: three
  big-endian `u16`s, which `register_extension!` writes. A component without the section falls
  back to the version of an imported `fyx:ui/host@…` or `fyx:ui/types@…` interface. With neither,
  it is rejected ("not a fyx:ui component").
- Two copies of the section (12 bytes) are rejected. Emit it only through `register_extension!`.
- From a directory, `[fyx_ui].api_version` must be supported **and** equal the component's
  version, or loading fails with `extension.toml says fyx:ui X, the component targets Y`.
- The SDK's `API_VERSION` is `(0, 1, 0)`. Rebuild against the matching SDK when the world changes.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:55-60, 73-130, 171-175, 420-440, 551-556; crates/ui_ext_sdk/src/ui_ext_sdk.rs:74-91, 210-215 -->

Expect 0.2 to be a breaking change. Nothing in 0.1 promises that a 0.1 component will load in a
host that supports 0.2.

## What changes in 0.2

`fyx:ui@0.1.0` is frozen for the sprint. These are the integrator's **candidates** for one 0.2
world, decided together after Phase 4. None of them is implemented, and each has a workaround
today.

| # | Candidate | Why | Workaround in 0.1 |
|---|---|---|---|
| 1 | `invalidate-rows(node-id)` patch | rows change while the list's props do not | `SurfaceTree::rebuild` the list (new id; loses scroll position) |
| 2 | `request-tick(ms)` import + `tick` export | `push-update` cannot drive timers, because a component runs only inside a host call | none: no clocks or progress status items |
| 3 | Document row-local ids in the WIT | row ids are row-local, and clicks in rows are `row-click` | the host never maps row ids; the SDK uses `ROW_NODE_ID_BASE` |
| 4 | Frozen icon list instead of upstream `IconName` strings | the ABI must not follow upstream's icon enum | names parsed as `IconName`, fallback icon |
| 5 | `surface-kind::modal` | quick picks, input boxes | a panel in the bottom dock |
| 6 | Add and remove surfaces at runtime | surfaces created after `surfaces()` | none for WASM components (the VS Code sidecar uses an extra `fyx/surfacesChanged` notification) |
| 7 | `ui-event::dismissed` | the user closed a surface | none for WASM components (the VS Code sidecar uses an extra `fyx/surfaceClosed` request) |
| 8 | `indent` prop on rows, `tooltip` on buttons | tree views, icon buttons | flattened labels, no tooltip |

<!-- tasks/wit-0.2.md:1-17; details for 5–8 in docs/vscode-coverage.md ("Schema gaps") -->

Plan for these: keep row-building and icon names in one place in your code, and do not depend on
`push-update` for anything time-based.
