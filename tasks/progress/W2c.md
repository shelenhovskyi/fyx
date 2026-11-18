# W2c progress: host side of `fyx:ui@0.2.0` (platform plan 2.2)

Branch `claude/fyx-w2c-host-0-2-cbde2c`, on `integration` (with p2-wit) plus W3's
`claude/fyx-w3-renderer-e20d2b` at `4ca0073db1` (cache, `CallBudget`, lifted-text cap, broker,
Inspector diagnostics), as the coordinator asked.

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
