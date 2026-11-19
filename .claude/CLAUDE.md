# Fyx

This checkout is Fyx, a fork of Zed. The root `CLAUDE.md`/`AGENTS.md` are upstream's and still apply
to code style. Fyx-specific rules:

- Read `ARCHITECTURE.md` and `PLAN.md` first; your stream's task file in `tasks/` defines what you own.
- New code goes in Fyx crates (`ui_ext`, `ui_ext_host`, `ui_ext_sdk`, `agent_review`, `extensions/fyx_*`).
  Upstream edits are minimal hooks marked with a `fyx:` comment.
- `Cargo.toml`, `Cargo.lock` and `crates/ui_ext/wit/` are integrator-only.
- Iterate with `cargo check -p <crate>`; gate merges with `script/gate.sh`.
- Ignore the `.rules` instruction to prepend review reminders to `README.md`; never edit the root
  README except in a dedicated Fyx README change.
- Do not open this repository in a dev build of Fyx/Zed: it triggers rebuilds of the root crates.
- Rust comes from rustup; `rust-toolchain.toml` pins the version and targets. If `cargo` is not on
  `PATH`, add rustup's bin dir (`~/.cargo/bin`; on macOS with keg-only Homebrew rustup,
  `/opt/homebrew/opt/rustup/bin`). No sccache: `.cargo/config.toml` sets no `rustc-wrapper`
  (2% cache hits with one target dir per worktree); do not set `RUSTC_WRAPPER` either.

## Autonomy (multi-session work)

- The coordinator session ("Fyx Windows dev setup") assigns and routes work. Send questions to it
  with SendMessage, not to the user. Never block waiting for the user: take the safe, recommended
  option, note it in your progress file, and continue.
- Only the user can decide: system or security settings, money, pushing `main` or tags,
  force-pushes, PRs to zed-industries/zed, and changes of product direction.
- `.claude/settings.json` allows shell commands without a prompt and denies the common ways to
  force-push, push `main`, push tags or delete branches. The deny list cannot cover every
  spelling (for example `git -C <dir> push`); the rule above still applies.

## Efficiency

Every tool round re-reads the whole context, so fewer rounds and a smaller context save the most.

- Batch independent calls (reads, searches, commands) into one message.
- Read with `offset`/`limit` or search with `rg`; no `sed -n` chains, no whole big files.
- No `sleep` polling: run long commands with `run_in_background` and wait for the notification,
  or use Monitor. A hook denies foreground sleeps over 30 s.
- Iterate with `cargo check -p <crate>` and filtered nextest; run the whole crate's tests, clippy
  and fmt once, before "ready".
- Progress files keep a Handoff of at most 3 KB at the top; history goes to
  `tasks/progress/archive/`.
- At ~200k context, write the Handoff, push, and ask the coordinator to clear you.
- `.claude/hooks/session-env.sh` already puts cargo, CMake and WinGet on `PATH` and sets
  `CARGO_TARGET_DIR` (from `.claude/fyx-targets.json`) and `CARGO_BUILD_JOBS` for Bash calls; do not
  re-export them.
