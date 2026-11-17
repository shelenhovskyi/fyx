# Judge review: integration-1 (W2 + W4)

Scope: `git diff fyx/main...integration` at `52317345a4` (W2 `3d022ab7a8`, `429ddaf400`; W4
`ed374f89d8`; W0 `52317345a4`). Code read, nothing compiled. Progress-file claims were checked
against the code, not taken on trust.

## Verdict

Mergeable as a base. No rule is broken outright: the host API is async-only, so it cannot be called
from `render()`; no raw colours cross the boundary; the upstream hooks carry `fyx:` markers; the
Apache crates depend only on `serde`, `thiserror` and `wit-bindgen`; no stream wrote outside the
paths it owns. The problems are in the **contract between the host and the SDK**. W3 is building on
it right now, so findings 1–4 should be settled before W3 merges.

## Findings, most severe first

### 1. High: the time budget is not enforced while a guest is blocked in a WASI host call (W2)

`crates/extension_host/src/wasm_host/fyx_ui.rs:285-293` enforces `UI_CALL_BUDGET` only from the
epoch-deadline callback, and wasmtime runs that callback only while guest wasm code is executing.
`fyx_ui.rs:208` links all of WASI p2, including `wasi:clocks/monotonic-clock` and `wasi:io/poll`, which
both demos import (W4 progress). A guest that calls `std::thread::sleep(Duration::from_secs(60))` in
`handle-event` waits inside the async host `poll`. No epoch callback fires during that wait, so
nothing interrupts the call. The UI queue (`fyx_ui.rs:320-324`) is serialized, so every click on the
extension's surfaces is stuck behind it for 60 s and the extension is never disabled. This breaks
the ARCHITECTURE rule "slow extensions are disabled by time budget". The existing test covers only a
busy loop (`tests.rs:177`).
**Fix (W2):** in `call` (`fyx_ui.rs:438-474`), race `f(extension, store)` against a timer of
`budget + slack`. On timeout, drop the call future and set `disabled`. Add a fixture event that
sleeps for 1 s and assert that it is interrupted at about 200 ms.

### 2. High: a rejected update desynchronizes the SDK mirror permanently (W4 + W3)

`ui_ext::Tree` applies an update atomically and rejects the whole batch on error
(`crates/ui_ext/src/tree.rs:179-195`). `SurfaceTree` in the SDK commits each edit to its own mirror
*before* the host sees it (`crates/ui_ext_sdk/src/ui_ext_sdk.rs:581`, `:605`). Nothing reports a
rejection back to the guest. After one rejected batch, the guest's view and the host's view differ
for the life of the surface. Later patches name nodes the host never inserted, those batches are
rejected too, and the panel freezes. The SDK also skips rules the host enforces, so its own API can
produce batches the host rejects:
- `SurfaceTree::insert` (`:605`) accepts a non-`flex` parent; the host returns `NotAContainer`
  (`tree.rs:245`).
- `SurfaceTree::set_props` (`:581`) can turn a `flex` that has children into a leaf; the host
  rejects it at `tree.rs:217`.
- There is no check for more than 2048 patches per update (`TreeLimits`, `tree.rs:19`).

**Fix:** W3 calls `mount` again whenever applying an update fails. `SurfaceTree::mount` already
resets the mirror, so this restores sync. W4 checks the container rule and the patch cap in
`SurfaceTree`, so SDK misuse fails inside the guest instead of desynchronizing.

### 3. Medium: pushed updates and the reply to the same call have no defined order (W2 → W3)

`host.push-update` during a call goes to a separate unbounded channel (`fyx_ui.rs:146-157`). The
call's own reply comes back through a oneshot. The guest emits the pushed update first. The host
test applies the reply first and the push afterwards (`tests.rs:116-133`), and that works only
because the fixture's patches happen to be independent. Suppose an extension calls
`push_update(tree.take_update())` and then returns further patches that touch nodes inserted by the
push. If the renderer applies the reply first, the batch is rejected (see finding 2).
**Fix (W2):** collect the pushes made during a call in `UiState` and return them, in order, together
with the reply. For example, `handle_event` returns `Vec<Update>`. Only pushes made outside a call
would go to the channel, and with the current world there are none (see finding 9). W3 applies the
updates in that order.

### 4. Medium: row-local node ids contradict the WIT, so nodes inside rows cannot be addressed (W3 decides, W0 documents)

