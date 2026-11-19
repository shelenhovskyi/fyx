# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, ninth context clear)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c. W0 merged eb33909735 (1bd717ba64), staged the
  Listener harness arms (072f0c8833) and `review.withdraw -> result<string, host-error>`
  (442b742a4e); FULL gate on 442b742a4e was running.
- **Ready, not yet at W0:** d6f0d4b8c6 5.11 wiring. `listeners::ListenersHandle` (open on
  `activate_terminals`, refresh on any `network:listen*`/`fs:discovery-file*` change via
  `resend_integration`, reload takeover keyed by (observer ptr, ext id), tools kept across
  close); the `wit_listeners::Host` impl; LoadOptions `listeners`, `discovery_files`,
  `listener_observer`; `${listen.*}` in created terminals' env; MCP 2025-11-25 first. Tests:
  `test_v0_4_listeners`, `test_v0_4_listener_grants_and_reloads`, `mcp_initialize_2025_11_25`.
- **Told:** W1 the observer contract (opened/closed semantics, reload order, profiles resolve
  `${listen.*}` from OpenListener); W3 to pass `listener_observer` in the registry.
- **Next: 5.13** (B.10 review, B.11 context). Plan: model traits in `ui_ext` (`review`:
  propose/withdraw/begin_turn/end_turn + ReviewDecided sink; `context`: watch/open/
  to_file_ref/absolute_path + ContextChanged), IoEvent cases, LoadOptions hosts, WIT impls in
  fyx_ui_v0_4.rs, LINKED_INTERFACES. Bridge over `agent_review` lives in `ui_ext_host` (W3's
  crate): ask the coordinator who writes it. Coordinator (b): `context.to-file-ref` must
  accept `file:///C:/…`, `file://C:\…` and backslash paths, a test each. Build `review` on the
  staged withdraw shape (442b742a4e) once it is on fyx/main.
- **Local-only, never commit:** `IoEvent::Listener(..) => None` in fyx_ext_harness
  v0_3.rs/v0_4.rs (drop them once 072f0c8833 is merged here).
- **Gotchas:** target `D:/fyx-targets/w2c`. Write python scripts with the Write tool, not
  heredocs. `cargo fmt` skips `t2_shared.rs`. HashMap order: close listeners in spec order.
