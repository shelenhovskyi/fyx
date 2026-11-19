# Fyx sprint plan (36 hours)

Goal: a narrow vertical slice. One WIT world, 5–7 primitives, a dock panel showing 1–2 demo
extensions, the loop "click → WASM callback → new patch → repaint", a rebranded cloud-free build, and
basic hunk-level approval of edits from an ACP agent. Behind a feature flag, verified on one OS: the
dev machine is Windows 11 since 2026-11-17 (Phase 0 was done on macOS).

Out of scope: permission prompts, ABI stability, frame budgets, sidecar transport, a second SDK
language, builds for a second OS, notarization, marketplace.

## Status

| Phase | Hours | What | State |
|---|---|---|---|
| 0 | 0–2 | Freeze interfaces: fork at stable tag, warm build, this plan, WIT, crate skeletons | done (2026-11-17) |
| 1 | 2–14 | Parallel workers W1–W8 | done (2026-11-17): W1–W5 and W8 merged on `fyx/main` (`2e91298e4a`); W2 and W8 sessions archived; reviews in `tasks/reviews/` |
| 2 | 14–20 | First integration, run the app, judge pass, fresh restart of drifting agents | done (2026-11-17): the dev build runs the demo panels (W3 user check closed); W5's end-to-end review check passed in the dev build (`b2e90dfdea`); judge passes integration-1 to 3 |
| 3 | 20–30 | Second wave: agent review, decorations, polish | in progress: W6 decorations; W5 phase B merged |
| 4 | last 6–8 | Integration only, manual testing in the running editor, human review | pending |
| 5 | after the sprint | Phase 5 backlog (below) | planned |

## Phase 5 backlog

After the sprint, in this order within each owner. The headline is W5's review policy. Open review
findings that belong to an item are noted next to it (`tasks/reviews/integration-3.md`).