The WIT says a node id is "unique within one surface" (`fyx-ui.wit:5-6`). The SDK numbers every row
from `ROW_NODE_ID_BASE = 0x8000_0000` (`ui_ext_sdk.rs:75`), so all rows share the same ids. W2's
`list_rows` returns one `Tree` per row (`fyx_ui.rs:398-436`), which matches W4's assumption. However,
`click(node-id)` carries no row index. A button inside a row would send `click(0x8000_0001)`, and the
extension cannot tell which row was clicked. Neither demo puts interactive nodes in rows yet.
**Decision:** keep row-local ids, because surface-unique ids for virtualized rows would cost the
guest bookkeeping on every scroll. W3 must never enter row ids into the surface's id map. W3 reports
any click inside a row as `row-click(list, index)` and never as `click(id)`. W0 records this in the
WIT doc comments for 0.2.

### 5. Medium: row refresh depends on the renderer noticing a `set-props` on the list (W2 + W3 + W4)

The WIT has no "rows changed" patch. The counter's rows read `Row i of {count}`
(`fyx_demo_counter.rs:174-178`), so they go stale unless the host re-requests the visible rows when
the list's props change. `Tree::apply_update` returns only `bool` (`tree.rs:153`), so W3 cannot tell
*which* list changed.
- **W2:** return the ids touched by the update (`set-props` targets, plus inserted and removed
  ids).
- **W3:** drop the cached rows of every touched list node and re-request its visible range once.
- **W4:** the todo demo rebuilds the list under a new id on *every* filter keystroke
  (`fyx_demo_todo.rs:430`, `sync(true)`), even when the matching set is unchanged. The rebuild loses
  the scroll position and burns ids. Rebuild only when `visible` actually changed.
- **W0:** `invalidate-rows(node-id)` is the 0.2 fix.

### 6. Medium: echoing input text has no defined owner on the host side (W3)

On `input-changed`, the SDK records the typed text in its mirror without sending a patch
(`set_props_from_host`, `ui_ext_sdk.rs:594`). The host's `Tree` still holds the old `value`.
Example: the user types `abc`, then the extension clears the field with `set_props(value: "")`. The
SDK sends `set-props ""` because its mirror holds `abc`. The host tree goes from `""` to `""`. If W3
syncs the editor only when the tree's value *changes*, the field is never cleared.
**Fix (W3):** on `input-changed`, also apply `SetProps(input, value)` to the host tree locally, so
both mirrors agree. Write a `set-props` value into the editor whenever it differs from the editor's
text. Keep the user's text authoritative while the field is focused and no `set-props` arrives.

### 7. Medium: no stream owns loading a UI extension from disk (W0 to assign)

Nothing calls `UiExtension::load` (`fyx_ui.rs:232`). W4 proposes an `[fyx_ui]` table in
`extension.toml` (W4 progress, "Proposal"), and no task file owns it. W3's "done when" requires the
counter to show up in the dock, so this blocks phase 2.
**Proposal:** W2 parses `[fyx_ui]` inside the `fyx_ui` module with `toml`, without touching
upstream's `ExtensionManifest`, and exposes `load_from_dir`. W3 calls it and registers the surfaces.

### 8. Low: the SDK does not emit the `fyx:ui-api-version` section (W4)

W2 asked for it (shared note `W2-host-api.md`); `ui_ext_sdk.rs` has no `link_section`. Loading still
works through the fallback, because both demos import `fyx:ui/types@0.1.0`
(`fyx_ui.rs:93-103`). Add the five-line static from the note.

### 9. Low: `push-update` promises timers that the world cannot run (W0, 0.2)

The WIT (`fyx-ui.wit:166`) and the SDK docs (`ui_ext_sdk.rs:86`) say `push-update` serves
"timers, background work". A component executes only inside a host call, and the world exports no
tick or timer. So `push-update` is useful only during `mount`, `handle-event` or `list-rows`, for
example to update a *different* surface. A clock or progress status item cannot be built. Either
add a `request-tick(ms)` import and a `tick` export in 0.2, or reword the docs.

### 10. Low: pushed updates are unbounded (W2)

`mpsc::unbounded` (`fyx_ui.rs:117`) with no count or size cap. Within one 200 ms budget, a looping
guest can queue hundreds of MB of 2048-patch updates before any of them is validated. Cap the
pushes per call, and disable the extension when the cap is exceeded.

### 11. Low: mount and replace are capped at 2048 nodes, the tree at 16384 (W2)

`Tree::replace` checks `max_nodes_per_update` (`tree.rs:170`). A surface can grow to 16384 nodes
through patches, but it can never be mounted or replaced at that size. Use `max_nodes` for
`replace`, or document that 2048 is the real mount cap.

