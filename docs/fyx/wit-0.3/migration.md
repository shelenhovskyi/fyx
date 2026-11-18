# `fyx:ui` 0.2 → 0.3: coexistence, gating, SDK, test plan

Companion to [`design.md`](design.md) and [`fyx-ui-0.3.wit`](fyx-ui-0.3.wit). Nothing here has
been implemented. It extends [`wit-0.2/migration.md`](../wit-0.2/migration.md), which this file
assumes has landed: the per-version WIT directories, `UiBindings`, the `HostEffect` list, the
channel staging and the SDK's per-version modules. Paths are relative to the repository root, at
`integration` `1ec364b`, with `crates/ui_ext_wasm` as on `claude/fyx-w3-renderer-e20d2b`
`be53175`.

**Goal.** A Fyx build that ships 0.3 loads 0.1, 0.2 and 0.3 components side by side and treats each
exactly as its world promises. A 0.2 component never sees a picker, a `handle-io` call, a consent
prompt or a `document-context`; a 0.1 component keeps 0.1 behaviour (0.2 migration, §3). No T2
capability exists for anything older than 0.3.

## 1. WIT layout (W0)

```
crates/ui_ext/wit/
  since_v0.1.0/fyx-ui.wit   # frozen
  since_v0.2.0/fyx-ui.wit   # frozen when 0.2 is declared frozen
  since_v0.3.0/fyx-ui.wit   # docs/fyx/wit-0.3/fyx-ui-0.3.wit once decided, minus its header comment
```