| # | Owner | Item | Notes |
|---|---|---|---|
| 1 | W5 | **Review policy (headline).** Ship it in four steps, each building on the previous one: (a) a review-mode setting, replacing the `FYX_AGENT_REVIEW` env var; (b) review per agent turn, not per write; (c) detection of writes that bypass the gate (disk changes with no `fs/write_text_file`); (d) project rules in `.fyx/review.toml` (paths that always need review or never do) | First, gate the script driver to dev builds (integration-3 #1). Then surface failures of host-applied hunks (#3) |
| 2 | W3 | Install UX: an installed-extensions folder and an install action, replacing `FYX_UI_DEV_EXTENSIONS` | integration-3 #2 |
| 3 | W3 | Hot reload of a UI extension's component | |
| 4 | W3 | Keybindings for panels (`ToggleExtensionPanel` per surface) | |
| 5 | W3 | A real `diff-view` on Zed's diff editor, replacing the placeholder | |
| 6 | W3 | Renderer fixes from the cycle-3 review: the `list-rows` loop after a resync, stale row clicks, the remount counter, Retry on a disabled instance, input lost on remount | integration-3 #8–#10 |
| 7 | W1 | Remaining brand gaps: the CLI binary and its `Zed.exe` lookup, the clap name, the installer | integration-3 #5 |
| 8 | W1 | Review the cloud-off defaults (`assets/settings/fyx.json`) and measure a `cloud` cargo feature that compiles out Zed's cloud crates | Move `fyx_default_settings` and `fyx_url_to_internal` out of upstream files (integration-3 #4) |
| 9 | W2c (later) | Move `crates/extension_host/src/wasm_host/fyx_ui*` into a Fyx-owned crate, no hook in `extension_host` (done: `crates/ui_ext_wasm`, W3) | Also integration-1 #11 and F2 |
| 10 | W0 | `fyx:ui` 0.2 design as one world: `request-tick` and `modal` first, then the rest of `tasks/wit-0.2.md` | Add `push-update -> result` (integration-3 #6) |
| 11 | W0 | Own CI: `fyx_*` GitHub workflows (gate, lockfile check, licenses, demo build). All 46 upstream Zed workflows are disabled on the fork | Mark the `Cargo.toml` members (integration-1 #13) |
| 12 | W8 | ~~VS Code adapter, Rust side: launch the sidecar under `--permission`, bridge JSON-RPC to the `ui_ext` tree and answer `push-update`~~ **Dropped (owner, 2026-11-20).** Fyx does not support VS Code extensions at all | Reason: [roadmap-and-vscode-port.md](docs/fyx/research/roadmap-and-vscode-port.md): the extensions users miss are Microsoft-licensed, LSP-based or webview-based; Node `--permission` is no sandbox; 5–15× memory and slower start. `sidecars/vscode-host` and `tools/vsix-convert` are unsupported; removal is a separate step |
| 13 | W5 + W3 | One shared end-to-end dev driver, built from W5's review script driver, that also drives `fyx:ui` panels. Compiled only into dev builds | Replaces per-stream drivers |
| 14 | W7 | Fix the author docs' drift (D1–D10), then keep `docs/fyx/` checked against each merge | integration-3, docs section |
| 15 | W1, W2c, W3, W4, W5 | **P6 product basics** (owner, 2026-11-20): extension settings in the 0.4 WIT (6.1), self-updater (6.2), local history and agent timeline (6.3), local crash reports (6.4), local/BYO edit predictions (6.5); after 1.0, extension diagnostics (6.6) and tree views with file badges (6.7) | [docs/fyx/platform/plan.md P6](docs/fyx/platform/plan.md#p6-product-basics); 6.1 before the 0.4 freeze, 6.2–6.5 before 1.0 |

## Rules for every agent

- Work only in the directories your task file lists under **Owns**. Anything else: note it in your
  progress file and stop.
- `Cargo.toml` (workspace), `Cargo.lock` and `crates/ui_ext/wit/` belong to the integrator (W0).
  Need a dependency or a WIT change? Ask W0.
- Iterate with `cargo check -p <crate>`. Only W0 builds the `zed` binary.
- Own worktree, own `CARGO_TARGET_DIR`. Seed it from the warm `target/`, never start cold: `cp -c`
  (APFS clone) on macOS; on Windows `robocopy target D:\fyx-targets\<stream> /E /MT:16` (a full
  copy, NTFS has no clones).
- Keep `tasks/progress/<stream>.md` current; log failures on one line starting with `ERROR`.
- Reference implementations: `ProjectPanel` and `AgentPanel` show how to implement `Panel`.
- Green tests are not done. Done is the behaviour seen in the running editor.

## Workstreams

| Stream | Task file | Depends on |
|---|---|---|
| W0 Integrator/CI | [tasks/W0-integrator.md](tasks/W0-integrator.md) | — |
| W1 Rebrand and de-cloud | [tasks/W1-rebrand.md](tasks/W1-rebrand.md) | W0 |
| W2 WIT and host bindings | [tasks/W2-wit-bindings.md](tasks/W2-wit-bindings.md) | frozen WIT |
| W3 Renderer and containers | [tasks/W3-renderer.md](tasks/W3-renderer.md) | W2 types |
| W4 Guest SDK and demos | [tasks/W4-sdk-demos.md](tasks/W4-sdk-demos.md) | WIT |
| W5 Agent change review | [tasks/W5-agent-review.md](tasks/W5-agent-review.md) | W3 container |
| W6 Decorations (stretch) | [tasks/W6-decorations.md](tasks/W6-decorations.md) | W3 |
| W7 Judge/reviewer/docs | [tasks/W7-judge.md](tasks/W7-judge.md) | — |
| W8 VS Code adapter (dropped 2026-11-20) | [tasks/W8-vscode-adapter.md](tasks/W8-vscode-adapter.md) | tree schema (W2), JSON-RPC can be mocked |

Scale the non-compiling streams (W7, W8, demo authors, `ui_ext` unit tests) to use spare capacity;
compiling streams (W3, W5, W6) are capped by disk and CPU.

Disk budget (Windows dev machine, Ryzen 5 7500F 6c/12t, 32 GB RAM): warm `target/` 27 GB, 640+ GB
free on D:, so CPU is the limit: run 2 compiling streams at once, 3 at most. Earlier, on macOS: the
warm `target/` was 24 GB and the machine had 35 GB free after it. APFS clones share
blocks until rebuilt, but each compiling worktree still grows by several GB, so free disk before
running more than 2–3 compiling streams at once.

## Open decisions for the owner

- Check github.com/fyx (fallbacks `fyx-editor`, `fyxdev`, `getfyx`) and domains fyx.dev, fyx.sh,
  getfyx.com.
- Trademark: several US FYX marks exist in classes 9 and 42; talk to a trademark lawyer before any
  commercialisation.
- Whether the build may talk to Zed's extension registry at all (Zed ToS apply).

**Decided:** Fyx does not support VS Code extensions: no Node sidecar, no `.vsix` import or
conversion as a feature (owner, 2026-11-20). Backlog item 12 is dropped; the W8 code stays in the
repository, unsupported, until a separate removal step. Research:
[roadmap-and-vscode-port.md](docs/fyx/research/roadmap-and-vscode-port.md).

**Decided:** the Fyx extension registry repo is `shelenhovskyi/fyx-extensions`, next to `fyx`
(owner, 2026-11-17). It is created in Platform 1 task 3.1, not before.
