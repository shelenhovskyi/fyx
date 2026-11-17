# Fyx

This checkout is Fyx, a fork of Zed. The root `CLAUDE.md`/`AGENTS.md` are upstream's and still apply
to code style. Fyx-specific rules:

- Read `ARCHITECTURE.md` and `PLAN.md` first; your stream's task file in `tasks/` defines what you own.
- New code goes in Fyx crates (`ui_ext`, `ui_ext_host`, `ui_ext_sdk`, `agent_review`, `extensions/fyx_*`).
  Upstream edits are minimal hooks marked with a `fyx:` comment.
- `Cargo.toml`, `Cargo.lock` and `crates/ui_ext/wit/` are integrator-only.
- Iterate with `cargo check -p <crate>`; gate merges with `script/gate.sh`.
- Do not open this repository in a dev build of Fyx/Zed: it triggers rebuilds of the root crates.
- Rust lives at `/opt/homebrew/opt/rustup/bin` (keg-only Homebrew rustup); prepend it to `PATH`.
