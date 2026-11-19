# W2c progress archive (older history; the current handoff is in ../W2c.md)

# W2c progress: host side of `fyx:ui@0.2.0` (platform plan 2.2)

Branch `claude/fyx-w2c-host-0-2-cbde2c`, on `integration` (with p2-wit) plus W3's
`claude/fyx-w3-renderer-e20d2b` at `4ca0073db1` (cache, `CallBudget`, lifted-text cap, broker,
Inspector diagnostics), as the coordinator asked.

## Handoff (2026-11-19, fourth context clear; superseded)

- **Branch:** claude/fyx-w2c-host-0-2-cbde2c, pushed at 54df51e48b.
- **At W0:** 0e7d6a00bb (5.5 step 3) and 54df51e48b (the 0.4 import side) go in one gate with
  W4's harness fix. The harness needs the two `V0_4_0` arms, `limits_match` (MAX 0.4) and
  `net::IoEvent::Terminal(_) => None` in `fyx_ext_harness/src/v0_3.rs` `io_event`.
- **Done in 54df51e48b:** `fyx_ui_v0_4.rs` (bindgen, `terminals` Host); the 0.3/0.4 shared
  imports in `t2_shared.rs` (`include!`d); `terminals.rs` (checks; a sink to
  `IoEvent::Terminal`; close_all on drop and on revoking create/env); fixture `probe_v0_4`;
  `tests_v0_4.rs`. `terminal` nodes are refused until 5.7; T3 calls answer `not-permitted`.
- **Next:**
  1. 5.6, the broker. **W3 OK'd it with changes: follow `W2c-broker-api.md`**, which overrides
     the summary below (W3 = local_31b10de5-…). Old summary:
     - in ui_ext_wasm: `Tier::T3`; `ConsentItem` {Capability, Program{capability,name},
       Listener, DiscoveryFile, EnvAllNew} with `key()`; `ItemLine`; `consent_items(&UiManifest)`
       and `consent_diff(old, new)` (§C.5; extend the existing `UiManifest::consent_items`);
       `CapabilityCheck::item_state` and `t3_request` as default methods; `T3Prompt` and
       `T3Answer` {Once, UntilClosed, Deny}; `journal::JournalStore` (`LoadOptions::journal`,
       `read`/`clear`, 2000 entries / 30 days, never terminal content);
     - in ui_ext_host `permissions.rs`/`consent.rs`: `ConsentRecord.items` (serde default);
       `install(ext, record, ticked)`; `set_item`; `item_state`; `items`;
       `set_always_allow(ext, cap, sha)`, reset when the sha changes; `set_t3_prompter`;
       `NeedsApproval { diff }`.
  2. Then wire T3 in `fyx_ui_v0_4.rs` `request_read`/`request_write`. Allow `terminal:read` and
     `terminal:write` in `world_capabilities` V0_4 and `permissions.rs` T3. The TerminalHost
     trait has no read/write yet: agree it with W1.
  3. `reason` for `[[fyx_ui.processes]]` and `[fyx_ui.network]`, 0.4-only. `fyx_ext/src/manifest.rs`
     (W4) also parses `processes`. Plan: a `deserialize_with` that strips `reason` before
     `ProcessSpec` (deny_unknown_fields), with the reasons kept beside it; refused below 0.4.
  0. First, approvals runtime for W3 (see the end of `W2c-broker-api.md`).
  4. Done: fyx_registry tiers for 0.4 (294ce6935c, which pairs with W10 9bdf8903a4).
- **Gotchas:** for ui_ext_wasm tests, patch `fyx_ext_harness` locally with the arms above, then
  `git checkout -- crates/fyx_ext_harness`. Targets: `E:/fyx-targets/w2c` (ui_ext_wasm,
  ui_ext_host) and `E:/fyx-targets/w2c-light` (ui_ext, fyx_registry). Bash heredocs with
  apostrophes can break: write scripts with the Write tool into the scratchpad.
  `cargo fmt` skips `t2_shared.rs` (it is `include!`d), so run `rustfmt --edition 2024` on it.

## Done (2026-11-18)

All in `crates/ui_ext_wasm`; `ui_ext_host` only gained `..LoadOptions::default()` in one literal
(`registry.rs`) and still builds against the 0.1 API.

- **Version dispatch** (`dispatch.rs`): `UiBindings { V0_2, V0_1 }`, one linker per world, the
  world picked from the `fyx:ui-api-version` section (or the imports) before instantiation.
  `MAX_VERSION = 0.2.0`, `MAX_FROZEN_VERSION = 0.1.0`; a 0.2 component loads only with
  `LoadOptions::allow_staged` (default: debug builds or `FYX_UI_DEV_EXTENSIONS` set;
  `staged_api_allowed(dev_or_nightly, dev_extension)` for callers that know the channel).
- **Capabilities, two checks**: declared (`LoadOptions::capabilities`, from the manifest in
  `load_from_dir_with_options`; `None` declares every UI capability) via `ui_ext::LiveIds`, and the
  broker via `UiState::check_capability` at every gated call (`add-surface` by kind,
  `show-surface` by kind, both decoration imports), so a revocation applies to the next call and
  the Inspector sees each check. A denial is `not-permitted`. `ui:modal` and
  `ui:editor-decorations` are T0 in `permissions.rs` already.
- 0.2 `push-update`, `add-surface`, `place-decoration` and `log` count toward
  `MAX_LIFTED_TEXT_PER_CALL` (a trap, as in 0.1); `log` is shared (`UiState::write_log`).
  `mount_v0_2` gets the first-mount budget; every call is in `diagnostics()`.
- **0.2 imports** (`fyx_ui_v0_2.rs`, checks in `effects.rs`), each checked at the call against
  `ui_ext::LiveIds` and recorded as a `HostEffect`, in call order:
  - `push-update -> result`: unknown id, 64 pushes per call, `check_caps` (2048 patches, 16384
    nodes per `replace`), 32768 items per call; all `limit-exceeded`, never a trap.
  - `request-tick` (floor 100 ms, > 24 h `invalid-argument`, a modal that is not shown
    `invalid-argument`, decorations refused), `cancel-tick`.
  - `add-surface` (id rule, duplicate, capability, cap 32), `remove-surface`.
  - `show-surface` / `hide-surface`: panels without a gesture; a modal needs a transient
    activation from `handle-event` `click`, `row-click` or `input-submitted`, valid during that
    call and 5 s after it, used up by one show. Showing a shown modal or hiding a closed one does
    nothing. Status items and decorations: `invalid-argument`.
  - `dismissed`: the modal is no longer shown, and a reply other than `unchanged` is dropped.
  - `place-decoration` / `remove-decoration`: `ui:editor-decorations`, id rule, no clash with a
    surface id, the path rule, 16384 nodes, 512 live, 512 decoration effects per call.
  - Frozen icons map one to one (`IconRef::Frozen`); tooltip, secret, selected and indent are
    passed through (the renderer truncates and clamps with `ui_ext::v0_2` helpers).
