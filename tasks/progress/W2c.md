# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## API note: account profiles (5.20, §C.6)

- Settings and resolver (W1's, `settings_content/src/fyx.rs`):
  - `FyxAccountsSettingsContent::accounts()`
  - `account(tool, name)` for a profile
  - `project_account(tool, roots)` / `project_accounts(path)` for a project
  - each gives a `FyxAccount {name, tool, env, dir, color}`; `env_entry()` is the one
    `(env name, dir)`.
- Extension runtime: `ui_ext::terminal::AccountDirs` (`dirs()`, `env_names()`,
  `account_dir(tool, cwd)`), set once with `ui_ext_wasm::set_account_dirs(Some(..))`.
  - It resolves `${account.<tool>}` for terminal:create.
  - It drops user tools' env names from that env.
  - It protects account dirs: no cwd, proposal, diagnostic, context file or absolute path in
    one.
  - `ui_ext::terminal::in_account_dirs(path, dirs)` compares paths as written and as resolved.

## Handoff (2026-11-20, final before the move to a Claude Project)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c. Last ready 9d0e1e6365 (contains 768aa26cdd),
  sent to W0; it merges integration 1a730599fa.
- **Done:**
  - 5.12 relay, extension side (§B.9, 38a8d578db): `ui_ext_wasm/src/relay.rs` RelayHandle;
    `UiExtension::relay_request / relay_channels / relay_granted`; `LoadOptions`
    `relay_channels`, `relay_binary` (`${relay}`). A dropped reply means on_failure.
  - Gap 8 (f847ec1f86, 9d0e1e6365): `ListenerEvent::McpInitialized(McpSession)` once per
    connection after `notifications/initialized`; WIT and mapping staged by W0; probe report
    `mcp-initialized <listener> <connection> <name> <version>` in `test_v0_4_listeners`.
  - 5.20 extension side (768aa26cdd): see the API note; test `test_v0_4_account_profiles`
    (temp dirs only). The model, presets, resolver, ACP hook and rule-5 lists are W1's
    4637d43836.
- **Open (others' crates):**
  - Host glue for `set_account_dirs` (W1/W3, ~20 lines). Recipe in the notes folder,
    `W2c-account-dirs-glue.md`. Until it lands, account-dir protection and `${account.*}`
    are inactive.
  - W3's workspace-edit host should also refuse files in account dirs (edits apply
    host-side; `in_account_dirs` or fyx_guard's `account_dir` rule).
  - Relay host glue (W3) against W1's `terminal_host` relay API (9a28af71e0).
  - fyx_guard_core (7aa8211636): root `Cargo.toml` member and `Cargo.lock` need W0.
- **Next steps:** none assigned. Take new items from the Coordinator. After a WIT change,
  check `fyx_ui_v0_4.rs` mappings, the probe fixture and `LINKED_INTERFACES`.
- **Gotchas:** prefix every cargo command with `CARGO_TARGET_DIR=D:/fyx-targets/w2c`. Bash
  heredocs break here: write Python scripts with the Write tool. `cargo fmt` skips
  `t2_shared.rs` and the probe fixture (use `rustfmt --edition 2024`). Close listeners in spec
  order. T1 calls go through the broker's `check`. io tests: permission changes arrive on the
  io channel too. Loading the same id shuts the old instance down.
  `test_reload_kills_the_process_tree` (0.3) flakes under full-crate load and passes alone.
