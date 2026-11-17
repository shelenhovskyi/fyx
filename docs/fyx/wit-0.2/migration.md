# `fyx:ui` 0.1 → 0.2: coexistence, gating, SDK, test plan

Companion to [`design.md`](design.md) and [`fyx-ui-0.2.wit`](fyx-ui-0.2.wit). Nothing here has
been implemented yet. Every path is relative to the repository root, at `integration` `2e91298`.

**Goal.** A Fyx build that ships 0.2 loads both 0.1 and 0.2 components side by side, and treats
each one exactly as its world promises. A 0.1 component keeps 0.1 behaviour: it gets no ticks, has
no modals, and keeps the old row refetch rule. Fyx follows Zed's pattern for `zed:extension`: one
frozen WIT directory and one bindings module per version, a dispatch enum, and the version read
from a custom section before instantiation.

## 1. WIT layout (W0)

```
crates/ui_ext/wit/
  since_v0.1.0/fyx-ui.wit   # today's crates/ui_ext/wit/fyx-ui.wit, moved byte-identical, then never edited
  since_v0.2.0/fyx-ui.wit   # docs/fyx/wit-0.2/fyx-ui-0.2.wit (decided 2026-11-17), minus its header comment
```

This mirrors `crates/extension_api/wit/since_v0.x.y/`. Every package keeps its full world. Zed
copies files forward rather than sharing them across versions, and so does Fyx. A frozen version
cannot then change by accident when a shared file is edited.

Every reference to the old path moves in the same commit:

| Reference | Change |
|---|---|
| `crates/extension_host/src/wasm_host/fyx_ui.rs:42` (`bindgen!` path) | moves into `fyx_ui/since_v0_1_0.rs` with `path: "../ui_ext/wit/since_v0.1.0"` |
| `crates/ui_ext_sdk/src/ui_ext_sdk.rs:61` (`generate!` path) | one `generate!` per version, both in every build (section 4) |
| `crates/ui_ext_sdk/src/ui_ext_sdk.rs:1521-1523` (`api_version_matches_the_wit_package`, `include_str!`) | one check per version directory |
| `crates/ui_ext/src/ui_ext.rs:11` (`WIT_DIR`) | `WIT_DIR_V0_1_0`, `WIT_DIR_V0_2_0` |
| comments: `crates/ui_ext/src/types.rs:1`, `fyx_ui.rs:1`, `ARCHITECTURE.md:23`, `sidecars/vscode-host/README.md:6`, `docs/fyx/sdk-reference.md`, `docs/fyx/limits-and-gotchas.md` (`<!-- … -->` source anchors) | new path; anchors to line numbers in the 0.1 file stay valid, because the file is moved unchanged |

Commit order: (1) move 0.1 alone and run `script/gate.sh`; no behaviour may change. (2) Add 0.2.

## 2. Host bindings (W2)

```
crates/extension_host/src/wasm_host/
  fyx_ui.rs                 # runtime, budgets, UiState, UiExtension, Reply/HostEffect, dispatch
  fyx_ui/since_v0_1_0.rs    # bindgen! for 0.1 + From/TryFrom to ui_ext + 0.1 host import impls
  fyx_ui/since_v0_2_0.rs    # bindgen! for 0.2 + conversions + 0.2 host import impls
  fyx_ui/tests.rs           # existing
  fyx_ui/fixtures/probe     # existing; builds for both versions (section 6)
```

`fyx_ui.rs` is already a file module whose submodules live in `fyx_ui/` (`tests.rs` is there now),
so no `mod.rs` is needed.

**Dispatch.**

```rust
enum UiBindings {
    V0_2_0(since_v0_2_0::UiExtension),
    V0_1_0(since_v0_1_0::UiExtension),
}
```

The newest version comes first, as in `wit.rs`'s `Extension` enum. `UiBindings` has
`instantiate_async(version, store, component)`, `call_surfaces`, `call_mount`,
`call_handle_event`, `call_list_rows` and `call_tick`. On `V0_1_0`, `call_tick` is unreachable,
because no 0.1 component can call `request-tick`. It returns `Ok(Update::Unchanged)` instead of
panicking.