- **Tick rules for the renderer** (`ticks.rs`, no timers): `PendingTick` (floor, 1 s in the
  background, deferred while hidden then due at once), `TickBudget` (20/s token bucket, 10 % CPU
  share over 10 s throttles to 1 s), `TickErrors` (stop after 3, resume on a user event).
- **Tests**: `fixtures/probe_v0_2` (wit-bindgen on the 0.2 WIT, since the SDK is 0.1 until plan
  2.4) runs a script of host calls and reports each result; `tests_v0_2.rs` covers every new
  import. Every 0.1 test passes unchanged except two version-gating assertions (0.2 is now known).

## For W3 (plan 2.3): the API to move to

`UiExtension` keeps `mount`, `handle_event`, `list_rows`, `surfaces`, `Reply` and `ReplyError` as
0.1-typed shims (lossless for 0.1 components; for 0.2 ones they drop what 0.1 cannot express).
The 0.2-typed API, for both versions:

- `load_with_options(id, bytes, LoadOptions { cache, capability_check, capabilities,
  allow_staged }, cx)`, `load_from_dir_with_options(dir, options, fs, cx)` (capabilities from the
  manifest). Pass `allow_staged: Some(staged_api_allowed(dev_or_nightly, is_dev_extension))`.
- `ui_api_version() -> ui_ext::ApiVersion`, `capabilities()`, `surfaces_v0_2()`,
  `initial_effects()` (effects of instantiation and `surfaces()`: apply after the static surfaces).
- `mount_v0_2`, `handle_event_v0_2`, `list_rows_v0_2`, `tick`, all `-> Result<EffectReply<T>>`;
  on a rejected reply the error downcasts to `EffectReplyError { effects, message }`, whose effects
  still apply.
- `modal_closed(surface_id)`: call it when a `ShowSurface` effect is dropped (no workspace window),
  so the next `show-surface` is not a no-op.
- `HostEffect::{Push, RequestTick, CancelTick, AddSurface, RemoveSurface, ShowSurface,
  HideSurface, PlaceDecoration, RemoveDecoration}`.

A 0.1 component is never ticked (`tick` answers `unchanged` without a call) and never gets
`row-selected` or `dismissed` (dropped with a debug log).

## Left

- Section M (notifications, progress): not in the WIT yet.
- W3 (2.3): pass `allow_staged` from the release channel and dev-extension flag; rename
  `EffectReply`/`EffectReplyError` to `Reply`/`ReplyError` when the 0.1 shims go.

## Plan 4.10: images, host side (2026-11-18)

- **0.3 WIT staged** in `crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit` (copied from
  `claude/fyx-w10-docs-a5772b` 6d5f9e5462 with a STAGED DRAFT header), `ui_ext::WIT_DIR_V0_3_0`,
  `ApiVersion::V0_3_0`. `MAX_VERSION = 0.3.0`; 0.3 is staged like 0.2 (`allow_staged`).
