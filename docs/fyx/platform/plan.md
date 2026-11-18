# Platform 1: work plan

Implements [spec.md](spec.md) (decision numbers in brackets). Owners are the existing streams;
"new" means a stream the coordinator starts. Every task ends with the behaviour seen in the
running editor, not only green tests. Integrator-only files (`Cargo.toml`, `Cargo.lock`,
`crates/ui_ext/wit/`) go through W0 as usual. Revised for the 0.2 decisions, [decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17):
decorations moved from P4 to P2 (§L); notifications and progress are §M.

## Before P1: merge what already exists

| # | Owner | Task | Done when |
|---|---|---|---|
| 0.1 | W0 | Merge `claude/fyx-ext-template` (`559014f693`) and `claude/fyx-ts-sdk` (`af96c0f873`) after review; they seed `fyx-ext new` and `sdks/ts` | both on `integration`, gate green |
| 0.2 | W0 | Merge the pending audit and review branches (`fyx-audit-network`, `-diff`, `-licenses`, `fyx-review-w5`, `fyx-rebase-report`) and route their findings; network-audit findings feed P1.6 and P4.6 | findings listed in `tasks/reviews/` with owners |

## P1 local platform

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 1.1 | W3 | Install store: layout, `state.json`, keep previous version, rollback, pin [13]; keep a precompiled `.cwasm` per installed version, keyed by the wasmtime engine, so large components start without recompiling | — | install, update, roll back a local `.fyxext` in the dev build; a second start skips compilation; only `.cwasm` files Fyx wrote into the install store are ever deserialized, never one from a package |
| 1.2 | W3 | "Install Dev Extension" + `fyx.extensions.dev_paths`; env var stays as override [6] | 1.1 | dev extension survives restart without the env var |
| 1.3 | W3 | Hot reload in place + "Restart extension" after a failure [7, 8] | 1.2 | edit → rebuild → panel updates within 1 s; failure 3 of spec §4 passes |
| 1.4 | new `platform-cli` | `fyx-ext` crate and binary: `new` (from the template), `build`, `dev`, `check` with declared-vs-used capabilities, `package` [4, 5, 12] | 0.1 | `fyx-ext new demo && fyx-ext dev` opens the panel in Fyx |
| 1.5 | W3 | Permission broker, tiers T0–T1, install dialog, revocation, capability list on the page (reuse capability chips) [21, 26] | 1.1 | T1 extension asks once; revoke stops reads at once |
| 1.6 | W3 | Instance limits: no preopens, no env, 256 MiB memory, 2 s first mount [25] | — | a guest that allocates 300 MiB traps without hurting others |
| 1.7 | W3 | Extension Inspector panel [9] | 1.3 | tree, patches, events, timings visible for the counter demo |
| 1.8 | W4 | `fyx_sdk::testing` harness + `fyx-ext test` [10] | 1.4 | demo tests run headless in CI |
| 1.9 | W7 | Generated API reference + doc drift test [11]; rewrite `extension-authoring.md` around `fyx-ext` | 1.4 | reference regenerates on merge; drift test catches a renamed item |
| 1.10 | W0 | wasmtime policy: Cranelift only, `cargo deny advisories` in `script/gate.sh` [27] | — | gate fails on a seeded advisory |