**Linkers.** `UiRuntime` (`fyx_ui.rs:302-343`) holds one `Linker<UiState>` per version. They are
built in `build_ui_runtime` with WASI and that version's `add_to_linker`. One shared linker would
also work, since the interface names differ (`fyx:ui/host@0.1.0` vs `@0.2.0`). Separate linkers
keep each version's imports exactly what its world declares, which is what Zed does. Each linker
costs a few kilobytes and is built once.

**Shared state.** `UiState` is version-independent. Both host-import implementations write into
the same `effects: Vec<HostEffect>`.

- The 0.1 `push-update` keeps its 0.1 behaviour. It has no result, and the 65th push of a call
  traps (`fyx_ui.rs:279-281`), so the existing test `test_push_flood_traps_and_disables_extension`
  stays valid.
- The 0.2 `push-update` returns `limit-exceeded` instead.

**Conversions.** `ui_ext` is the *latest* model, a superset of every world.

- 0.1 into the model: `icon: option<string>` → `Option<IconRef::Upstream(String)>`;
  `ListRow.indent = 0`; `ListProps.selected = None`; `ButtonProps.tooltip = None`;
  `InputProps.secret = false`.
- The model into 0.1: only events go this way. `TryFrom<ui_ext::UiEvent> for
  since_v0_1_0::UiEvent` fails for `RowSelected` and `Dismissed`. The host never produces them
  for a 0.1 surface: 0.1 has no modals, and keyboard selection exists only in modals. A failure
  is logged at debug level and the event is dropped, which is defence in depth.
- 0.2 maps one to one, with `IconRef::Frozen`.

**Version gating.**

- `parse_api_version` (`fyx_ui.rs:86-116`) is unchanged: the 6-byte section
  `fyx:ui-api-version` holds three big-endian `u16`s.
- `api_version_from_imports` (`fyx_ui.rs:120-130`) already accepts any version suffix of
  `fyx:ui/host@` and `fyx:ui/types@`. It needs no change, but gets a test per version (section 6).
- `MIN_VERSION = 0.1.0`, and `MAX_VERSION` becomes `0.2.0`. The supported set is explicit, not a
  semver range:

  ```rust
  match (version.major, version.minor, version.patch) {
      (0, 1, 0) => V0_1_0,
      (0, 2, 0) => V0_2_0,
      _ => bail!("extension {id} targets fyx:ui {version}, this build supports 0.1.0 and 0.2.0"),
  }
  ```

  For 0.x, every minor version is a different world, and no patch release is planned. If one ever
  ships, it may change only comments, and gets its own match arm.
- `UiManifest.api_version` must still equal the component's version (`fyx_ui.rs:551-556`).
- `check_surfaces` gains `ui:modal`. A 0.1 manifest that declares `ui:modal` is accepted: it is
  harmless, because a 0.1 component cannot declare a modal surface.

**Staging 0.2 before it is final** (decided 2026-11-17). Zed caps stable at an older version while it stages a new API
(`wit.rs:60-70`). Fyx does the same with upstream's `ReleaseChannel`:

```rust
pub fn api_version_range(channel: ReleaseChannel) -> RangeInclusive<Version> {
    let max = match channel {
        ReleaseChannel::Dev | ReleaseChannel::Nightly => Version::new(0, 2, 0),
        ReleaseChannel::Stable | ReleaseChannel::Preview => Version::new(0, 1, 0),
    };
    MIN_VERSION..=max
}
```

Remove the split, setting both arms to 0.2.0, in the commit that declares 0.2 frozen. That is the
point after which the 0.2 file gets the same "FROZEN" header as 0.1. This is a behaviour change to
`api_version_range()`, whose doc comment currently says Fyx does *not* cap by channel
(`fyx_ui.rs:73-79`). Update the comment.

**What older hosts do with a 0.2 component.** They reject it before instantiating it, with the
existing error "targets fyx:ui 0.2.0, this build supports 0.1.0..=0.1.0" (`fyx_ui.rs:434-439`,
`:171-175`). Nothing new is needed.

## 3. Renderer: behaviour by version (W3)

`UiBackend` (`crates/ui_ext_host/src/backend.rs`) gets `fn api_version(&self) -> Version`.
`SurfaceState` derives its rules from it once, at construction, as a
`struct ApiRules { precise_row_refetch: bool, ticks: bool, … }` built from the version, so no
version comparisons are scattered through the renderer.