### 12. Low: documentation drift (W4)

- `fyx_demo_counter.rs:7` says mount returns "fixed ids" 1–7. `SurfaceTree` keeps its
  `IdAllocator` across mounts (`ui_ext_sdk.rs:511`), so a second mount, such as a panel reopen or
  the recovery from finding 2, returns 8–14. Write "on the first mount".
- Both icon sets used are valid snake_case `IconName` values (`hash`, `dash`, `plus`, `list_todo`,
  `rotate_cw`, `sparkle`; `crates/icons/src/icons.rs:9`). W3 should parse them with
  `IconName::from_str` and show a fallback icon for unknown names. Long term, the extension ABI
  should not be tied to upstream's icon enum: freeze a list for 0.2 (W0).

### 13. Low: fork-discipline nits (W0)

- `Cargo.toml` member and dependency lines for `ui_ext*` and `agent_review` have no `fyx` marker;
  only the demo lines do. `git grep 'fyx:'` therefore misses part of the upstream surface.
- The diff-measurement command in `tasks/W7-judge.md` counts the Fyx-owned module
  `crates/extension_host/src/wasm_host/fyx_ui*` (1866 lines, including an 825-line fixture
  lockfile) as upstream diff. Exclude it, as done below.

## Duplication between W2 and W4

| What | Where | Who changes |
|---|---|---|
| Patch semantics are implemented three times, with different validation | `ui_ext::Tree` (`tree.rs`), `SurfaceTree` (`ui_ext_sdk.rs:495-729`), the SDK's test `apply` (`ui_ext_sdk.rs:984`) | **W4.** `ui_ext` is Apache-licensed and free of wasmtime and GPUI, so the SDK can take it as a dev-dependency and test that its emitted updates apply cleanly to `ui_ext::Tree` (W0 adds the dep and the lock entry). That test would have caught finding 2. |
| Two counter guests with different contracts | W2 fixture `fyx_ui/fixtures/counter` (ids 1–5, left dock, 240 px, `SPIN`, push on reset) and W4 `fyx_demo_counter` (ids 1–7, right dock, 320 px) | **W2.** Keep the fixture only for what the demo cannot do (runaway call, push, blocking sleep from finding 1), rename it `fyx_ui_test_guest`, and add one test that loads the real `fyx_demo_counter`. |
| Row index clamping | SDK `row_range` (`ui_ext_sdk.rs:364`) and the host range check (`fyx_ui.rs:418-425`) | Nobody. These are complementary checks, not duplication. |

## Contract points W3 needs, in one place

1. Never call WASM from `render()`. The W2 API is async-only, so structurally that holds; keep it
   that way.
2. Row ids are row-local. A click inside a row is sent as `row-click(list, index)` (finding 4).
3. Re-request a list's visible rows after any update that touches the list node (finding 5).
4. Write the typed input text into the host tree; push `set-props` values into the editor (finding 6).
5. Apply the pushed updates of a call before its reply (finding 3). If applying fails, re-mount
   (finding 2).
6. Debounce typing and scrolling, so each input event makes one `handle-event` call.
7. Icons: `IconName::from_str` with a fallback (finding 12).

## Upstream diff (`v1.28.1..integration`)

Measured with
`git diff v1.28.1 --stat -- . ':!crates/ui_ext*' ':!crates/agent_review' ':!docs' ':!tasks' ':!extensions/fyx_*'`:
17 files, +2171/−0. Most of that is Fyx's own files:

| Kind | Files | Lines |
|---|---|---|
| **Edits to upstream files** | `Cargo.toml` (+10), `.cargo/config.toml` (+8), `crates/extension_host/Cargo.toml` (+1), `crates/extension_host/src/wasm_host.rs` (+1) | **+20** |
| Generated | `Cargo.lock` | +46 |
| Fyx module inside an upstream crate | `crates/extension_host/src/wasm_host/fyx_ui*` | +1866 |
| Fyx root files and scripts | `ARCHITECTURE.md`, `PLAN.md`, `FYX_UPSTREAM`, `.claude/CLAUDE.md`, `script/gate.sh`, `script/fyx-check-licenses-index` | +239 |

The hand-edited upstream surface is 20 lines in 4 files. That is well within the "few dozen lines"
target. Baseline for the next cycles: W1 and W3 are expected to add hooks in `crates/zed`,
`crates/workspace`, `crates/client` and `crates/paths`.
