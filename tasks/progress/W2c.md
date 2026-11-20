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

## Handoff (2026-11-20, project thread)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c. Ready 9949aa4679 (FAST green): §E/§F runtime
  (publish-diagnostics, decorate-files, LoadOptions.diagnostics_publisher/file_decorations,
  MarksHandle, revoke) and §F.1 tree rows; W5's harness merged, limits only in ui_ext.
- **7.5 Seatbelt (in progress, after 9949aa4679):** `ui_ext::seatbelt` (deny-default profile,
  pure, tested on every OS) and `ui_ext_wasm/src/spawn_sandbox.rs`: on macOS process:spawn
  children run under `sandbox-exec -p`; read system + toolchain + worktree, write worktree
  (not `.git/hooks`, `.zed`, ...) + own TMPDIR; network only via an http_proxy for the
  declared `[fyx_ui.network] domains` while network:fetch is granted (revoke kills). Real
  sandbox tests are `cfg(target_os = "macos")`: the owner runs them on the Mac. Mac code was
  type-checked on Windows by temporarily swapping the cfg.
- **Open for 7.5:** MCP servers and the agent terminal tool (upstream `crates/agent`
  sandboxing, read restriction) and the per-secret host list (needs 7.4) are not done;
  per-project egress list (user setting) not done.
- **Open (others' crates):** set_account_dirs host glue (W1/W3), relay host glue (W3),
  fyx_guard_core Cargo lines (W0), W3's renderer for row chevrons and OpenSettings.
- **Gotchas:** Bash heredocs mangle backslashes: use Write/Edit or a script file. `cargo fmt`
  skips `t2_shared.rs` and the probe fixture. `test_reload_kills_the_process_tree` flakes
  under load.