| Behaviour | 0.1 component | 0.2 component |
|---|---|---|
| Rows refetched after an update | every touched list or table, any `set-props` (`surface.rs:418-425`) | only `invalidate-rows`, a change to `row-count`, `row-height` or `columns`, an insert or a replace (design G) |
| Icons | `IconName::from_str`, fallback `Blocks` (`view.rs:849-851`) | exhaustive `match` over `Icon` (design I) |
| `push-update` over the per-call cap | trap, extension disabled | `limit-exceeded`, push dropped |
| Surface ids | any string | `[a-z0-9._-]{1,64}`, checked when a surface is declared or added |
| Surfaces after `surfaces()` | fixed | `add-surface` / `remove-surface` |
| Ticks, modals, `show-surface`, `row-selected`, `dismissed` | never | design B–D |
| Mount timing | every surface at load (`surface.rs:113`) | panels and status items at add; modals at each show |

Where the existing 0.1 behaviour was a workaround, it is kept for 0.1 components only. Two
examples: the full refetch on any list `set-props`, and the sidecar-only channels. No 0.1
extension changes behaviour by being loaded in a 0.2 host.

## 4. SDK (W4)

`ui_ext_sdk` generates the bindings of **both** worlds in every build, and an extension picks its
world where it registers:

```rust
pub mod bindings {
    pub mod v0_2 {
        wit_bindgen::generate!({ path: "../ui_ext/wit/since_v0.2.0", world: "ui-extension", /* as today */ });
    }
    pub mod v0_1 {
        wit_bindgen::generate!({ path: "../ui_ext/wit/since_v0.1.0", world: "ui-extension", /* as today */ });
    }
}
```

```rust
register_extension!(MyExtension);        // fyx:ui 0.2, the default
register_extension!(MyExtension, v0_1);  // for Fyx builds that only know 0.1
```

