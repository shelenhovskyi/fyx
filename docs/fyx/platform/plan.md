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

## P5 terminals and CLI integrations

Decided by the owner on 2026-11-19, from the research in
[`terminal-api/research.md`](../terminal-api/research.md) (Ukrainian). Design:
[wit-0.4/design.md](../wit-0.4/design.md). The coordinator's answers to the open questions are
in its "Decisions": documented protocols and OSC 7501 first, Claude Code's IDE protocol in
phase 4 with a compat test; env in profile terminals by default with an "all new terminals"
checkbox; SSH and PowerShell shell integration in phase 6; loopback TCP on every OS.

Owners: **W1** terminal host, **W2c** WIT, model and permissions, **W3** renderer and consent UI,
**W4** SDK, harness and demos. W0 stages the WIT (integrator-only), W7 reviews.

| # | Owner | Task | Depends on | Done when |
|---|---|---|---|---|
| | | **Phase 1, terminal foundation (layer A), about 2 weeks** | | |
| 5.1 | W1 | `crates/terminal_ext`: `TerminalId`, the terminal registry (`observe_new`, `on_release`), `FYX_TERMINAL_ID` and `FYX_TERMINAL_TOKEN` through one `fyx:` hook in `insert_zed_terminal_env` ([design §A](../wit-0.4/design.md#a-the-terminal-foundation-layer-a-host-only) 1) | — | every local terminal has both vars; ids survive a tab move; the hook is ≤ 3 lines |
| 5.2 | W1 | OSC tap: `TeePty` prototype first, then OSC 7, 9/777/99, `9;4`, 1337 and **7501** with the support-query reply from the spec, size and rate caps on the IO thread (§A 2) | 5.1 | a recorded Claude Code 2.1.295+ session sets the tab state; bytes reach alacritty unchanged; the swap is one line |
| 5.3 | W1 | `TerminalStatus` and the `TerminalDecorations` provider; status icon and host-drawn source on the tab (§A 3) | 5.2 | OSC 7501 `blocked` shows on the tab with "from <app> via OSC 7501"; `tab_content` diff ≤ 15 lines |
| 5.4 | W3 | Agent sessions panel ("needs attention" first, click to focus) and window-unfocused notifications on `blocked` and `done` (§A 4) | 5.3 | two Claude sessions sort by attention; a notification fires only while the window is unfocused |
| | | **Phase 2, consent model and the 0.4 terminal world, about 3 weeks** | | |
| 5.5 | W2c | 0.4 model and manifest: the `terminals`, `listeners`, `relay`, `review`, `context` and `approvals` interfaces, the `terminal` node, the new manifest tables with `reason`, T2 additions and `t3-capability`; the `terminal:env` forbidden names, including the auth and config-dir names (design §B.4, §C.6); `world_capabilities`, `fyx-ext` and harness lists. WIT diff to W0 for `since_v0.4.0` | P4 | `wasm-tools component wit` parses the staged world; a 0.3 manifest with a 0.4 table is refused; `PATH`, `HOME`, `CLAUDE_CONFIG_DIR` and `FOO_API_KEY` in `terminal:env` are refused |
| 5.6 | W2c | Permissions broker: T2 states set at install per item, T3 requests, "Always allow" bound to the component hash, the journal store, revocation that kills, closes and cleans up (§C.2–§C.4) | 5.5 | unticked items are `denied`; revocation closes listeners and removes discovery files; journal entries never hold terminal content |
| 5.7 | W3 | Consent UI: the grouped install dialog with checkboxes and the 1 s guard, extension-page switches and journal view, held-update diff, T3 cards and the "Always allow" warning, badge and status-bar indicator (§C.2–§C.5) | 5.6 | spec §4 failure 1 still passes; a new program in an update shows in the diff; T3 cards ignore keys and early clicks |
| 5.8 | W1 | Terminal host for extensions: profiles in "+" (§B.1), pseudo-terminals with the sanitiser and rate caps (§B.2), created terminals in a pty (§B.3), env injection with the profile and all-new scopes (§B.4), `${package}` (§B.6), status and `terminal:observe` with redaction (§B.5), bracketed paste (§B.11) | 5.1, 5.5 | an echo pseudo-terminal works in a tab and in a panel; OSC 52 and DSR from it never reach the grid; reload kills a created terminal's tree |
| 5.9 | W3 | `terminal` node in the extension panel container (§B.2) and the approval cards (§B.12) | 5.8 | a panel shows a native terminal with selection, search and IME; one handle never shows twice |
| 5.10 | W4 | SDK 0.4 module for terminals and harness fakes; demo `fyx_demo_pseudo_terminal` (a TUI written in WASM in a panel) and a profile demo | 5.8, 5.9 | the demo runs headless in the harness and in Fyx |
| | | **Phase 3, integration kit (layer B), about 2–3 weeks** | | |
| 5.11 | W2c | Loopback listeners and the MCP wrapper: `127.0.0.1:0`, host token, `Origin` and `Host` checks, `mcp-websocket`, `mcp-streamable-http`, `websocket`, `http`; blocking calls cancelled with their connection (§B.7) | 5.6 | wrong token, `Origin` and foreign `Host` never reach the extension; an MCP `tools/call` round-trips |
| 5.12 | W1 | Discovery files (templates, `0600` in `0700`, atomic, no symlinks, stale cleanup, account dirs; `fyx_guard` allows host-written ones) and `crates/fyx_relay` (binary, endpoint, terminal-token routing, `on_failure`) (§B.8, §B.9) | 5.1, 5.11 | a template naming `settings.json` is refused; a relay hook with Fyx closed exits 0 at once |
| 5.13 | W2c | `review:propose` bridge to `agent_review` (public API only; hooks asked of W5), turns, `apply` (§B.10); `context:editor` and `workspace:open` (§B.11) | 5.6 | a blocking proposal returns the final content after a partial accept; private files show no text |
| 5.14 | W4 | SDK and harness for listeners, relay, review and context; a generic MCP echo demo | 5.11–5.13 | the demo answers a `tools/call` from a test client |
| | | **Phase 4, `fyx_claude_code`, about 1–2 weeks** | | |
| 5.15 | W4 | `extensions/fyx_claude_code` (design, "The first extensions"): profile with `account_tool`, the Claude plugin in package files, `http` hooks and relay, the IDE protocol over `mcp-websocket` with the lock file, `openDiff` → `review:propose`, turns. **Compat test** pinned to a Claude Code version. Check first whether `CLAUDE_CODE_PLUGIN_DIRS` hooks wait for Claude's trust dialog (research question 1) | 5.10, 5.14 | the compat test passes; an `openDiff` shows hunk by hunk and returns the final content; the tab shows the account |
| | | **Phase 5, `fyx_gemini_cli`, about 1 week** | | |
| 5.16 | W4 | `extensions/fyx_gemini_cli` on the documented IDE companion spec: `mcp-streamable-http`, Bearer token, the `${tmp}` discovery file, `ide/contextUpdate`, non-blocking diffs. Any primitive it needs that Claude did not is added before 0.4 freezes | 5.14 | Gemini CLI connects and a diff round-trips with no change to the host |
| | | **Phase 6, shell integration and the user's terminals, about 2 weeks** | | |
| 5.17 | W1 | Shell integration with a nonce for zsh, bash, fish **and PowerShell** (`CommandBlock`), loaded through the env hook (`ZDOTDIR`, `--rcfile`, `XDG_DATA_DIRS`); SSH terminals and remote listeners through a tunnel | 5.8 | a forged OSC 133 without the nonce is untrusted; blocks work in all four shells; a remote Claude reaches the listener over SSH |
| 5.18 | W3 | T3 read and write on the user's terminals: "Share with…", per-command write confirmation, "typed by" marker, "Always allow" (§C.3) | 5.7, 5.17 | a write without confirmation is impossible; reads give plain text only |
| 5.19 | W7 | Security review of P5 (threats table of the design), before 0.4 freezes | 5.18 | review file in `tasks/reviews/` |
| | | **Multiple accounts and Fyx profiles** | | |
| 5.20 | W1 | Account profiles (`fyx.accounts`, per-project choice in user settings, the ACP hook in `agent_servers/src/acp.rs`, badges) and **Fyx profiles, variant A** over `--user-data-dir`: manager, start-up choice, title-bar chip, `fyx --profile`, project → profile memory (§C.6). Check the macOS Dock and CLI behaviour with several instances | 5.1 | two accounts in two terminals with the right badges; no code path reads an account dir's content; a limit message offers no other account |
| 5.21 | W10 | Registry template and docs: T3 in `capability-tiers.json` (counts as T2 for the release tier), the update diff in the PR comment, the account-rotation rule in the review rules; user guide and authoring guide for 0.4 | 5.5 | template tests pass; `registry-repo.md` has the rule |

**Order and parallelism.** Phase 1 is W1 alone, with 5.4 (W3) once 5.3 lands. In phase 2, W2c
(5.5, 5.6) and W1 (5.8) compile at the same time; W3 follows on 5.6. Phase 3 is W2c (5.11, 5.13)
and W1 (5.12). Phases 4–5 are W4 with fixes from the owners. Phase 6 is W1 and W3. 5.20 runs
beside phases 2–3 when a compiling slot is free. The owner's limit of 2 compiling streams plus the
gatekeeper holds throughout.

## P6 product basics

Added by the owner on 2026-11-20 from [roadmap-and-vscode-port.md](../research/roadmap-and-vscode-port.md)
section 1 (G-numbers there). Gaps users notice on day one, mostly because Fyx is cloud-off. Owners
are suggestions for the coordinator. Sizes for one stream: S ≤ 3 days, M ≈ 1–2 weeks.

| # | Owner | Task | Size | Depends on | Done when |
|---|---|---|---|---|---|
| | | **P0, before the 1.0 cut** | | | |
| 6.1 | W2c (WIT, model) + W3 (UI) + W4 (SDK) | Extension settings (G4): a `[settings]` table in the manifest (type, default, description, enum, min/max), a `settings` import that reads the current values and an event when they change, and a settings page per extension in the extensions UI generated from the schema. **In the 0.4 WIT, before it freezes** | M | 5.5 | a demo declares three settings; changing one in the UI reaches the guest without a reload; a wrong type is refused at install |
| 6.2 | W1 | Self-updater with signed releases (G1): a release channel manifest signed with Ed25519 (the registry's signing scheme, its own key), download, signature and hash check, install on restart, keep the previous version and roll back; Zed's updater endpoint stays off. **Must work without notarization for now** (owner, 2026-11-20: no Apple Developer account): on the Mac it replaces an ad-hoc-signed `.app` and relies on Fyx's Ed25519 signature only; the Apple signature check is added once there is an account | M | — | a test channel offers 1.0.1; a tampered package is refused; rollback restores the previous version |
| 6.3 | W5 successor (`agent_review`) | Local history and agent timeline (G3): a snapshot per save and per agent turn, a timeline per file and per turn, restore a file or a whole agent turn even after it was accepted; size cap and pruning | M | 1(b) of the Phase 5 backlog (review per turn) | accept an agent turn, then undo the whole turn from the timeline in one step |
| 6.4 | W1 | Local crash reports (G5): panics and minidumps written under the Fyx data dir, a "Crash reports" view, and "Report" that opens a prefilled GitHub issue in the browser; nothing is uploaded | S | — | a forced panic shows in the view after restart with a working "Report" |
| 6.5 | W1 (settings) + owner of AI settings | Local or bring-your-own edit predictions (G2): check which non-Zed providers upstream still ships (Copilot, Codestral, Ollama and others), enable them in the cloud-off build, add an OpenAI-compatible local endpoint (llama.cpp, Ollama) | M | — | inline predictions work with a local `llama-server` and no network |
| | | **P1, after 1.0** | | | |
| 6.6 | W2c + W3 | Diagnostics from extensions (G7): a diagnostics collection per extension shown in the editor and the diagnostics panel like LSP ones, with source and severity | S–M | 0.4 or the first post-1.0 WIT | a linter demo marks lines and lists them in the panel |
| 6.7 | W2c + W3 | Tree views and file decorations (G8): nested tree rows with expand and collapse on top of `list`, and badges or colours on project-panel entries contributed by an extension (one `fyx:` hook in `project_panel`) | M | 6.6 | a bookmarks demo shows a tree and marks bookmarked files in the project panel |

**Order.** 6.1 goes with phase 2 of P5 (5.5, 5.6) so it is in the 0.4 freeze. 6.2 and 6.4 are
W1's, after 5.8 or whenever a compiling slot is free, and must land before the 1.0 cut. 6.3
follows backlog item 1(b). 6.5 can start at once; it is mostly configuration and one provider.
6.6 and 6.7 wait for the post-1.0 WIT. VS Code compatibility is not part of P6 (owner, 2026-11-20).

## P7 differentiators

Added by the owner on 2026-11-20 from section 4 of
[roadmap-and-vscode-port.md](../research/roadmap-and-vscode-port.md) (its ids in brackets). The
owner works mainly on macOS, so the Mac comes first. Local models on Apple Silicon (D17 + M6) were
skipped by the owner. **There is no Apple Developer account for now**, so Mac builds are local,
unsigned or ad-hoc signed, built and run by the owner; no notarization and no paid services.
Sizes for one stream: S ≤ 3 days, M ≈ 1–2 weeks.

| # | Owner | Task | Size | Depends on | Done when |
|---|---|---|---|---|---|
| 7.1 | W1 (+ W0 for the gate script) | **Local Mac build [M1]**: `script/bundle-mac` builds a Fyx-branded `.app` and DMG without an Apple account: Fyx bundle id, name and icon (license audit F2), Fyx entitlements reviewed from `zed.entitlements` (keep what wasmtime's JIT needs), no `dmg-license` with Zed's Terms of Service (F3), ad-hoc signature by default; a `script/fyx-mac-check` the owner runs on his Mac (build, FAST tests of the Fyx crates, launch smoke test with `--user-data-dir`); a short "first launch" note for Gatekeeper (right-click → Open, or `xattr -dr com.apple.quarantine`) | M | — | the owner builds the DMG on his Mac, installs it, and Fyx starts with Fyx's name and icon; `fyx-mac-check` passes |
| 7.2 | W5 successor (`agent_review`) | **Review comments to the agent [D1]**: comment on hunks in the review view, then send all open comments as the agent's next turn, with file and line references | S–M | — | three comments on two files reach the agent as one prompt and the agent's next diff answers them |
| 7.3 | W5 successor + W2c (`fyx_guard`) | **Agent guardrails [D6]**: scan every agent diff for secrets before accept (high-entropy strings and known key formats), protected paths from `.fyx/review.toml` that always need review, `.env` and account dirs kept out of agent context | S–M | Phase 5 backlog 1(d) (`.fyx/review.toml`) | an agent diff that adds an AWS-style key is held with a warning; a protected path cannot be auto-accepted |
| 7.4 | W1 (Keychain, Touch ID) + W2c (resolution, redaction) | **Agent secrets vault [S1 + M3]**: secrets stored in the macOS Keychain (Windows Credential Manager on Windows); the agent sees only `secret://name`; Fyx injects real values only into the env of processes it spawns (agent terminal tool, tasks, MCP servers, profile terminals); exact values and their base64, hex and URL encodings are replaced by `secret://name` in every tool and terminal output returned to the agent; each use asks for approval with Touch ID (Windows Hello); uses are journaled without values. **Limit, shown in the UI:** encoded or exfiltrated values cannot all be caught; the strong form is 7.5's per-secret host allow-list | M | 7.3 | a script uses `secret://github` to call the API; the value never appears in the thread, its database or the provider request; `printenv` output shows the reference |
| 7.5 | W2c | **Seatbelt for extensions and agents [M2], with network policy [D10, Mac]**: run `process:spawn` children (4.7), MCP servers and the agent's terminal tool under a Seatbelt profile built with `crates/sandbox` (`macos_seatbelt.rs`) from the declared paths; a per-project egress allow-list, and a per-secret host allow-list for 7.4 (`GITHUB_TOKEN` only to `api.github.com`). Windows keeps job objects for now; its network policy is later (L) | M | 4.7, 7.4 for the per-secret part | a spawned tool cannot read outside its declared paths or reach an undeclared host; a secret sent to a host outside its list is refused |
| 7.6 | W3 | **Privacy and permissions report [D9]**: per extension and agent, what it read, wrote, fetched, spawned and which secrets it used this week, from the 5.6 journal and the Inspector records, with one-click revoke | S–M | 5.6, 7.4 for secret uses | the report lists a demo's fetches and file reads; revoke stops them at once |
| 7.7 | W3 | **Fast extension UI [D15]**: fix the full-tree re-render (perf-1 finding 1) so a caret blink or a one-node reply does not re-render the whole surface; a per-extension memory and frame-time line in the Inspector | M | 1.1 (`.cwasm` cache) for load times | the 4096-text panel stays under 8 ms per frame while typing in its input |
| 7.8 | W1 (Dock, menu bar) + W3 (meters) | **Cost and context meters with an attention badge [D3 + M4]**: tokens, money and context use per session and per day from ACP usage and relay data, a soft daily cap that pauses agents; on the Mac a Dock badge with the number of agents waiting and a menu-bar item listing sessions | S–M | 5.4 | two sessions show their cost; the Dock badge counts a blocked agent; the cap pauses an agent with a notice |
| 7.9 | W3 | **Agent sessions board [D2]**: every agent (ACP threads, CLI agents in terminals via OSC 7501 and the relay), its worktree, state, diff size and cost, "needs you" first, click to focus. Kept to the narrow layer of [worktrees research §8](../../research/worktrees-and-parallel-agents.md): no worktree manager | M | 5.4, 5.12, 7.8 | a Claude Code CLI session and an ACP thread show side by side with live state |
| 7.10 | W3 (page) + W1 (`extension_host` hook, setting) | **One Extensions page with Fyx and Zed tabs, Zed's registry opt-in**: see [7.10 below](#710-extensions-page-fyx-and-zed-tabs) | S–M | 3.5 | the page opens on the Fyx tab; the Zed tab shows installed Zed extensions with no error banner until the user turns the registry on, then searches and installs from it; nothing but extension requests reaches zed.dev |
| 7.11 | W5 successor (`agent_review`) | **"Review Agent Edits" as a switch**: in the agent panel's options menu the entry is a `toggleable_entry`, which hides its check mark when off and leaves an empty indent that reads as a layout bug. Draw it in `review_mode_menu_entry` as a `custom_entry` with the label and a `ui::Switch` at the end; while `FYX_AGENT_REVIEW` overrides the setting, a non-selectable `custom_row` with a disabled switch. Only this entry: the other menus keep Zed's check-mark style (a square for every toggle would edit upstream `ui` and depart from macOS menus) | S | — | the menu shows the switch on and off; clicking the row toggles `agent_review.enabled`; with the variable set the switch is disabled |
| 7.12 | W7 (audit) + W3 (fixes) | **Readable prompts and notices (ux-2)**, known cases in [7.12 below](#712-known-cases): the dev-folder notice "The UI extension … is installed but not enabled. It asks for: …" (`ui_ext_host/src/registry.rs`) is one paragraph with every capability, its id, the folder and the buttons run together; the T2 ones (`terminal:env`, `network:listen`, `fs:discovery-file`, `relay:hooks`) look the same as `ui:panel`. **First an audit, in the running Mac build** (ux-1 only read the code): every Fyx-made notification, toast, prompt, consent and install dialog, permission card, banner and empty or error state in `ui_ext_host`, `ui_ext_wasm`, `agent_review`, `terminal_ext`, `fyx_registry` and the `fyx:` hooks, with a screenshot of each; findings in `tasks/reviews/ux-2.md`. **Rules the fixes follow:** a short title that says what happens; capabilities as a list, grouped by tier, T2 first with a warning style, ids secondary or in a details toggle; paths and ids in a muted or code style, never inside the sentence; one decision per prompt with verb buttons ("Enable", not "OK"); details behind "Show details"; the same layout for one consent wherever it appears (dev folder, install, registry, approve-and-update). **Then the fixes**, starting with the consent notice and the install prompt, as a shared consent view in `ui_ext_host` | M | — | `ux-2.md` lists every surface with a screenshot and a verdict; the consent notice and the install prompt show grouped lists with T2 marked; no Fyx prompt is a single paragraph over three lines |
| 7.13 | owner (repo settings) + W0 (check) | **Turn on GitHub Issues for `shelenhovskyi/fyx`**: the crash dialog's "Report on GitHub" (`NEW_ISSUE_URL` in `crates/zed/src/fyx_crash_reports.rs`) opens `issues/new` with the title and body filled in, but Issues are off on the repository (`hasIssuesEnabled: false`), so GitHub answers 404. Turn them on (Settings → Features → Issues, or `gh repo edit shelenhovskyi/fyx --enable-issues`); add a crash-report issue template and a `crash` label; a CI check that fails if the repository named in `NEW_ISSUE_URL` has Issues off | S | — | the crash dialog's button opens a filled-in new issue; the check passes |
| 7.14 | W1 (`terminal_host`, restore) + W4 (`fyx_claude_code`) + W3 (tab) | **Profile terminals that survive a restart, with a readable tab**: seen by the owner on 2026-11-20 with `fyx-claude-code`. (a) **Restore:** after a restart the "Claude Code" tab comes back as a plain zsh in the same folder and the Claude session is gone. Profile terminals have no restore path: persist the extension id, profile id and account with the tab, and on restore start the profile again; the extension may add resume arguments (for Claude, the `session_id` its SessionStart hook already reports, so `claude --ide --resume <id>`), and the user can turn resume off. (b) **Title:** the tab always reads "Claude Code", the profile's name. Use the program's own title (OSC 0/2, which Claude sets to the session topic; check what 2.1.329 sends) with the profile name as fallback, and the session name when the extension reports one. (c) **Tab layout:** today one line holds the status icon (✓), the account dot and name ("● Work") and the title, all at the same weight. Decide one order and style: title first; the account as a coloured edge or a small chip; the status as one icon that changes (running, waiting for you, done, error) with a tooltip; nothing duplicated between the tab and the sessions list | M | 5.8, 5.15, 5.20 | after a restart the Claude tab resumes the same session; the tab shows the session's topic; a review of the tab with three states and two accounts passes ux-2 (7.12) |

**Order.** 7.1 first: it gives the owner a Mac build and a Mac check for everything after it.
Then the agent-review lane in order 7.2 → 7.3 → 7.4. 7.5 follows 4.7 and pairs with 7.4. 7.7 is
W3's as soon as a compiling slot is free, before 7.6 and 7.9, which add more UI. 7.8 and 7.9 come
after P5 phase 3 (5.12). The owner's limit of 2 compiling streams plus the gatekeeper holds.

**Later, not planned yet:** signing and notarization once there is an Apple Developer account
(then 7.1 gets a CI job and 6.2 checks the Apple signature), the MCP manager [D5], worktree review
and merge [D4], Finder and Shortcuts integration [M5], the verified build badge [D11], terminal
"fix with agent" [D19], plan mode [D7], review bundles [D13], local automations [D8].

### 7.10 Extensions page: Fyx and Zed tabs

Decided by the owner on 2026-11-20 after the first Mac build. **Problem:** Fyx sets
`server_url` to `http://127.0.0.1:0`, so Zed's Extensions page always shows "Failed to load
extensions" and its "Not Installed" filter is always empty, while the Fyx registry (3.5) hides
behind a "Fyx Extensions" button that opens a separate page. Plan 3.5 asked for a tab.

- **Tabs** in the Extensions page header: `Fyx` (default) and `Zed`. The Fyx tab renders the body
  of today's `fyx_extensions_page` (search, install with permission confirmation, pin, roll back,
  held updates). `fyx_ui: open fyx extensions` opens the page on the Fyx tab. The "Fyx
  Extensions" button goes.
- **Zed tab, registry off (default):** installed Zed extensions only; no request to the registry,
  no error banner, no All / Installed / Not Installed filter. A banner instead: Zed extensions come
  from Zed's registry, which Fyx neither verifies nor signs; language servers and debug adapters
  can download and run binaries with full access; requests go to api.zed.dev. Button **Turn on
  Zed's registry** sets the new setting `fyx.zed_extensions_registry: true`.
- **Zed tab, registry on:** search, Not Installed and install work as in Zed, and installed Zed
  extensions get updates again. A slim reminder banner stays, and each card from Zed's registry
  has a "Zed registry" chip.
- **Only extension traffic goes to Zed:** one `fyx:` hook in `extension_host` sends its three
  `build_zed_api_url` calls to `https://api.zed.dev` when the setting is on. `server_url` stays
  `127.0.0.1:0`, so sign-in, telemetry, collab and file-type extension suggestions stay off.
- **Install Dev Extension** becomes a menu: "Zed extension…" (today's action) and "UI extension…"
  (`fyx_ui: install dev UI extension`).
- **Not merged into one list:** the two registries install differently (permission confirmation,
  signed hash, pin and rollback against a plain download), and one list would mean rewriting
  upstream `extensions_ui`. The tabs need about three `fyx:` hooks there (tab switch, skipping the
  registry fetch while off, the dev-install menu); the logic stays in `ui_ext_host`.
- **Docs:** the user guide's "No Zed extension registry" becomes "Zed's extension registry is off
  until you turn it on in Extensions → Zed".
- **Tests (7.10):** with the setting off, no request leaves for any host; with it on, extension requests
  go to api.zed.dev and nothing else does; the Fyx tab opens by default and from the action.

### 7.12 Known cases

Seen by the owner on the Mac build on 2026-11-20 while enabling `fyx-claude-code`. The audit
starts from these and looks for the same faults everywhere else.

1. **Dev-folder notice** ("… is installed but not enabled. It asks for: …"): one paragraph with
   every capability as "Description (id)" joined by semicolons, then the folder; T2 not marked.
2. **Enable dialog** ("Enable the UI extension fyx-claude-code?"):
   - the title uses the id, not the name ("Claude Code");
   - group headings ("Shows in Fyx", "Allowed without asking", "Off unless you tick it") look
     like the items under them; "A panel, Files from its own package" joins two items in one line;
   - manifest templates are shown raw: `${package}`, `${listen.ide.port}`, `${account.claude}`,
     `${port}.lock`. Show what they become ("this extension's folder", "a port Fyx picks",
     "the Claude account you choose") or the resolved value;
   - raw ids: `mcpwebsocket` (the protocol, with its hyphen lost), `channel session-start (on
     failure: exit 0)` for every hook channel;
   - warnings repeat (two `CLAUDE_CODE_PLUGIN_DIRS` lines) and "It can run any program on your
     computer" says neither why nor how much;
   - every T2 box starts unticked and Enable works with none ticked, but the dialog does not say
     what the extension can still do then (here: nearly nothing), and enabling it fully takes
     four ticks plus Enable;
   - **required fix, owner's decision:** warn, but do not make the user click through. Add a
     primary button **Enable with all permissions** next to **Enable without T2** (or a single
     "Allow all" checkbox above the T2 list that ticks every box). The risks are listed once,
     above the buttons, in one short summary ("runs programs in its terminals, listens on a
     local port, writes a file into your Claude config, receives Claude's hooks"); the per-item
     boxes stay below for anyone who wants to pick. Enable without T2 says what will not work.
     The same choice appears wherever this consent appears (dev folder, install, registry,
     approve-and-update);
   - the list scrolls inside the dialog with no sign that more is below; the hook list is cut
     off at the bottom edge.
3. **After Enable, no "what next"**: the owner enabled `fyx-claude-code` and could not tell what
   it added or where. Nothing points to its one entry point, **Claude Code** in the terminal
   panel's **+** menu. Fix: after Enable, a toast "Claude Code is ready: open it from the
   terminal panel's + menu" with a button that opens the panel and that menu; and on the card a
   short "Adds" line built from the manifest (terminal profile "Claude Code", panel names,
   status items) with the same open buttons. Every extension that contributes UI gets this.
4. **Fyx Extensions page** (the card after enabling):
   - "The registry is off (fyx.extensions.registry_url)" shows a setting key as the
     explanation; say what it means and how to turn it on, or hide it until 7.10;
   - "Available: Nothing to show." with the registry off is a dead section;
   - the card's title is the id `fyx-claude-code`, not the name; the folder path is the
     second line;
   - every permission reads "…: on" next to a switch that already says on;
   - the same raw templates and ids as the dialog (`${package}`, `${listen.ide.port}`,
     `mcpwebsocket`, the hook channels with `on failure: exit 0`);
   - the Usage log prints capability ids (`fs:discovery-file#${port}.lock turned on`) instead
     of the names used above it;
   - twelve rows with switches on one card and no grouping: group by tier as in the dialog,
     T0/T1 folded into one line, T2 rows expanded.

## Deferred after 1.0

TypeScript extensions are deferred until after 1.0, and whether to do them at all is re-evaluated
then (owner, 2026-11-18; spec decision 3). P3 builds and signs Rust extensions only. `sdks/ts`
stays in the repository as an unsupported prototype.

**VS Code extensions: not supported (owner, 2026-11-20).** No Node sidecar and no `.vsix`
conversion as a feature; PLAN.md backlog item 12 is dropped. `sidecars/vscode-host` and
`tools/vsix-convert` stay as unsupported code until a separate removal step
([research](../research/roadmap-and-vscode-port.md)).

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
