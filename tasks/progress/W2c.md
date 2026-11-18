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
  `host`, `images`; every other 0.3 interface traps when called (`define_unknown_imports_as_traps`),
  so a component that does not call them works.
- **`images` import**: `ui:images` declared and granted by the broker at each call, the byte
  checks, at most 32 live handles, handles never reused; `HostEffect::AddImage { handle, format,
  bytes }` / `RemoveImage { handle }`. `ui:images` and `ui:canvas` are T0.
- **Not yet**: `image` nodes are refused (`invalid-argument` in a push, the call's error in a
  reply) until the model has `Props::Image`, which waits on W3 (asked 2026-11-18: add it to
  `ui_ext::v0_2::Props`?). Then the host checks `check_image_props` and `check_image_placement` on
  every update, as it already can.
- Tests: `fixtures/probe_v0_3` and `tests_v0_3.rs` (the bomb, 1 MiB + 1, entity SVG, junk, the
  32-handle cap, the capability, image nodes refused, traps), unit tests in `ui_ext::image`,
  `effects` and `images` (path fixtures, links out on Unix).