- **`ui_ext::image`**: `ImageProps`, `ImageSource`, `ImageFit`, `ImageFormat`; every §12 limit as
  a constant (shared with the renderer's decoder, plan 4.11); `check_image_props` (alt 1–256 bytes,
  box 1–2048), `check_asset_path` (`assets/…`, no `..`, `\`, `:`, NUL, `.png`/`.svg`),
  `check_image_bytes` (size cap; PNG signature + IHDR read before any allocation, over 2048 × 2048
  refused; SVG must be UTF-8 with `<svg`, no `<!ENTITY`, no gzip), `check_image_placement` (panels
  only).
- **`ui_ext_wasm::images`**: `resolve_asset` / `read_asset` canonicalize inside the version
  folder, regular files of at most 2 MiB.
- **0.3 dispatch**: `UiBindings::V0_3` (`fyx_ui_v0_3.rs`); the 0.2 and 0.3 bindings share the
  `host` imports and the type conversions through macros in `conversions.rs`. Linked: `types`,
  `host`, `images`; every other `fyx:ui` 0.3 interface traps when called (`define_unbuilt_as_traps`; not
  wasmtime's `define_unknown_imports_as_traps`, which also shadowed newer-patch WASI imports),
  so a component that does not call them works.
- **`images` import**: `ui:images` declared and granted by the broker at each call, the byte
  checks, at most 32 live handles, handles never reused; `HostEffect::AddImage { handle, format,
  bytes }` / `RemoveImage { handle }`. `ui:images` and `ui:canvas` are T0.
- **`image` nodes** (coordinator decision 2026-11-18): `ui_ext::v0_2::Props::Image(ImageProps)`;
  `v0_2` is documented as the newest model, staged 0.3 additions included. The host checks image
  props when an update applies (`ApiState::check_images`): in `push-update` (`invalid-argument`)
  and in 0.3 replies to `mount`, `handle-event`, `tick` and `list-rows` (the guest's error, a
  resync): `check_image_props`, panels only (`check_image_placement`; decorations are refused
  too), and a `generated` handle `add-image` returned (a removed one is fine, design.md §12).
  The renderer draws the alt text in a muted box (`Renderer::image_placeholder`) until W3's 4.11;
  the 0.1 shims (host and harness) turn an image into its alt text.
- Tests: `fixtures/probe_v0_3` and `tests_v0_3.rs` (the bomb, 1 MiB + 1, entity SVG, junk, the
  32-handle cap, the capability, image nodes refused, traps), unit tests in `ui_ext::image`,
  `effects` and `images` (path fixtures, links out on Unix).

## Triage-3 (2026-11-18)

- Registry (`claude/fyx-w2c-registry`): every row; see that branch's commit.
- `fyx_guard` (`claude/fyx-w2c-guard`, on `fyx-w9-fixes`): every row but the shared real-path
  resolver (top-7 #1), which waits for W5's `fyx_guard::paths`.
- W2c rows, here:
  - ASYNC 5 done: the epoch callback counts executed epochs (`UiState::executed`), and
    `with_budget` counts only time a call waits on the host, not time it is ready but unpolled.
    `CallRecord::executed` is the same counter, for the renderer's tick CPU-share throttle
    (`ticks::TickBudget::record`). Pinned by `test_budget_ignores_time_spent_waiting_for_a_worker`,
    which fails on the old wall-clock budget.
  - UX F16 and PERF 6/12, agreed with W3 (2026-11-18):
    - F16: `input-props.label: option<string>` in WIT 0.2 and 0.3, and `InputProps.label` in
      the model. `label_text()` caps it at `MAX_LABEL_BYTES` (256). Buttons keep `tooltip` as
      their accessible name. The renderer side is W3's: an input's aria label is its label,
      else its placeholder; an icon-only button's is its tooltip, else the icon name.
    - PERF 6: `v0_2::Tree::apply_patches` applies in place with an undo log; `Touched` is
      unchanged.
    - PERF 12: `TreeNode` holds `Arc<Props>`, plus `props_arc()`, whose pointer changes exactly
      when the props are replaced.
    - The fixes in other crates were left to their owners: W3's ui_ext_host test literals, and
      W4's SDK `InputProps::new` and the harness wit→model conversion.

## §M notifications and progress, host side (2026-11-18)

- WIT: design.md M.6 applied to `since_v0.2.0` (replaces its TODO) and copied into the staged
  `since_v0.3.0`, as 0.3's own design says. Both parse; a dummy 0.2 component validates.
- Model: `ui_ext::v0_2::notifications` (types, caps as constants, `Notification::validate`,
  `ProgressOptions::validate`, `check_progress_update`: the registry's text rules) and three
  `v0_2::UiEvent` cases.
- Host: `notifications` linked for 0.2 and 0.3 (one macro); `ui:notifications` checked against the
  manifest and the broker; owner = a live surface or decoration (a panel for `panel` progress);
  caps 5 notify/60 s, 3 open, 4 per call, 4 running, 30 starts/60 s, 64 updates per call; refused
  notifications are logged with their title; `notification-action` is a gesture and frees its slot,
  `notification-closed` frees its slot, owner removal forgets both kinds; effects
  `Notify`, `DismissNotification`, `ProgressStart`, `ProgressUpdate`, `ProgressEnd`.
- Minimal fallout fixes in other streams' code: the SDK mirror and four demos (W4) and the harness
  (W4, notifications refused there for now) match the new events; the renderer (W3) logs and
  ignores the new effects until it draws them.

## Plan 4.6: `network:fetch`, host side (2026-11-18)

- **Where:** `crates/ui_ext_wasm/src/network.rs`, the engine and the rules. The import and the
  `handle-io` conversion are in `fyx_ui_v0_3.rs`. The start-after-call hook is in
  `UiExtension::call`. `UiExtension::{take_io_events, handle_io}` are in `dispatch.rs`.
- **No new `HostEffect` variants.** A fetch starts inside `ui_ext_wasm` after its call returns,
  and a trap drops it. Its outcome arrives on `take_io_events()`; the host passes each one to
  `handle_io()`, whose effects come back like any call's. W3's exhaustive effect matches are
  untouched.
- **Manifest:** `[fyx_ui.network] domains` (`UiManifest.network`). `network:fetch` is allowed
  only from 0.3 and only with domains. The table without the capability is refused, and so is a
  wildcard over a public suffix (`fyx_registry::is_public_suffix`, now public).
- **The import checks:**
  - the capability is declared; the broker's `CapabilityCheck` runs at every call (refused →
    `denied`);
  - the URL: https; http only to a declared `localhost`/`127.0.0.1` port; no user info; ≤ 8 KiB;
  - the domain list: `http_proxy::HostPattern`, so IDNA, case and the trailing dot are handled,
    and `*.` matches subdomains only. An undeclared host is `denied` (plan 4.6);
  - the headers: forbidden set plus `proxy-*` and `accept-encoding`, ≤ 64, ≤ 16 KiB;
  - the body ≤ 4 MiB, a timeout of 1–30000 ms, 8 in flight, 120 per minute.
- **The broker** (W3's plan 4.5 API, agreed 2026-11-18): `CapabilityCheck::{state, request,
  changes}` and `PermissionState { Granted, Denied, Ask, Unavailable }`.
  - At the import: `state()`. Ask or Granted is accepted; Denied or Unavailable is `denied`.
  - When the request is applied: in Ask, `request()` awaits the prompt.
  - Before delivery: `state()` again, and anything but Granted is `denied`.
  - `changes()`: a revocation aborts the requests in flight, which deliver `denied`, and every
    change goes out as `io-event::permission-changed`.
  - A broker without T2 (the defaults), or no broker at all, leaves T2 denied.
- **The engine:**
  - redirects followed by hand, ≤ 5, every hop checked (off-list, `http://`, or a sixth hop →
    `redirect-not-allowed`);
  - one resolution per hop, every address global or `host-not-allowed`, and reqwest pinned to
    the checked addresses;
  - `localhost` never goes to DNS; no environment proxy;
  - `accept-encoding: gzip` set by the host, gzip decoded, and the 16 MiB cap applied to the
    decoded stream;
  - response headers ≤ 64 KiB.
- **Delivery:** `handle-io`, with a bounded queue of 256 (backpressure). A capability revoked
  before delivery turns the outcome into `denied`. `cancel-fetch` → `cancelled`. Dropping the
  instance (reload, disable) aborts its requests, and none delivers.
- **Model:** `ui_ext::v0_2::HostErrorCode::Denied`. A 0.2 component sees it as `not-permitted`.
- **Tests:** 11 engine tests against a local HTTP server: the caps, a 17 MiB body by length,
  streamed, and as a gzip bomb; redirects; timeout; private answers; and pinning, where a
  rebinding resolver is called once and the connection goes to the checked address. Also
  7 end-to-end tests through `probe_v0_3`: the import, the broker, consent ask/allow/refuse,
  no broker, revocation before delivery and in flight (`permission-changed`), cancel, the
  in-flight cap, reload, trap, and the manifest rules.
- **Not done yet:**
  - Fyx's proxy setting;
  - brotli, zstd and deflate (any other `content-encoding` → `other`);
  - TLS currently uses reqwest's native roots only, without the webpki roots;
  - a local TLS test server (the pinning test uses a plain TCP server);
  - an Inspector line per request;
  - the user-agent carries no Fyx version.

## Plan 4.13: canvas host and limits (2026-11-18)

- **Model** (`crates/ui_ext/src/canvas.rs`):
  - `CanvasProps`, `DrawOp` (line, rect, rounded-rect, circle, path, text), `Paint`,
    `Stroke`, `Point`, `Rect`, `PathSegment`, `DataColor`, `CanvasFit`, `CanvasClick`;
  - numbers are `Num(f32)`, compared by bits so the model keeps `Eq`;
  - `Props::Canvas` and `UiEvent::CanvasClick` (whose target is the canvas).