**Why not Cargo features** (an earlier draft had exclusive `fyx-ui-0-1` / `fyx-ui-0-2` features
with a `compile_error!`). Cargo unifies features. An extension that selects 0.1 and depends on a
helper crate built on the SDK's default would get both features and fail to compile, with no way
out for the author (judge wit-0.2-design #3). With both modules always present, the SDK has no
features to unify. The probe fixture picks its world with a feature of its own (`probe-0-1`
switches its `register_extension!` arm), and a leaf crate's feature unifies with nothing.
`gate.sh` needs no feature check.

**Why it does not leak 0.2 into a 0.1 component.** The SDK uses `pub_export_macro: true`. With
it, wit-bindgen 0.41 emits two `component-type` sections. The `generate!` itself emits one with
the world's imports only. The `export!` macro, invoked by `register_extension!` in the
extension's own crate, emits one with the exports and the imports
(`wit-bindgen-rust` 0.41, `lib.rs` around line 1322). `wit-component` drops imports that the core
module never calls. So a 0.1 extension carries the 0.1 exports, plus only the imports it actually
calls. **This has to be verified before the SDK adopts it.** Build one demo with
`register_extension!(T, v0_1)` and check that `wasm-tools component wit` shows only `@0.1.0`
`fyx:ui` interfaces and no `tick` export. If a 0.2 import appears, the fallback is the feature
design above, with the unification trap documented in `docs/fyx/sdk-reference.md`.

**How the API splits.**

- The crate root re-exports the 0.2 types and API, as today. `ui_ext_sdk::v0_1` has the 0.1 API
  for 0.1 guests.
- The version-independent code (`TreeBuilder`, `IdAllocator`, `SurfaceTree`, `PatchSet`, the
  row helpers) is written once. It is instantiated per version by an internal macro over that
  version's bindings types, so it is neither copied by hand nor made generic in the public API.
- Each version module brings its own `Extension` trait (0.2 adds `tick`), host-function wrappers,
  props builders and `host_model.rs` conversions into the shared `ui_ext` model. `ui_ext::TreeLimits`
  stays shared, which keeps integration-2 #8 fixed.
- `register_extension!(T, v0_1)` writes the `0.1.0` version section and exports the 0.1 world.
  The default arm writes `0.2.0` and also exports `tick` (`ui_ext_sdk.rs:211-214`).
- 0.2-only APIs exist only in the 0.2 module, so using them in a 0.1 guest fails to compile
  rather than silently doing nothing:
  - `Extension::tick`;
  - `request_tick`, `cancel_tick`, `add_surface`, `remove_surface`, `show_surface` and
    `hide_surface`;
  - `place_decoration` and `remove_decoration`;
  - `SurfaceTree::invalidate_rows`, `SurfaceTree::push` and `SurfaceTree::place`;
  - `Surface::modal`;
  - the builders for `tooltip`, `secret`, `selected` and `indent`.
- `push_update` returns `Result<(), HostError>` in 0.2 and `()` in 0.1.
- `list_row` takes an `indent` argument only in 0.2.
- Icons: 0.2 builders take the generated `Icon`; 0.1 builders keep `&str`.
- `SurfaceTree::mount` checks `max_nodes` (16384) in both versions, since the host accepts that
  for `mount` and `replace` (design.md, Summary).
- Crate version `0.1.0` → `0.2.0`.

**Porting a 0.1 extension to 0.2 can require new surface ids.** 0.2 rejects static surfaces whose
ids break `[a-z0-9._-]{1,64}` (design.md, E). Surface ids are part of panel keys and persisted
dock state, so a renamed id loses the saved size and position. Every Fyx demo already complies
(`counter`, `todos`, `gallery`, `gallery-status`).

## 5. VS Code sidecar (W8)

The sidecar speaks the `ui_ext` serde form over JSON-RPC. It has no custom section, so it
negotiates the version in `initialize`:

- `InitializeParams` (`sidecars/vscode-host/src/host.ts:115-122`) gets
  `uiApiVersions: string[]`, the versions the host accepts, for example `["0.2.0", "0.1.0"]`.
- `InitializeResult` gets `uiApiVersion: string`, the one the sidecar speaks. The host then
  treats the sidecar as a backend of that version (section 3).
- The sidecar ships with the host, so after the migration it speaks **0.2 only**. It fails
  `initialize` with `InvalidParams` if `0.2.0` is not offered. The 0.1 side channels
  `fyx/surfacesChanged` and `fyx/surfaceClosed` are removed rather than kept for a 0.1 mode
  nobody runs.

JSON method mapping (the WIT import or export, and its RPC form):

| WIT | Direction | JSON-RPC |
|---|---|---|
| `tick` (export) | host → sidecar | request `tick {surface-id}` → `Update` |
| `request-tick`, `cancel-tick` | sidecar → host | requests `request-tick {surface-id, delay-ms}`, `cancel-tick {surface-id}` |
| `add-surface`, `remove-surface`, `show-surface`, `hide-surface` | sidecar → host | requests with the same names; replace `fyx/surfacesChanged` |
| `push-update` | sidecar → host | already a request; its error gets `data.code` = the `host-error-code` name |
| `ui-event::dismissed` | host → sidecar | inside `handle-event`; replaces `fyx/surfaceClosed` |

On the host side, effects from the sidecar arrive asynchronously, not inside a call. The ordering
rule from design A holds per connection: the host applies sidecar requests in arrival order on one
queue, and a `handle-event` reply is applied after every request the sidecar sent before it.
JSON-RPC over one pipe already preserves that order.

Files:
- `src/schema.ts`: every type change, with `Icon` as a string-literal union.
- `src/host.ts`: the handshake, the new handlers, and removing `fyx/*`.
- `src/mock-host.ts`: the new rules (id format, 32-surface cap, capability per kind, the
  gesture rule for `show-surface` on a modal, the tick floor) and `uiApiVersions`.
- `src/ui/{surfaces,quickpick,treeview,output,statusbar}.ts`: design C–K.
- `test/*.test.ts`.
- `docs/vscode-coverage.md`: the schema-gap table becomes "closed in 0.2", and `ignoreFocusOut`
  becomes a partial entry.

## 6. Test plan

Every new rule gets a test at the lowest layer that owns it. GPUI tests use executor timers
(`cx.background_executor.timer`, `cx.executor().advance_clock`), never `smol::Timer`
(`CLAUDE.md`, "Timers in tests").

**`ui_ext` (`crates/ui_ext/src/tree.rs` tests)**
- `invalidate-rows`:
  - on a list and on a table;
  - on a non-list node: `NotAList`, whole update rejected;
  - on an unknown id;
  - with a range past `row-count` (clamped);
  - two in one update (hull).
- `Touched`:
  - a `set-props` that changes only `selected` records no row invalidation;
  - one that changes `row-count`, `row-height` or `columns` records `All`.
- Serde round trip of every new type. The JSON fixtures are shared with the sidecar's
  `schema.test.ts`, so both sides are checked against the same bytes.

**Host (`crates/extension_host/src/wasm_host/fyx_ui/tests.rs`, probe fixture built for 0.1 and 0.2)**
- Version gating:
  - the 0.1 probe loads as 0.1 and the 0.2 probe as 0.2;
  - 0.3.0, 0.1.1 and 1.0.0 sections are rejected;
  - with the section renamed, the import fallback finds each version. This is the existing
    `test_version_falls_back_to_imports` run on both builds, and it closes integration-1 F2 for
    0.2;
  - on Stable/Preview a 0.2 component is rejected while staged, and accepted on Dev/Nightly.
- A 0.1 regression: every existing test passes unchanged against the 0.1 probe. That includes
  `test_push_flood_traps_and_disables_extension`, which pins the trap for 0.1.
- Effect ordering (0.2 probe):
  - push A, `remove-surface(A)`, push A → `unknown-surface` on the second push, and the effect
    list is `[Push, Remove]`;
  - `add-surface(B)`, push B → OK;
  - effects of a call that returns `Err` arrive in the `ReplyError`.
- `push-update` results:
  - the 65th push returns `limit-exceeded` and the extension stays enabled;
  - a `patches` update with 2049 patches and a `replace` with 16385 nodes return
    `limit-exceeded`.
- `request-tick`: `unknown-surface`; a delay above 24 h → `invalid-argument`; the effect records
  the delay clamped to 100 ms.
- `show-surface`:
  - a modal from `list-rows` or a tick without a recent user event → `not-permitted`;
  - from `handle-event(click)`, `row-click` or `input-submitted` → OK, also from a tick
    within 5 s after it;
  - after only `input-changed`, `row-selected` or `dismissed` → `not-permitted`;
  - a second show in the same activation window → `not-permitted`.
- `add-surface`:
  - a bad id → `invalid-argument`;
  - a missing capability → `not-permitted`;
  - the 33rd surface → `limit-exceeded`.
- A blocking `tick` (sleeps 1 s) is cut off by the budget and disables the extension, like the
  existing blocked-call test.

**Renderer (`crates/ui_ext_host/src/tests.rs` with the fake backend; `wasm_tests.rs` with the probe)**
- Ticks:
  - delivered after the delay (advance the clock);
  - re-requested every time, so periodic;
  - the floor clamps 10 ms to 100 ms;
  - deferred while the panel is hidden, then delivered at once on show;
  - 1 s minimum while no window is active;
  - the token bucket delays the 21st tick in a second;
  - the CPU-share throttle (the fake backend advances the clock inside `tick`);
  - stop after 3 errors, and resume after a click;
  - no tick while `Mounting`; dropped on `Failed` and on modal close;
  - a tick queued behind a pending click is sent after it;
  - a tick reply does not reset `remounts_without_input`.
- Modals:
  - show mounts and focuses the first input;
  - Escape → `dismissed(escape)`, the tree is dropped, and a reply other than `unchanged` is
    ignored;
  - a click outside → `focus-lost`;
  - opening a second extension modal while one is shown → the first gets `replaced` **and the
    second opens** (the `toggle_modal` same-type trap, design C);
  - `hide-surface` → no event;
  - showing it again → a fresh `mount`;
  - closing the window → `window-closed`.
- Selection:
  - Down moves `selected` locally and sends `row-selected`;
  - holding Down coalesces events in the queue;
  - Enter in the input sends `input-submitted`;
  - the extension setting `selected` scrolls the row into view and fetches no rows.
- `invalidate-rows`: only the visible part of the range is fetched, in one call; old rows stay
  drawn until the replacements arrive.
- Runtime surfaces:
  - `add-surface(panel)` adds a panel to two open workspaces and to one opened later;
  - `remove-surface` closes it in all of them, drops queued events, and discards an in-flight
    reply;
  - removing a status item uses the new `remove_item_by_id` hook;
  - re-adding the same id mounts again.
- A 0.1 fake backend keeps the full refetch on any list `set-props`, and never sees
  `row-selected` or `dismissed`.
- Icons: a unit test that every `Icon` maps to a non-`Blocks` `IconName` except `blocks`,
  `spinner` and `blank`. The `match` itself is exhaustive at compile time.
- Secret input: the editor is masked, and dropped-event debug logs contain no value (capture the
  logs with `env_logger` in the test).

**SDK (`crates/ui_ext_sdk`)**
- `SurfaceTree::push`: on `err`, the next `take_update` is a `Replace` of the full mirror, and it
  applies cleanly to `ui_ext::Tree` (the dev-dependency test from integration-1, "Duplication").
- `take_host_effects` records ticks and surface changes in order.
- `api_version_matches_the_wit_package` for each version: the version section bytes of
  `register_extension!(T)` and `register_extension!(T, v0_1)` match the `package` line of their
  WIT directory.
- A 0.1 build of a demo (`register_extension!(T, v0_1)`): `wasm-tools component wit` shows only
  `@0.1.0` `fyx:ui` interfaces and no `tick` export (the check from section 4).
- `cargo check -p ui_ext_sdk --target wasm32-wasip2`. The probe builds for both versions inside
  the workspace.

**Sidecar (`sidecars/vscode-host/test`)**
- Version negotiation: `initialize` without `0.2.0` fails.
- QuickPick:
  - `show()` sends `show-surface`;
  - Escape → `onDidHide` once;
  - `hide()` → `onDidHide` once, with no `dismissed` round trip;
  - `row-selected` updates `activeItems`.
- InputBox `password: true` → `secret`, and no `NotImplementedInFyx`.
- TreeView: `indent` is sent instead of padding, and a depth of 300 no longer fails the reply
  (W8-8); `reveal` sets `selected`.
- The mock host enforces the new caps and error codes.

**Manual, in the running editor** (add to `docs/fyx/manual-tests.md`)
1. Clock status item (counter demo):
   - ticks once a second;
   - slows to 1 s or more when Fyx is in the background;
   - keeps ticking while its panel is closed, because it is a status item;
   - the counter panel's own ticks pause while the dock is closed.
2. Picker modal from a status-item click, built from an input and a list:
   - typing filters;
   - Up and Down move the highlight;
   - Enter accepts;
   - Escape closes and logs `dismissed(escape)`;
   - a click outside closes it with `focus-lost`;
   - opening the command palette while it is shown closes it with `replaced`;
   - focus returns to the editor afterwards.
3. A runtime panel: add and remove it from a button. It appears and disappears in two open
   windows.
4. A secret input shows dots.
5. A tree with indent shows clean indentation and no vertical padding.
6. A button tooltip appears on hover.
7. Load the 0.1 counter build: it behaves exactly as before.

## 7. Order of work

| Step | Owner | Depends on | Gate |
|---|---|---|---|
| 1. Move 0.1 WIT to `since_v0.1.0`, update paths | W0 | — | `script/gate.sh` green, no behaviour change |
| 2. Add `since_v0.2.0`; `ui_ext` types and `Touched` | W0, then W2 | 1 | `cargo test -p ui_ext` |
| 3. Host bindings split, dispatch, effects, 0.2 imports, staging by channel | W2 | 2 | host tests on both probe builds |
| 4. SDK version modules, 0.2 APIs | W4 | 2 | SDK tests; the 0.1 build check from section 4; demos still build on 0.2 |
| 5. Renderer: ticks, modals, runtime surfaces, rows, icons, fields; status-bar hook | W3 (hook reviewed by W0) | 3 | renderer tests, manual 1–7 |
| 6. Demos: clock, picker, `invalidate-rows` | W4 | 4, 5 | manual 1–2 |
| 7. Sidecar | W8 | 2 (schema), 5 (to run for real) | `npm test` with the mock host |
| 8. Docs, coverage report, close `tasks/wit-0.2.md`; declare 0.2 frozen and drop the channel split | W7, W0 | all | judge review |

Steps 3 and 4 can run in parallel after step 2. Step 7 can start against the mock host as soon as
the schema from step 2 is merged.
