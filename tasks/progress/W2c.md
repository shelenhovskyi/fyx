# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, tenth context)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c. W0 merged eb33909735; staged the Listener harness
  arms (072f0c8833) and `review.withdraw -> result<string, host-error>` (442b742a4e).
- **Ready, not yet at W0:** d6f0d4b8c6 5.11 wiring (ListenersHandle, `wit_listeners::Host`,
  LoadOptions listeners/discovery_files/listener_observer, `${listen.*}`, MCP 2025-11-25) and
  f0d1640700 5.13 context: `ui_ext::context` (ContextHost, `parse_tool_path` for
  `file:///C:/`, `file://C:\`, backslash paths; `file_ref_in` deepest root), runtime
  `editor_context::ContextHandle` (privacy filter, 64 KiB cap, watch takeover on reload, focus
  rule via `ListenersHandle::requested_within`), `wit_context::Host`, LoadOptions `context`;
  `ui_ext::review` model only (ReviewHost, Proposal, ReviewOutcome); IoEvent `Context` and
  `ReviewDecided`. Tests `test_v0_4_editor_context`, `..._rules_and_reloads`, context.rs units.
- **Next: 5.13 review runtime:** handle in ui_ext_wasm (ids, 64 open, 8 MiB, labels 256 B,
  protected paths via `ui_ext::protected`, 1024 turn paths), LoadOptions `review`,
  `wit_review::Host` (withdraw on the 442b742a4e shape: merge fyx/main once it lands),
  revoke/drop -> close_all, probe commands + tests. Bridge over agent_review in ui_ext_host:
  default W3 writes it (coordinator not yet answered).
- **Local-only, never commit:** fyx_ext_harness v0_3.rs/v0_4.rs arms `IoEvent::Listener`,
  `Context`, `ReviewDecided` => None (drop once W0's arms are merged here).
- **Gotchas:** target `D:/fyx-targets/w2c`. Python via Write-tool scripts. `cargo fmt` skips
  `t2_shared.rs` (use `rustfmt --edition 2024 --check`). Close listeners in spec order.
  `test_reload_kills_the_process_tree` (0.3) flakes under full-crate load, passes alone.