- **Shared checks:**
  - `check_canvas_props`: sides 1–4096; description 1–1024 bytes; ≤ 16384 ops,
    65536 segments and 256 KiB of text; every number finite and within ±32768, with
    non-negative sizes and radii; strokes 0.5–16; opacity ≤ 100; text ops one line of
    ≤ 256 bytes; paths start with move-to, draw something, and have a fill or a stroke;
  - `check_canvas_placement`: panels only;
  - `canvas_text_len`: the description plus the text ops, which is what the host counts
    toward the lifted-text cap.
- **The tree** (`v0_2::Tree`): at most 16 canvases per surface, counted across inserts,
  set-props, replaces, removals and rollbacks; `from_row` refuses a canvas
  (`TreeError::{TooManyCanvases, CanvasInRow}`).
- **Host:**
  - the 0.3 conversion;
  - `check_images` also checks canvases: props and panels only, in pushes and in 0.3 replies;
  - `canvas-click` is a gesture;
  - the 0.1 shim shows the description;
  - a 0.2 component never gets `canvas-click`.
- **Probe:** case 8 (pushes: valid, NaN, no description, 16385 ops, status, modal), case 9 (a
  canvas reply), and a `canvas-click` report.
- **Not done here:** the renderer and the hit-test mapping (W3, 4.14), the SDK and the harness
  (W4, 4.15), and the bench.

## `ui_ext::network`, for the Apache harness (2026-11-18, W4's request, coordinator-approved)

- The pure part of `network:fetch` moved to `crates/ui_ext/src/network.rs`, behind the `ui_ext`
  feature `network`, so guests don't compile `url`. It holds:
  - the `CAPABILITY_NETWORK` and `MAX_*` constants;
  - `Method`, `FetchRequest`, `FetchResponse`, `FetchError`, `IoEvent` and `PermissionState`;
  - `DomainList::{parse, texts, allows}` and `check_request`;
  - `FetchCounters`, the in-flight and per-minute caps;
  - `is_public_suffix` (the list, moved from `fyx_registry`, which now re-exports it).
- No GPL dependency: `http_proxy::HostPattern` was reimplemented on `url::Host::parse`
  (IDNA, case, trailing dot, `*.` subdomains only, IP literals and `localhost` refused), and
  header syntax on RFC 9110 `token` and `field-value`.
- `ui_ext_wasm::network` keeps the engine (resolver, address rules, redirects, the decoded cap,
  running requests, delivery) and re-exports the rest; `ui_ext_wasm::permissions::PermissionState`
  is a re-export.
- `ui_ext_wasm` no longer depends on `fyx_registry`.


## Plan 4.7: `process:spawn`, host side (2026-11-18)

