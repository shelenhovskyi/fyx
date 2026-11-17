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
| 1 | 2–14 | Parallel workers W1–W8 | pending |
| 2 | 14–20 | First integration, run the app, judge pass, fresh restart of drifting agents | pending |
| 3 | 20–30 | Second wave: agent review, decorations, polish | pending |
| 4 | last 6–8 | Integration only, manual testing in the running editor, human review | pending |

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
| W8 VS Code adapter | [tasks/W8-vscode-adapter.md](tasks/W8-vscode-adapter.md) | tree schema (W2), JSON-RPC can be mocked |

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
