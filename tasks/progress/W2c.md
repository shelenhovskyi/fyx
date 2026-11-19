# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, twelfth context)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c (push every ready before reporting it).
- **5.12 relay, extension side (§B.9), 38a8d578db:** `ui_ext_wasm/src/relay.rs` RelayHandle;
  `UiExtension::relay_request(channel, terminal, argv, stdin, RelayReply) -> Result<u64,
  String>`, `relay_channels()`, `relay_granted()`; `LoadOptions` `relay_channels`
  (load_checked) and `relay_binary` (`${relay}` in terminal:env). A dropped reply means
  on_failure. W3 has the names and the glue recipe against W1's 9a28af71e0 API.
- **Gap 8 (W4), f847ec1f86:** `ListenerEvent::McpInitialized(McpSession)` on
  `notifications/initialized`, once per connection, with clientInfo. The WIT
  `mcp-initialized(mcp-session)` is proposed to W0. Until it is staged, `fyx_ui_v0_4.rs`
  maps it to `None`; then map it there, add a probe report and a tests_v0_4 test. W10 has
  the §B.7 note.
- **fyx_guard_core (7aa8211636):** root `Cargo.toml` member and workspace dep and
  `Cargo.lock` are local only until W0 merges them.
- **Local-only, never commit:** fyx_ext_harness v0_3.rs/v0_4.rs arms `RelayRequest` => None
  and `McpInitialized` => unreachable (W0 has the diffs).
- **Told W3:** ReviewHost, ContextHost, close(ids), DiagnosticsSource, relay hook contracts.
- **Gotchas:** prefix every cargo command with `CARGO_TARGET_DIR=D:/fyx-targets/w2c`. Bash
  heredocs break here: write Python scripts with the Write tool. `cargo fmt` skips
  `t2_shared.rs` and the probe fixture (use `rustfmt --edition 2024`). Close listeners in spec
  order. T1 calls go through the broker's `check`. io tests: permission changes arrive on the
  io channel too. Loading the same id shuts the old instance down.
  `test_reload_kills_the_process_tree` (0.3) flakes under full-crate load and passes alone.