- **Pure part** (`crates/ui_ext/src/process.rs`, feature `process`; `t2` gates it and `network`):
  - `ProcessSpec`, which `UiManifest` re-exports;
  - the limits;
  - `SpawnRequest`, `FileRef`, `ProcessOutput`, `ProcessExit`, `SpawnError`;
  - `args_match` (literal, `*` within an argument, a final `**`), `is_forbidden_env` and
    `check_env_name` (the loader now refuses `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `GIT_SSH*`, …);
  - `check_spawn` (caps, declared environment names, patterns, the `cwd` rule; an undeclared
    program gets an id and `spawn-failed(not-found)`);
  - `ProcessCounters` (4 live).
  - `IoEvent` gains `ProcessOutput`, `ProcessExited` and `SpawnFailed`.
- **Host** (`crates/ui_ext_wasm/src/process.rs`):
  - `resolve_on_path`: skips empty, relative and in-worktree entries; on Windows `.exe` and
    `.com` only, `.bat` and `.cmd` refused; the absolute path is spawned;
  - `resolve_in_worktree` for `path` programs, trusted worktrees only, no link out;
  - `resolve_cwd`;
  - `program_environment`: cleared, then the base names, `LC_*`, the declared names, and the
    guest's declared pairs.
  - The tree dies with the program. On Windows the child is created suspended, assigned to a
    `KILL_ON_JOB_CLOSE` job, then resumed. On Unix it gets its own process group, and on Linux
    `PR_SET_PDEATHSIG`. Kill is SIGTERM, then SIGKILL after 2 s.
  - Output goes in ≤ 64 KiB chunks through the shared `handle-io` queue (backpressure); stdin is
    a queue (1 MiB per write, 4 MiB queued); `process-exited.killed` is set for host kills.
- **Kills** on `kill`, revocation (`changes()`), a trap, and dropping the instance (reload,
  disable).
- **Broker:** `state()` at every import (Ask accepted); `request()` when applied. An untrusted
  worktree is `denied` without a prompt; no worktree is `unavailable`.
- **LoadOptions:** `processes`, `process_env` (set by `load_checked`), `workspace`
  (`process::Workspace`, the host's worktrees with `trusted`), and `process_environment` (tests).
- **Tests:**
  - spec §4 failure 4, end to end: a planted `hostname` in the worktree with the worktree and
    `.` first on PATH, and the system one runs;
  - a reload kills the tree, checked by the grandchild's pid;
  - the 4-process cap, kill, `kill` on an exited id;
  - undeclared and `.cmd` programs → `not-found`;
  - revocation kills, and a refused prompt never starts;
  - untrusted → `denied`, no worktree → `unavailable`;
  - the filtered environment, and stdin;
  - plus unit tests of resolution and the environment.
- **Not done:**
  - Linux `PR_SET_CHILD_SUBREAPER`;
  - the Inspector line;
  - the Unix variants of the tests have not been run (no Linux runner for the full crate).

## Plan 4.8: `workspace:edit` and `agent:observe`, host side (2026-11-18)

- **Pure model** (`ui_ext::edit`, `ui_ext::observe`, feature `t2`; features `edit`/`observe`
  alias it):
  - edit types, the import checks `check_edit`: label 1..=128 bytes on one line, 1..=64 files,
    ≤ 4096 text edits, ≤ 4 MiB new text, the `file-ref` path rule, ranges in order;
  - `check_targets` when applied: overlapping ranges, two inserts at one point and a target named
    twice → `invalid`; `.git`, `.fyx`, `.zed`, `.vscode`, `.idea` as the first component (ASCII
    case-insensitive) → `not-allowed`; `EditCounters` (4 in flight);
  - review types, `cut_review` (256 KiB of hunk text, whole hunks; no hunks outside the
    worktrees), `cut_snapshot` (newest 1000), `OpaqueIds` (one map per extension).
  - `IoEvent::EditDone` and `IoEvent::Agent`.
- **Engine** (`ui_ext_wasm::edit`, `ui_ext_wasm::observe`):
  - `apply-edit` checks the manifest, the broker (Ask accepted), `check_edit` and the cap, and
    returns an id; after the call: `check_targets`, consent, then the host's `WorkspaceEdits`.
    A trap drops it; a revocation ends unapplied edits with `denied`; a reload stops them with no
    outcome. An applied edit's `ok` is not turned into `denied` at delivery.
  - `start`/`stop` (the last wins, applied after the call): consent, then the host's
    `AgentReviews::subscribe`; snapshot first, opaque ids, cuts; under backpressure a newer
    `updated` replaces a queued one, and past 1024 queued events it resubscribes for a fresh
    snapshot. `stop`, revocation, a trap and a reload end the feed (the host stream is dropped).
  - `LoadOptions.workspace_edits` / `agent_reviews`; without them the imports are `unavailable`.
  - `UiManifest::read` accepts both capabilities from 0.3.
- **Tests:** 11 end to end through the probe with fake editor and review sides (apply, import
  checks, protected/overlap never reach the editor, editor errors, trap, consent, revocation of
  each, feed and ids, stop/trap/reload, coalescing), a WIT test that no `agent-observe` function
  takes an argument, and unit tests of the rules and the queue.
- **For W3:** implement both traits in ui_ext_host (contract in their doc comments).
- **Not done:** the real implementations (W3 with W5's read API), the Inspector line.

## 4.6 follow-up 1: Inspector lines for T2 operations (2026-11-18)

- `Diagnostics.io: VecDeque<IoRecord>` (200 kept): capability, id, `what`, outcome (`None` while
  running), duration. Started by each handle after its call, finished in `Running::deliver`
  when the outcome is delivered; the agent feed notes start and stop.
- `what` holds no secrets: a URL without user info, query or fragment; a program and its
  argument count; an edit's label and file count.
- Rendering it in the Inspector is W3's (ui_ext_host/src/inspector.rs).

## 4.6 follow-ups 2 and 3: user-agent, decoders (2026-11-18)

- `user-agent: Fyx/<major.minor> (extension <id>/<version>)` (design §8); the version from
  `extension.toml` (`UiManifest.version`, `LoadOptions.extension_version`) only when it is a
  plain version string, else left out. Fyx's version from `release_channel::AppVersion`.
- `accept-encoding: gzip, deflate, br, zstd`; each is decoded under the 16 MiB decoded cap
  (`deflate` is zlib, RFC 9110); zstd windows are capped at 8 MiB (`window_log_max(23)`), so a
  frame cannot make the decoder allocate 128 MiB. Tests: round trip and bomb per encoding, the
  header, a wide-window zstd frame refused.
- Lock: async-compression features `brotli`, `zlib`, `zstd` in ui_ext_wasm add brotli 8.0.4,
  brotli-decompressor 5.0.3, alloc-no-stdlib 2.0.4, alloc-stdlib 0.2.4 (BSD-3-Clause/MIT;
  `cargo deny` advisories and bans ok).

## 4.6 follow-ups 4 and 5: TLS roots, local TLS test server (2026-11-18)

- `network:fetch` uses Zed's TLS exactly as its own client does
  (`use_preconfigured_tls(http_client_tls::tls_config())`): the platform verifier, falling back
  to the bundled webpki roots when it cannot load. No extension CA, no bypass.
- `test_server::start_tls` serves the same handler over TLS with a checked-in test CA and
  `localhost` certificate (`src/test_tls/`, DER, P-256, valid to 2126, regeneration steps in its
  README); `Fetcher::with_tls` (test only) trusts that CA. Test: https end to end, and the same
  request with Zed's roots fails with `tls`.
- Lock: ui_ext_wasm gains `http_client_tls` (dependency) and `rustls`, `tokio-rustls`
  (dev-dependencies), all already in the workspace.

## 4.6 follow-up 6: Fyx's proxy setting (2026-11-18)

- `network::ProxySetting`: a shared cell the host keeps current from Zed's `proxy` setting
  (the raw value, not `ProxySettings::proxy_url()`, which falls back to the environment); each
  request reads it. `http`, `https`, `socks5`, `socks5h` only; anything else goes direct.
- `LoadOptions.proxy`; environment proxies stay off (`no_proxy()` first). The address checks
  still run before a proxied request (the proxy resolves on its own); `localhost` is never
  proxied. Tests: the CONNECT reaches the proxy, a private answer is refused before it,
  `localhost` goes direct, clearing applies to the next request, the scheme rule.
- For W3: set it in ui_ext_host from `ProxySettings::get_global(cx).proxy` and on each change.

## 0.3 `permissions` interface, host side (2026-11-18, W4's gap)

- `query(capability)`: `unavailable` unless the manifest declares it, else the broker's state,
  mirrored per call (one call sees one answer per capability).
- `request(capability)`: not declared → `not-permitted`; needs the click-or-Enter activation
  and uses it up (§0.4), else `not-permitted`; in state `ask` the prompt (`broker.request`) is
  asked once the call returns (a trap drops it); other states change nothing. The answer
  reaches the guest as `permission-changed` through the broker's `changes()`.
- Probe: case 6 now traps on `language.unregister-provider`; cases 10 and 11 and the
  `perm query|request` commands. 3 tests.
- `AgentReviews::subscribe` is async (85a7d3c2ed) for W5's `agent_review::observe(&mut App)`.

## Retained-tree text cap (2026-11-18, triage-3 #7 remainder)

- `ui_ext::v0_2::tree::MAX_TREE_TEXT_BYTES` = 8 MiB of props text per surface tree (0.2 and
  0.3), counted by `Props::text_len()` (moved from ui_ext_wasm's conversions, same rules) and
  kept as a running total through insert, set-props, remove, replace and undo.
- Past it: `TreeError::TreeTextTooLarge { max }`, atomically like the other caps (a push is
  refused, a reply fails). `Tree::text_bytes()` for the Inspector.
- 0.1's tree is unchanged. Not counted: list-row trees, each built and dropped per fetch and
  bounded by `max_nodes_per_update` and the per-call text cap.

## integration-5 B items (2026-11-18)

- **R2** (`011d142682`, `0e60df0e1d`): a snapshot is verified against the keyset without the
  keys the previous one revoked; every revocation carries over; only a rotation-key snapshot
  lifts one. Old `state.json` (`protected_revocations`) still restores.
- **S2**: `ui_ext::protected` holds the agent-config rule list (moved from fyx_guard, which now
  uses it). `workspace:edit` refuses `.git`, `.fyx`, `.zed`, `.vscode`, `.idea` at any depth and
  every agent config (`.claude/`, `.codex/`, `.mcp.json`, `AGENTS.md`, ...) in every mode.
  `fyx_guard::extension_write_denial` (mode-independent) for W3's host side.
- **S1**: `fyx_guard::extension_read_denial` (private_files + `.fyx/security.toml`,
  mode-independent; repo policies now load in every mode) for W3's observe feed; as a floor,
  `ui_ext::observe::cut_review` drops the path and hunks of Zed's default private files.
- **S4**: with Fyx's proxy set, `network:fetch` to a declared domain is refused (`other`,
  "does not go through proxies yet"): reqwest cannot CONNECT to the checked address and keep
  the name for SNI, so the proxy would resolve again. Proxy URLs with credentials are never used.
  `localhost` still goes direct. Revisit with a hand-made CONNECT tunnel if proxied fetches matter.
- **S6**: more forbidden names (git's programs and dirs, pagers and editors, Java/.NET/Perl/Node/
  Lua/PowerShell module paths, `RUSTC*`, `NPM_CONFIG_*`, `YARN_*`, `CARGO_*`, `COR_*`, `PATH`,
  `PATHEXT`, `ComSpec`); the program's PATH drops empty, relative and worktree entries; no
  `args` patterns now means no arguments (`["**"]` for any).
- **S7**: `authorization`, `cookie`, `proxy-authorization` are dropped on a cross-origin
  redirect.
- **M2** (host side): `UiExtension::decoration_closed(id)`: a decoration the renderer dropped
  stops counting toward the 512 and loses its notifications, like `modal_closed`. W3 calls it.
- Drift: `ALL_UI_CAPABILITIES` should gain `ui:canvas`; held until W4 adds it to the harness's
  `ALL_CAPABILITIES` (the conformance test compares them).

## integration-5 A items (2026-11-18)

- **A2** (`cd9c665a92`): `MAX_FROZEN_VERSION` is an explicit 0.2.0; 0.2 loads in Stable and
  Preview, 0.3 stays staged. The comment says never to use `MAX_VERSION` for it.
- **A4**: `UiManifest::read` reads `api_version` first (a 0.4 manifest says "targets 0.4.0", not
  "unknown field"), then the world's manifest. `UiManifest::check_world` applies
  migration.md §2's table: per-version capabilities (error, not warning), and the 0.3 tables
  (`network`, `processes`, `process`, `commands`, `picker_sources`) only from 0.3.
  `[[fyx_ui.commands]]` (`id`, `title` ≤ 128, ≤ 64) and `[[fyx_ui.picker_sources]]` (`id`,
  `picker = "<extension>/<surface>"`, ≤ 64) are accepted and checked; nothing uses them yet.
  If the scope decision moves them to 0.4, only the version gate changes.
- **A6**: doc comments only, in both WIT files where the text is copied forward (so 0.2 and 0.3
  stay identical there): depth 64 and 8 MiB of tree text (`update`), 32768 pushed items
  (`push-update`), 24 h tick delay (`request-tick`), 512 decoration calls (`place-decoration`),
  4 notify/dismiss calls (`notify`), `notification-action` as a gesture (`show-surface`); 0.3
  only: `canvas-click` as a gesture, the protected paths of `edit-error.not-allowed`, the asset
  path rule. `docs/fyx/api/*.md` regenerated.

## R8 (integration-5): age gate from first sight

The 24 h gate counts from the later of `published_at` and when this client first saw the release (`FirstSeen`, kept in the snapshot state, carried across snapshots). The first index a client ever accepts records `published_at`. A backdated release from a leaked release key still waits 24 h.

## Owner decisions 2026-11-19 (A3, A5, Q1, Q2; Q5/Q8/Q10 checked)

- A3 (a8c2fee176): the smaller 0.3. The staged WIT has no documents, language, virtual-documents, picker or requests interface, no decoration extras or document targets, and no provider, command or picker exports. position, text-range and file-ref are in types; workspace-edit edits files only. 0.3 links 0.2's decorations. The 0.3 manifest refuses the 0.4 capabilities ("planned for a later version"), and commands and picker_sources are unknown fields.
- A5 (8b1c611f44): host-error-code gains untrusted-worktree and no-worktree; edit-error gains untrusted-worktree and other; spawn-error gains other; snapshot is review-snapshot {reviews, older-dropped}, and truncated means hunks only.
- Q1, revised (047e0ac8da replaces 17acb5ed05): a package with a Zed extension.wasm is accepted. The signed Release has zed_extension (in the canonical bytes only when true; was runs_zed_extension in 047e0ac8da), such a release must be T2, and verify_package checks the flag against the package. The template derives the flag and the T2 tier. W3 owns the warning text (RUNS_ZED_EXTENSION_WARNING).
- Q2 (eac5325c40): ui_ext::process::RUNS_ANY_CODE, runs_any_code and RUNS_ANY_CODE_PROMPT. Not done: registry review needs args patterns or a written reason. The template refuses T2 today, and a release lists process names only, without args, so this waits until T2 opens in the registry.
- Q5: apply-edit needs no gesture (only permissions.request takes the activation); showing edits is W3's. Q8: cut_review keeps whole hunks up to 256 KiB for worktree files and nothing outside them. Q10: the prompt is W3's UI; the host side needs no change.

## integration-6 (2026-11-19)

- **Plan 4.13 lift bench** (`test_canvas_lift_bench` in `ui_ext_wasm/src/tests_v0_3.rs`, probe
  commands `canvas <n>` and `canvas-push <n>`): `cargo test -p ui_ext_wasm --profile release-fast
  --lib canvas_lift_bench -- --ignored --nocapture`. A `#[ignore]` test, not `cargo bench`: a
  bench target would need a `Cargo.toml` entry (integrator-only). Results (release-fast, probe at
  opt-level s, mixed ops, median of 30, two quiet runs):
  - round trip (guest clone and lowering, lift, convert, checks, effects, one dispatch): 10 000
    ops 1.13–1.27 ms, 16384 ops 1.9–2.0 ms (3.0 ms in a noisy run);
  - inside the push call: 10 000 ops ≈ 1.0 ms, of which the host's convert and checks are
    0.16 ms (`check_canvas_props` alone 0.09 ms); the rest is wasmtime's canonical-ABI lift and
    the guest's lowering, which the host cannot tune without an ABI change;
  - the cost per op is flat, so a lower cap would not change the per-10k figure. Recommended to
    the coordinator: the 16384 cap stays (a full canvas is about 2–3 ms of the 200 ms budget).
    The test asserts ≤ 2 ms per 10 000 as a regression guard.
- **N1:** `[fyx_ui.process] env` is an allowlist (`ui_ext::process::ALLOWED_ENV_NAMES`: locale,
  terminal and output, `CI`, `RUST_LOG`/`RUST_BACKTRACE`, `NODE_ENV`, `PYTHONUNBUFFERED`/
  `PYTHONIOENCODING`/`PYTHONUTF8`, and `LC_*`). `HOME`, `USERPROFILE`, `XDG_*`, `ZDOTDIR`, ... and
  Fyx's own `FYX_*`/`ZED_*` (Fyx reads `FYX_UI_DEV_EXTENSIONS`) are refused at load and at the
  call. The denylist stays for its clearer message. Additions go through W2c.
- **N4 (registry half) and R9:**
  - the highest block-list sequence ever accepted is kept in the snapshot state
    (`blocklist_floor`), so dropping the cache (a revoked block-list key) does not forget it; a
    list below it is `KeepReason::BelowFloor`;
  - with no cache, an expired list is `KeepReason::Expired`;
  - `RegistryClient::blocks_can_lift(now)`: a block lifts only while a list is held and not
    expired. W3's `apply_plan` must re-enable only when it is true;
  - a hash-only block entry also matches an install whose hash was never recorded;
  - "daily" in `blocklist.rs` is now "every 12 h" (spec 19);
  - **role separation, decision:** clients do not check it per document (they check each
    signature's role); the compiled-in keyset is fixed at build time, so the build checks it
    once: W3's `release_keyset()` should refuse a keyset that fails
    `check_role_separation(None)`. Its two error texts had lost a line break (fixed).
- **R8 `urgent`:** it no longer skips the age gate on its own. It skips it only to replace an
  installed version the block list blocks (so the block-list key, held apart from the release
  key, must agree). `AgeGate::Urgent` is gone. Spec decision 17's text needs the same narrowing
  (W10).
- **F3:** `permissions.request`'s error lists click, row-click, input-submitted, canvas-click and
  notification-action; the `fyx_ui_v0_3` module doc describes the smaller 0.3.
  `define_unbuilt_as_traps` stubs nothing for 0.3 and stays for the next staged world.
- **Q2:** `runs_any_code` ignores a version suffix (`python3.12`, `node22`, `lua-5.4`), lists more
  runners (`wsl`, `ssh`, `rundll32`, `regsvr32`, `msiexec`, `schtasks`, `docker`, `podman`, npm/
  pnpm/yarn, cargo/rustup/go, make/cmake/ninja/just, rake/bundle/gem/composer, gradle/mvn/ant,
  uv/uvx/pip/pipx/poetry/conda/mamba), and for `git` sees `--git-dir`, skips the values of `-C`,
  `--work-tree` and `--namespace`, and treats the subcommands config, bisect, submodule, rebase,
  filter-branch, difftool, mergetool and daemon as "any program".
- **Decisions recorded (coordinator, 2026-11-19):** Q4 = (2): loopback only with a declared port
  (built), other private addresses only from a user setting (`private_hosts`, not built; it is
  additive). Q7 = (3): 256 MiB for every world, as the code does. Q14 = (1): paint each frame,
  a per-update texture only if W3's paint bench misses 4 ms.
- **W3 dfdb6b0023 follow-ups:** `Permissions.zed_extension` (serde default, so old `state.json`
  has not accepted it) and `Additions.zed_extension`: an approved T2 install whose update newly
  runs as a Zed extension is held for approval. `BUTTON_CLICK_GUARD`'s doc says the host applies
  at least 1 s to every card (the value stays 750 ms, the renderer's floor).

## Handoff (2026-11-19, second context clear)

- **Branch:** claude/fyx-w2c-host-0-2-cbde2c, pushed. Its base is fyx/main ebafa3f573, merged in
  as a fast-forward.
- **Tip, ready at the gatekeeper (not merged):** 226eea0e96, which includes f53acdcfd4,
  4458b3abd9 and 099d5cf888 (integration-6) and the zed_extension additions. It also carries the
  earlier ready batch (A2, A3, A5, R6, R8, Q1, Q2, prescan_svg, accessible-label) if W0 has not
  merged that yet.
- **Breaks in others' crates** (lists sent; merge with or after their fixes):
  - W4: `tests_v0_3::harness_conformance_process` declares env `FYX_TEST` at lines 1941, 1959
    and 2005. Two tests fail; the fix is `NO_COLOR`. The harness's `permissions.request` text
    must match F3's (`api_state.rs:1157`).
  - W3: done in ba5de3f4a4 (on claude/fyx-w3-renderer-e20d2b and `integration`, checked
    2026-11-19): the consent test env is `NO_COLOR`; `apply_plan` re-enables only when
    `blocks_can_lift(now)` holds and the list is fresh; `release_keyset()` goes through
    `parse_release_keyset`, which refuses a keyset failing `check_role_separation(None)` (tested);
    `describe_additions` shows `RUNS_ZED_EXTENSION_WARNING` for `Additions.zed_extension` and the
    host-side hold is gone. Merge W2c with or after it.
- **Docs:** W10 did the R8, R9/N4 and N1 texts (a768772afb on claude/fyx-w10-docs-a5772b).
- **Canvas cap:** 16384 stays on my recommendation (bench ~1.1–1.3 ms per 10 000 round trip;
  the excess is the ABI lift). The coordinator records the decision.
- **Open items for the 1.0 cut** (after the 0.3 freeze; nothing done yet):
  - drop 0.1 and 0.2 loading (Q9): `MIN_VERSION`, the 0.1 shim and the 0.2 world, their probes
    and tests, and the 0.1/0.2 manifest paths;
  - rename the 0.3 world to 1.0.0: the WIT package version, `since_v0.3.0`,
    `MAX_FROZEN_VERSION`, `ApiVersion::V0_3_0`, manifest `api_version` gating and the capability
    tables, coordinated with W0 (the WIT is integrator-only) and W4 (SDK and harness).
- **Waiting for T2 in the registry:** Q2's registry part (registry review needs args patterns or
  a written reason for a "runs any code" program) and R7 (process args and env in the signed
  release). Releases list process names only, so the args go into the release first.
- **Q4 `private_hosts` (2026-11-19):** `ui_ext_wasm::network::PrivateHosts` (shared handle, like
  `ProxySetting`; `LoadOptions::private_hosts`). Entries are names of two or more labels or `*.`
  plus one (subdomains only); IP literals, ports, `localhost`, `*` and `*.<tld>` are ignored and
  returned by `set` for logging. A listed name that the extension also declares may resolve to
  RFC 1918, `100.64/10` and `fc00::/7` (`is_private_network`), never loopback or link-local;
  `https` is still required. The refusal names the setting. W3 wired the setting in 1534a1e7de:
  user settings only, one global set on every change, both load paths pass it. Q4 is done.
- **P5 5.5, step 1 (2026-11-19): `ui_ext::terminal`** (behind `t2`; W0 may add a
  `terminal = ["t2"]` alias). Contents: the types of the 0.4 `terminals` interface, the
  `TerminalHost` trait and `TerminalSink`, the limits, the
  profile/program/env tables with `reason`, `Template` and `TemplateContext`,
  `is_forbidden_terminal_env`/`is_account_env`, and `ACCOUNT_DIR_ENV_NAMES` and
  `CREDENTIAL_ENV_NAMES`. Agreed with W1:
  - W1 implements `TerminalHost` in a new `terminal_host` crate; terminal_ext stays a leaf.
  - A sink may be called from any thread and only queues.
  - W1 allocates handles and never reuses them in one load.
  - **Decision (mine):** a name with a credential suffix (`_TOKEN`, …) may carry exactly one
    `${listen.<n>.token}` of a `token_in_env` listener: that is Fyx's own per-run secret. The
    explicit credential names, the account-dir names and `FYX_*` stay refused.
  - Profile icons are frozen `Icon` names and optional.
- **P5 5.5, step 2: the WIT draft** `tasks/wit/fyx-ui-0.4.wit` (145f748bbf), generated from
  0.3 plus the design's diff. `wasm-tools component wit` parses it. Keyword renames: `%list`, and
  `respond(…, outcome)`. It went to W0 to stage as `since_v0.4.0`; the corrections went to W10.
  Plan 4.2–4.4's parts are not in it yet.
- **P5 5.5, step 3: 0.4 in the loader** (W0 staged the WIT as bce8df3b54, merged via
  fyx/main):
  - `ApiVersion::V0_4_0` with `WIT_DIR_V0_4_0`. `ui_ext_wasm::MAX_VERSION` is 0.4.0 (staged);
    `MAX_FROZEN_VERSION` stays 0.3.0.
  - `dispatch.rs` refuses to instantiate a 0.4 component: "this build does not run 0.4
    components yet". No 0.4 bindings exist yet.
  - New `ui_ext::integration` module: `ListenerSpec`, `DiscoveryFileSpec` and
    `RelayChannelSpec` with their checks, and `IntegrationTables::listener` for the templates.
  - `UiManifest` gains the six 0.4 tables, each refused below 0.4.
  - `UiManifest::check_terminals`, called from `read`: each table goes with its capability,
    plus the `ui_ext::terminal` and `ui_ext::integration` checks.
  - `world_capabilities` has a `V0_4` list. The new capabilities have T0, T1 and T2 tiers in
    `permissions.rs`. `terminal:read` and `terminal:write` (T3) are refused as unknown until
    5.6.
  - Tests: `tests_v0_3::test_v0_4_manifest_tables` covers the plan's acceptance cases. The
    gating and load-from-dir tests now use 0.5.0 as the unknown version.
- **Breaks sent to W4** (merge with or after their fix):
  - `fyx_ext_harness.rs:~1009` and `api_state.rs:242`: an `ApiVersion::V0_4_0` arm is missing;
  - `tests.rs` `harness_conformance::limits_match` asserts `MAX_VERSION == 0.3`;
  - the mirror list for `fyx_ext/src/manifest.rs` and the harness.

## Handoff (2026-11-19, third context clear)

- **Branch:** claude/fyx-w2c-host-0-2-cbde2c, pushed, with fyx/main merged.
- **Ready at the gatekeeper:** 22dbc5ea54 (Q4), ba7186021a (`ui_ext::terminal`) and the 5.5
  step 3 commit (see the ready message); 282e99d69a (title rule) rides along.
- **Next turn, after the clear:**
  1. The 0.4 import side in ui_ext_wasm:
     - bindgen for `since_v0.4.0`, and `dispatch.rs` `V0_4` built from the 0.3 linker plus the
       new interfaces;
     - the `terminals` import over `ui_ext::terminal::TerminalHost` (`LoadOptions::terminals`);
     - the who-checks-what split agreed with W1 (top of `terminal.rs`);
     - the io events from `TerminalSink`;
     - stubs that trap for listeners, relay, review, context and approvals until 5.11 and 5.13.
  2. 5.6, the broker:
     - `Tier::T3`;
     - T2 at install with per-item grants;
     - "Always allow" bound to the component hash;
     - the journal store (`<data_dir>/extensions/fyx/journal/<id>.jsonl`, never terminal
       content);
     - revocation that calls `TerminalHost::close_all`;
     - the consent items of the 0.4 tables for the update diff (§C.5).
     - `ui_ext_host`'s `consent.rs` and `permissions.rs` are W3's crate: agree the API with W3
       first.
  3. `reason` for `[[fyx_ui.processes]]` and `[fyx_ui.network]` in 0.4 manifests. It is not done,
     because `ProcessSpec` has 13 struct literals in other crates. Add it as a 0.4-only field
     that is refused below 0.4.
  4. `fyx_registry`'s capability tiers and `capability-tiers.json` for the 0.4 capabilities,
     with W10's 5.21: T3 counts as T2 for the release tier.
- **W1** implements `TerminalHost` in `terminal_host` (phase 2). Agreed: observe redacts titles
  to `•••`, except profile names and the extension's own terminals.
- **Gotchas:**
  - `ui_ext_wasm`'s tests need `fyx_ext_harness` to compile. Until W4 adds the `V0_4_0` arms,
    patch the harness locally for a run and revert it with `git checkout -- crates/fyx_ext_harness`.
  - PowerShell `.Replace` with `` `n `` misses lines in CRLF files; use the Edit tool.
- **Gotchas (older):**
  - Targets: `E:/fyx-targets/w2c` (release-fast build of ui_ext_wasm, for the bench and the
    suite) and `E:/fyx-targets/w2c-light` (fyx_registry and ui_ext only).
  - ui_ext's t2 modules are tested only with `--all-features`.
  - Heredoc Python edits lose backslashes: build them with `chr(92)`.
  - Run `bash script/fyx-prune-target <target>` after tests.