## P2 `fyx:ui@0.2.0`

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 2.1 | W0 | Open questions answered and §L decorations added, [decided 2026-11-17](../wit-0.2/design.md#decisions-2026-11-17). **Landed** (data layer, [migration.md "Status"](../wit-0.2/migration.md#status-2026-11-17)): the 0.2 WIT in `crates/ui_ext/wit/since_v0.2.0/` next to the unmoved 0.1 file [29]; `ui_ext::v0_2`, `ApiVersion`, `LiveIds` with unit tests and proptests. Section M (notifications, progress) [22] was drafted in design.md and added to the WIT in place of its `TODO` on 2026-11-18 (W2c `700077009f`); the host treats 0.2 as frozen (`MAX_FROZEN_VERSION = 0.2.0`, `8abde04072`) | — | both worlds in `crates/ui_ext/wit/`, 0.1 demos unchanged and working; M drafted and in the WIT before 0.2 is frozen |
| 2.2 | W2c | Host side of A–M (including the `decorations` interface of §L), version dispatch. On the 2.1 data layer: `bindgen!` on `ui_ext::WIT_DIR_V0_2_0` next to the 0.1 bindings (moving 0.1 to `since_v0.1.0/` is optional, migration.md §1); dispatch on `ui_ext::ApiVersion`; 0.2 conversions into `ui_ext::v0_2` and 0.1 ones through its `From` impls; `UiState` keeps a `ui_ext::LiveIds` for `add-surface`, `remove-surface`, `place-decoration`, `remove-decoration` and the targets of `push-update` / `request-tick` / `show-surface`; `push-update` checks `v0_2::Update::check_caps` and returns `v0_2::HostError::from(&ApiError)`; the per-call caps the model leaves to the host (64 pushes, 512 decoration effects); ticks, the gesture window, `ui:modal`, staging by channel; M once drafted | 2.1 | probe fixture covers every new import |
| 2.3 | W3 | Renderer side of A–K and M: ticks, modal with keyboard selection and the click-or-Enter gesture rule, dismissed, runtime surfaces, frozen icons, notifications and progress. On the 2.1 data layer: the renderer moves from the 0.1 root types to `ui_ext::v0_2` (one coordinated change with 2.2, since both build those types); `Tree` moves with it and gains design G (`Patch::InvalidateRows`, `TreeError::NotAList`, the `Touched` result), which 2.1 left out because it changes `Tree` under the renderer; icons through `IconRef` (exhaustive `match` for `Frozen`, today's `from_str` for `Upstream`); tooltips via `ButtonProps::tooltip_text`, indent via `ListRow::indent_levels`; the per-version rules of migration.md §3 | 2.2 | clock status item, picker modal, progress demo in the dev build |
| 2.4 | W4 | Rust SDK 0.2 (with `place_decoration` / `remove_decoration` for §L) + demos moved to 0.2 as examples | 2.2 | demos build for both 0.1 and 0.2 |
| 2.6 | W6 (restart) | Decorations §L on W6's anchor mechanism: an extension backend for `Decorations`, worktree-relative path matching, the registry effects, the 512 cap | 2.2 | a demo places blocks under TODO lines that follow edits and route clicks to `handle-event` |

## P3 registry

**Status 2026-11-19.** The user created `shelenhovskyi/fyx-extensions`. It is **private**, and goes
public at 1.0. **3.1 is done:** its `main` holds one initial commit, `551cdc8`, of
`tools/fyx-extensions-template/` from `fyx/main` `062d6c99cd` (pushed with the user's yes). The user does the rest later: branch rules, Environments, keys, and the GitHub App behind
`AUTO_MERGE_TOKEN` ([registry-repo.md](registry-repo.md#bootstrap-task-31-then-32)).

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 3.1 | W0 | Create repo `shelenhovskyi/fyx-extensions` (owner's decision, 2026-11-17); entry format, review rules as CODEOWNERS + a CI check for decision 16 [14, 16]. Moderation (owner, 2026-11-18): agents plus CI review every entry; T0–T1 entries auto-merge on green CI, T2 needs the owner's approval. The `policy` decision and the `needs-owner` label run in the trusted `review.yml`, started by `workflow_run` of `pr.yml` and of `review-trigger.yml` (a review on a fork PR has a read-only token) ([registry-repo.md](registry-repo.md#reviewyml-the-agent-review-and-the-policy-decision)). Reserved ids only for entries owned by the registry owner alone, with the owner's approval; the owner's own PR counts as the owner's approval. Template: `tools/fyx-extensions-template/` (W2c) | 1.4 | a PR adding a T2 capability cannot merge without a maintainer; a PR with `needs-owner` cannot auto-merge |
| 3.2 | W0 | Registry CI: build Rust extensions from source in a pinned image, `check`, `test`, Ed25519 sign, publish `index.json`, `snapshot.json` + packages to Releases, and the 12 h refresh of the snapshot and block list ([registry-repo.md](registry-repo.md#ci)) [15]. The `agent-review` job: an agent reviews every entry's diff and built package, with no signing keys or write token and the PR as untrusted data, and can only escalate (`concern` adds `needs-owner`) [16]. Auto-merge through a GitHub App token (`AUTO_MERGE_TOKEN`, Environment `auto-merge`), since a `GITHUB_TOKEN` merge does not start `publish.yml`; the owner creates the App with the repository | 3.1, 1.8 | a sample extension appears in `index.json` with a valid signature; a PR whose README tells the reviewer to approve gets `concern` (`prompt-injection`) and `needs-owner` |
| 3.3 | W3 | Registry client: index, 24 h age gate, `urgent`, signature check, unsigned-file badge [17, 18]. The client compiles in the three keysets (release, snapshot, block-list keys), verifies the snapshot first (expiry, sequence, revoked keys), then the index against it and the releases against the release keys | 3.2 | install from the index; tampered package refused; a release signed with a block-list key refused; an expired or rolled-back snapshot stops installs and updates |
| 3.4 | W3 | Block list: fetch every 6 h, disable, auto-rollback, cached copy, 7-day warning [19]. Fetched and cached lists are verified only against the block-list keys, minus revoked ones; stale after 7 days or `expires_at` | 3.3 | spec §4 failures 1, 2, 5 pass; a block list signed with a release key refused |
| 3.5 | W3 + one `fyx:` hook | Extensions page Fyx tab: search, install, update, pin, roll back, held updates [17, 21] | 3.3, 1.5 | user flow done end to end in the dev build |

## P4 `fyx:ui@0.3.0` and T2

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| 4.1 | W0 | 0.3 design draft for decision 28 (cloud task, like the 0.2 draft), then the WIT | P2 | design reviewed by W7 |
| 4.2 | W6 | **Moved to 0.4** (owner, 2026-11-19). Decoration extras beyond 0.2 §L (decision 28): inline kinds, gutter signs, line backgrounds, namespaces, visible-range pull, `color-swatch`, `decoration-orphaned` | 4.1, 2.6 | TODO highlighter and gutter-sign demo work while editing |
| 4.3 | W3 | **Moved to 0.4** (owner, 2026-11-19). Picker protocol on the 0.2 modal | 4.1 | two extensions contribute sources to one picker |
| 4.4 | W3 | **Moved to 0.4** (owner, 2026-11-19). Code lens, hover, virtual documents, commands in palette and keymap | 4.1 | each has a demo in the gallery |
| 4.5 | W3 | T2 consent at first use, held updates for new T2 / domains / programs [21] | 1.5 | spec §4 failure 1 passes |
| 4.6 | W2c | `network:fetch` host import with domain list and caps [23] | 4.5 | undeclared domain → `denied`; 17 MiB response cut |
| 4.7 | W2c | `process:spawn`: PATH outside worktree, filtered env, job objects [24] | 4.5 | spec §4 failure 4 passes; reload kills the process tree |
| 4.8 | W2c | `workspace:edit` (undoable transactions), `agent:observe` read-only sessions | 4.5 | an edit undoes with one ctrl-z; observe cannot accept or reject |
| 4.9 | W7 | Judge pass over P1–P4, then the 1.0 freeze clock starts [30]. At the 1.0 cut, loading 0.1 and 0.2 is removed (owner, 2026-11-19) | 4.1, 4.5–4.8, 4.10–4.15 | review file in `tasks/reviews/` |
| 4.10 | W2c | Images, host side ([design §12](../wit-0.3/design.md#12-images-plan-410412)): `image` props and validation (alt, sizes, asset paths, panels only), the `images` import with the 1 MiB and 32-handle caps, `ui:images` (T0) [28] | 4.1 | the image bomb and path fixtures are refused; a 0.2 component cannot send `image` |
| 4.11 | W3 | Images, renderer: decode queue off the main thread, PNG and SVG decoders with the §12 limits, 64 MiB LRU per extension, placeholder with alt text, clipping [28] | 4.10 | logo, file preview and generated SVG show in a panel; the decoder fixtures fail without a large allocation |
| 4.12 | W4 | Images, SDK and harness: builders, `add_image`, harness asset checks; an image gallery demo [28] | 4.10 | demo runs in Fyx and headless in `fyx-ext test` |
| 4.13 | W2c | Canvas, host side ([design §13](../wit-0.3/design.md#13-canvas-plan-413415)): draw-op types, caps (16384 ops, segments, text, 16 per surface), float checks, `canvas-click`, `ui:canvas` (T0) [28] | 4.10 | the cap and NaN cases are refused; the lift bench is at most 1 ms per 10 000 ops |
| 4.14 | W3 | Canvas, renderer: GPUI paint of the draw list with tokens and the data palette, clipping, `scale-down`, hit-test mapping [28] | 4.13 | the 16384-op paint bench is at most 4 ms; click coordinates are right when scaled and scrolled |
| 4.15 | W4 | Canvas, SDK and harness: typed builders, `canvas-click` in the harness; a bar-chart and sparkline demo [28] | 4.13 | demo runs in Fyx and headless |

**Decisions of 2026-11-19 (owner):**
- **The smaller 0.3.** 0.3 is images, canvas, the T2 imports and notifications. Tasks 4.2–4.4 move
  to 0.4, and the staged 0.3 WIT drops their interfaces.
- **No pre-1.0 support.** The frozen 0.3 becomes `fyx:ui@1.0.0`, and loading 0.1 and 0.2 is
  removed at the 1.0 cut.
- **Q2, Q5, Q8 and Q10 are answered** as recommended. **Q1 differs:** a package with an
  `extension.wasm` is accepted, always as tier T2, with an explicit warning
  ([0.3 open questions](../wit-0.3/open-questions.md)).

**Priority within P4 (owner's rule, 2026-11-18):** P4 is the first candidate to cut. Tasks 4.10–4.15
rank below every other P4 task. If P4 slips, 4.10–4.12 (images) stay and 4.13–4.15 (canvas) move
to 0.4.

**4.6 follow-ups** (W2c's "not done yet" list in `tasks/progress/W2c.md`, at 0213dd3cc7). The
first cut of `network:fetch` landed without these; each is W2c's, needed for 4.6 to match
[design §8](../wit-0.3/design.md#8-networkfetch-plan-46).

Done (W2c, `claude/fyx-w2c-host-0-2-cbde2c`):

- ~~brotli, zstd and deflate decoding~~: **done** in 1cec81f03f. `accept-encoding: gzip, deflate,
  br, zstd`, each under the 16 MiB decoded cap, zstd windows capped at 8 MiB;
- ~~the Fyx version in the user-agent~~: **done** in 9fad627507,
  `Fyx/<major.minor> (extension <id>/<version>)`;
- ~~one Extension Inspector line per request~~: **done host side** in 6b36bc4e7d. Each fetch,
  process, edit and agent-feed operation leaves a record (capability, id, what, outcome,
  duration), with the URL stripped of user info, query and fragment; never bodies or header
  values. Showing the records in the Inspector is W3's (`ui_ext_host/src/inspector.rs`).

- ~~the webpki roots and a local TLS test server~~: **done** in d11017466c (Zed's TLS config:
  platform verifier, webpki roots as fallback; a test CA only in tests);
- Fyx's proxy setting: **done as a refusal** in b3822c6fce and a1c2557b27. Fyx's own setting is
  read, never environment proxies, and credentials are never used. A fetch to a declared domain
  is refused while a proxy is set, because reqwest's CONNECT cannot keep the checked address and
  the SNI name. `localhost` goes direct.

After 1.0: proxied fetches through a hand-made CONNECT tunnel that pins the checked address
([design §8](../wit-0.3/design.md#8-networkfetch-plan-46), "Proxy").

All six are done (2026-11-18, W2c `6b36bc4e7d`, `9fad627507`, `1cec81f03f`, `d11017466c`,
`b3822c6fce`). The proxy turned out unsafe as built and now refuses proxied domain fetches
(integration-5 S4); routing through the proxy is D.2.

## Deferred after 1.0

TypeScript extensions are deferred until after 1.0, and whether to do them at all is re-evaluated
then (owner, 2026-11-18; spec decision 3). P3 builds and signs Rust extensions only. `sdks/ts`
stays in the repository as an unsupported prototype.

| # | Was | Task | Revisit |
|---|---|---|---|
| D.1 | 2.5, new `ts-sdk` | `@fyx/sdk` on the current world with `jco componentize`, `fyx-ext new --lang ts`, SDK parity test | after the 1.0 freeze |
| D.2 | 4.6 follow-up, integration-5 S4 | `network:fetch` through Fyx's proxy: a hand-made `CONNECT` to the checked address, keeping the name for SNI, `Host` and the certificate check, so the address pinning holds behind a proxy. Until then a proxied fetch to a declared domain is refused (W2c `a1c2557b27`) | after 1.0 |

## Parallelism

At most 2 compiling streams plus the gatekeeper (owner limit). Non-compiling work runs alongside:
1.9, 2.1, 3.1, 3.2 (registry repo) and 4.1 (design). Suggested pairs:
P1 = W3 (1.1→1.3→1.5→1.7) + `platform-cli` (1.4, then 1.8 with W4); P2 = W2c + W3, then W6
for 2.6;
P3 = W3 alone (registry CI is non-compiling); P4 = W6 + W3, then W2c.
