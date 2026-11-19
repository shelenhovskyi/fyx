# Claude project instructions

The instructions of the Fyx project on claude.ai, saved before the project was deleted.

```
Reply to the user in Ukrainian. Code, commits, PRs and repo docs stay in English.

Fyx is a fork of Zed (Rust, GPUI) adding a sandboxed WASM UI API for extensions (WIT world fyx:ui, wasmtime, wasm32-wasip2) and agent change review. Dev machine: Windows 11 (D:\projects\fyx, D:\projects\fyx-extensions).

Repos:
- shelenhovskyi/fyx: the editor. Default branch fyx/main; worker branches merge into `integration` first. Before work, read .claude/CLAUDE.md, ARCHITECTURE.md, PLAN.md and your stream's tasks/W*.md; progress files live in tasks/progress/ (Handoff at the top, max 3 KB).
- shelenhovskyi/fyx-extensions: the extension registry (TOML entries in extensions/, policy/, keys/, stdlib Python scripts + GitHub workflows). Seeded from fyx's tools/fyx-extensions-template/; design in fyx docs/fyx/platform/registry-repo.md. Change the template in fyx and the registry together.

Rules (fyx):
- New code goes in Fyx crates (ui_ext, ui_ext_host, ui_ext_sdk, ui_ext_wasm, agent_review, extensions/fyx_*). Upstream Zed edits are minimal hooks marked with a `fyx:` comment.
- Cargo.toml, Cargo.lock and crates/ui_ext/wit/ are integrator-only (W0).
- Follow the root CLAUDE.md Rust style (no unwrap, no `let _ =` on fallible calls, no mod.rs, full-word names), EXCEPT: never add the README.md review-reminder lines.
- Iterate with `cargo check -p <crate>` and filtered nextest; lint with ./script/clippy. Gate merges with script/gate.sh (FAST=1 for integration; FULL=1 only before pushing fyx/main). Never merge red.
- Do not open the repo in a dev build of Fyx/Zed.
- Registry: run `python -I -m unittest` in scripts/; full pipeline via scripts/dry-run.sh. Never commit real keys.

Git/PRs: work on the claude/* branch given to the thread. Only the user may push main or tags, force-push, open PRs to zed-industries/zed, or change product direction. PR titles: imperative, capitalized, no `fix:` prefixes; body ends with "Release Notes:" and one bullet (- N/A if not user-facing).
```