The 0.3 file is self-contained (Zed's copy-forward rule, 0.2 migration §1): it repeats `types`,
`host` and the 0.2 half of `decorations`. When 0.2 section M lands, its interface is copied into
both 0.2 and 0.3 in the same commit. `crates/ui_ext/src/ui_ext.rs` gets `WIT_DIR_V0_3_0`.

A check in `gate.sh` (W0): the parts of the 0.3 file marked as copied from 0.2 are byte-identical to
the 0.2 file, except the three types design §0.1 lists. It catches a fix applied to one version
only.

## 2. Host bindings and dispatch (W2c)

```
crates/ui_ext_wasm/src/
  ui_ext_wasm.rs        # runtime, budgets, UiState, UiExtension, Reply/HostEffect, dispatch
  since_v0_1_0.rs       # (0.2 migration)
  since_v0_2_0.rs       # (0.2 migration)
  since_v0_3_0.rs       # bindgen! for 0.3 + conversions + 0.3 import impls
  tests.rs, test_support.rs
```

**Dispatch.**

```rust
enum UiBindings {
    V0_3_0(since_v0_3_0::UiExtension),
    V0_2_0(since_v0_2_0::UiExtension),
    V0_1_0(since_v0_1_0::UiExtension),
}
```

New methods: `call_handle_io`, `call_handle_command`, `call_provide_decorations`,
`call_provide_hover`, `call_provide_code_lenses`, `call_provide_virtual_document`,
`call_picker_source_event`. On `V0_2_0` and `V0_1_0` they are unreachable, because no older
component can register a provider, start I/O, declare a command or own a picker; they return
`Ok` of an empty value instead of panicking, as 0.2's `call_tick` does on 0.1.

**Version gating.** `MAX_VERSION = 0.3.0`; the explicit match gains `(0, 3, 0) => V0_3_0`. The
channel split moves up one version:

```rust
pub fn api_version_range(channel: ReleaseChannel) -> RangeInclusive<Version> {
    let max = match channel {
        ReleaseChannel::Dev | ReleaseChannel::Nightly => Version::new(0, 3, 0),
        ReleaseChannel::Stable | ReleaseChannel::Preview => NEWEST_FROZEN, // 0.2.0 once frozen
    };
    MIN_VERSION..=max
}
```

`MIN_VERSION` stays 0.1.0 for every 0.3 build (spec decision 29; design Q9 for what happens at
1.0).

**Manifest parsing** (implemented: W2c, d65c68a2fc; design §0.6, "What the host checks
today"). `UiManifest` is `deny_unknown_fields`. A 0.3
manifest has new tables (`network`, `processes`, `process`, `commands`, `picker_sources`). Read it
in two steps: first only `api_version` (a lenient struct), check the version, then the full
manifest for that version. An older host then says "targets fyx:ui 0.3.0, this build supports
0.1.0..=0.2.0" instead of "unknown field `network`". This is a change to the 0.2 host too, so that
0.2 builds give the right error for 0.3 extensions; it must ship with 0.2.

Per version, the manifest accepts:

| Field | 0.1 | 0.2 | 0.3 |
|---|---|---|---|
| `ui:panel`, `ui:status-bar` | yes | yes | yes |
| `ui:modal`, `ui:editor-decorations` | accepted, unused (0.2 migration) | yes | yes |
| `ui:picker`, `ui:hover`, `ui:code-lens`, `ui:virtual-documents`, `ui:commands` | error | error | yes |
| `workspace:read-buffers`, `workspace:diagnostics` (T1) | error | error | yes (design Q3) |
| `network:fetch`, `process:spawn`, `workspace:edit`, `agent:observe` (T2) | error | error | yes |
| `[fyx_ui.network]`, `[[fyx_ui.processes]]`, `[fyx_ui.process]`, `[[fyx_ui.commands]]`, `[[fyx_ui.picker_sources]]` | error | error | yes |

An error, not a warning, for capabilities an older world cannot use: otherwise the install dialog
would ask consent for something the component cannot do.

`[[fyx_ui.commands]]` (≤ 64, title ≤ 128 bytes) and `[[fyx_ui.picker_sources]]` (≤ 64,
`picker = "<extension>/<surface>"`) are accepted and checked in 0.3 manifests. No host feature
uses them yet.

**Linkers.** A third `Linker<UiState>` with WASI and `since_v0_3_0::UiExtension::add_to_linker`.
Every 0.3 interface is linked for every 0.3 component and checks its capability per call, as 0.2
does for `decorations` (0.2 design §L), so a component that imports `network` still instantiates
where the capability is not granted and gets `not-permitted` or `denied` it can handle.

**WASI context.** For all versions, built with `allow_tcp(false)`, `allow_udp(false)` and
`allow_ip_name_lookup(false)` stated explicitly (design §8). This is a behaviour-preserving change
for 0.1 and 0.2, verified by a test.

**Store limits.** Design Q7: `UI_MEMORY_LIMIT` becomes per version, 256 MiB for 0.3 if the owner
agrees, 32 MiB for 0.1 and 0.2.

**Shared state.** `UiState` gains, all version-independent:
- the counters of design §8–§10 (fetches in flight and the per-minute bucket, processes, edits);
- the permission mirror (`[PermissionState; 4]`), refreshed by the host before each call so the
  sync `permissions.query` needs no main-thread access;
- live namespaces, providers, and the next host-chosen ids;
- the transient-activation timestamp of 0.2, now also set by `handle_command` and by an accepted
  picker event (design §0.4).

**Conversions.** `ui_ext` stays the superset:
- 0.2 into the model: `decoration-location.path` → `DecorationTarget::Path`; no other new field
  has a 0.2 counterpart.
- The model into 0.2: `UiEvent::Orphaned` fails `TryFrom` and is dropped with a debug log (the
  host never produces it for a 0.2 decoration: 0.2 has no `orphaned`); `SurfaceKind::Picker`
  cannot come from a 0.2 component.
- 0.3 maps one to one.

## 3. Renderer: behaviour by version (W3, W6)

`ApiRules` (0.2 migration §3) gains fields derived from the version once:

| Behaviour | 0.1 | 0.2 | 0.3 |
|---|---|---|---|
| Block decorations | none | §L, path targets | §L, path or document targets, `orphaned` |
| Styled decorations, namespaces, pull | — | — | design §1 |
| Pickers (host-drawn) | — | — (0.2 pickers are modal trees) | design §2 |
| Hover, code lens, virtual documents, commands | — | — | design §3–§5 |
| Document context in requests | — | — | design §6 |
| `handle-io`, T2 operations, consent prompts | — | — | design §0.3, §7–§11 |
| Gesture sources | — | click, row-click, input-submitted | the same plus commands and picker accepts |

Hot reload (P1.3) gains steps for 0.3 instances, in this order: cancel fetches, kill the process
set, drop pending edits, stop agent observation, clear namespaces and providers, close pickers as
0.2 closes modals, keep virtual document tabs with their last text; then the new instance mounts
and every open virtual document is pulled again. No `dismissed` is sent for the closed modals and
pickers: the instance that showed them is gone, and the new one never knew them. Spec decision 7
says "`dismissed(reload)`", but 0.2's `dismiss-reason` has no such case; the platform spec should
drop the phrase rather than 0.3 add a case nobody can receive.

## 4. SDK (W4)

`ui_ext_sdk` adds a `bindings::v0_3` module and makes 0.3 the default world of
`register_extension!(T)`; `register_extension!(T, v0_2)` and `(T, v0_1)` keep the older ones.
The no-leak check of 0.2 migration §4 runs for 0.2 as well: a `v0_2` build shows only `@0.2.0`
interfaces and none of the 0.3 exports.

- `Extension` (0.3 module) gains default methods for every new export: `handle_io`,
  `handle_command`, `provide_decorations`, `provide_hover`, `provide_code_lenses`,
  `provide_virtual_document`, `picker_source_event`. Defaults return empty results, so an
  extension implements only what it registers.
- Wrappers for every import, returning `Result<_, HostError>` or the operation id, with typed
  handles (`FetchId`, `ProcessId`, `EditId`) so outcomes can be matched without casting.
- A small `IoRouter` helper that maps `IoEvent`s back to per-operation callbacks the extension
  registered when it started them. It is optional: the WIT is one variant, and some authors will
  match it directly.
- `PickerSession` (design §2), decoration builders (§1), `WorkspaceEditBuilder` that turns
  character positions into UTF-8 columns from the `document-context` text (§10), and
  `HttpRequest::get(url)` style builders.
- `take_host_effects()` off wasm records every new effect, so extension unit tests see fetches,
  spawns and edits without running them; `fyx_sdk::testing` (P1.8) can feed `IoEvent`s back.
- Crate version `0.3.0`.

**Porting a 0.2 extension to 0.3** needs no change beyond `decoration-location.path` becoming
`target: path(…)` (the SDK builder keeps the 0.2 signature as a shorthand). A 0.2 picker built
from a modal with an input and a list keeps working in 0.3; moving it to `surface-kind::picker`
removes its `input-changed` handling and its `list-rows`.

## 5. TypeScript SDK and the VS Code sidecar

- `@fyx/sdk` (P2.5) generates 0.3 bindings with `jco` and wraps the same surface; the SDK parity
  test (spec decision 3) fails until every 0.3 import and export has a wrapper in both SDKs.
- The VS Code sidecar (W8) is a process and has its own `fetch` and `child_process` under Node's
  permission model (`vscode-coverage.md`, "Sandboxing with Node's permission model"). It does not
  get `network:fetch` or `process:spawn` through this world in 0.3. It may adopt §1–§5: `setDecorations`
  maps onto styled decorations (with `accent` for raw colours it cannot keep),
  `registerHoverProvider` and `registerCodeLensProvider` onto §3, QuickPick onto §2, commands onto
  §5. That is W8's choice and not required for 0.3.

## 6. Test plan

The design lists tests per section; this is where they live. GPUI tests use executor timers
(`cx.background_executor.timer`, `cx.executor().advance_clock`), never `smol::Timer`.

**`ui_ext` (`crates/ui_ext/src/tree.rs`, new `io.rs` tests)**
- Serde round trip of every new type, with JSON fixtures shared with the TS SDK.
- `DecorationTarget` from a 0.2 path equals the 0.3 `path(...)`.

**Host (`crates/ui_ext_wasm/src/tests.rs`, probe built for 0.1, 0.2 and 0.3)**
- Version gating: the three probes load as their versions; `0.4.0`, `0.3.1` and `1.0.0` are
  rejected; Stable/Preview reject 0.3 while it is staged; a 0.2 host build (feature-gated test
  path) reports the version for a 0.3 manifest, not an unknown field.
- Manifest: each row of the §2 table; public-suffix domain patterns, forbidden environment names
  and host-namespace command titles fail at load.
- Every 0.2 test passes unchanged against the 0.2 probe, and every 0.1 test against the 0.1 probe.
- Effects: `fetch`, `spawn`, `apply-edit` return ids and record effects in call order with pushes;
  a trap after them drops them; the ninth fetch, fifth process and fifth edit in flight return
  `limit-exceeded`.
- `permissions.query` reflects the mirror; `request` without a gesture → `not-permitted`; with one
  → effect recorded.
- `handle-command` grants the activation: `show-surface` on a picker inside it succeeds, after 5 s
  it fails.
- WASI: a probe that opens a TCP socket and one that resolves a name both fail.
- A `handle-io` that blocks past the budget disables the extension, as every call does.

**Renderer (`crates/ui_ext_host/src/tests.rs` with the fake backend, `wasm_tests.rs` with the probe,
`decoration_tests.rs`)**
- The per-section tests of design §1–§5 and §7–§11.
- Backpressure: with 256 queued `handle-io` calls, a process's pipe is not read and memory stays
  flat; draining the queue resumes it.
- Reload: a 0.3 extension with an open picker, a fetch in flight, a process tree, a pending edit,
  an observation and an open virtual document reloads; afterwards no process survives, no old
  outcome arrives, the picker is closed, the virtual document shows the new text (spec failure 3,
  extended).
- A 0.2 fake backend never sees `handle-io`, providers, pickers or prompts, and a 0.2 decoration
  never gets `orphaned`.

**Network, process and edit integration (`ui_ext_host`, behind a test feature that injects a test
CA and resolver)**
- Design §8, §9 and §10 test lists, run on Linux, macOS and Windows CI; the job-object and
  `PATHEXT` cases are Windows-only, `PR_SET_PDEATHSIG` Linux-only.

**SDK (`crates/ui_ext_sdk`)**
- `api_version_matches_the_wit_package` for 0.3; the `v0_2` no-leak check.
- `take_host_effects` records the new effects in order; `IoRouter` delivers each outcome to the
  callback of its id and drops unknown ids.
- `WorkspaceEditBuilder` produces UTF-8 columns for text with multi-byte characters.
- `cargo check -p ui_ext_sdk --target wasm32-wasip2`.

**Manual, in the running editor** (add to `docs/fyx/manual-tests.md`)
1. TODO highlighter: TODOs styled and signed in the gutter while typing; scrolling a 20000-line
   file stays smooth; deleting a line with a block decoration logs `orphaned`.
2. A shared picker: two demo extensions' items in one picker; typing filters instantly; Tab marks;
   alt-Enter runs the secondary action of the right extension.
3. Hover and code lens on a demo language: the hover block under the language server's; a lens click
   opens a modal (gesture).
4. A virtual document opens read-only with highlighting, refreshes on invalidate, survives a hot
   reload.
5. A command from the palette and from a key binding in `keymap.json`.
6. First fetch: the prompt appears once, Allow ignores clicks for 1 s, Don't allow persists, revoke
   on the extensions page cancels a slow request.
7. `git status` panel: a planted `git` in the worktree is not run; reload kills a long `git log`;
   an untrusted worktree shows "restricted" without a prompt.
8. A multi-file edit: the notification appears without taking focus; Undo reverts all files;
   typing just before a single-file edit is not undone with it.
9. `agent:observe` demo panel lists open reviews while an agent edits; it has no way to accept;
   its decorations do not appear in the review pane.
10. Load the 0.2 and 0.1 counter builds: they behave exactly as before.

## 7. Order of work

| Step | Owner | Depends on | Gate |
|---|---|---|---|
| 1. Owner answers design.md's open questions; W7 reviews the design (plan 4.1) | owner, W7 | — | review file in `tasks/reviews/` |
| 2. Lenient manifest version parse, shipped with 0.2 | W2c | 0.2 landed | host tests |
| 3. Add `since_v0.3.0`; `ui_ext` types and `io.rs` | W0, then W2c | 1 | `cargo test -p ui_ext` |
| 4. Bindings, dispatch, effects, permission mirror, WASI hardening, staging | W2c | 3 | host tests on three probe builds |
| 5. SDK 0.3 module, wrappers, `IoRouter`, builders | W4 | 3 | SDK tests; the `v0_2` no-leak check |
| 6. Decoration extras and the editor hooks (plan 4.2) | W6 | 4 | decoration tests, manual 1 |
| 7. Picker, hover and code lens, virtual documents, commands, palette and code lens hooks (plan 4.3, 4.4) | W3 | 4 | renderer tests, manual 2–5 |
| 8. T2 consent, held updates (plan 4.5) | W3 | 4, P1.5 | manual 6 |
| 9. `network:fetch` (plan 4.6) | W2c | 8 | §8 tests on three OSes |
| 10. `process:spawn` (plan 4.7) | W2c | 8 | §9 tests on three OSes, manual 7 |
| 11. `workspace:edit`, `agent:observe` (plan 4.8) | W2c | 8 | §10, §11 tests, manual 8–9 |
| 12. TS SDK parity, docs, coverage report; declare 0.3 frozen and move the channel split | W7, W0, `ts-sdk` | all | judge pass (plan 4.9); then the 4-week 1.0 clock (spec decision 30) |

Steps 5, 6 and 7 can run in parallel after step 4, within the limit of two compiling streams
(platform plan, "Parallelism"): W6 + W3 first, then W2c for steps 9–11. Steps 9–11 can start
against the fake backend before step 8, but must not merge before it: no T2 operation may run
without the broker.
