# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, eleventh context)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c (push every ready before reporting it). W0 has up
  to 83f07c47e4; 50171cd30b, f4cedd3b96, bcadfde5fb (5.13 review) are in W0's FAST batch.
- **5.13 diagnostics (§B.11, W10's 824b124aea):** 31bf4de62c model + runtime:
  `ui_ext::context::DiagnosticsSource` (W3 implements it in ui_ext_host; contract sent to W3
  2026-11-19), `diagnostics_snapshot` (errors first, 500 cap, 2 KiB messages), ContextHandle
  `diagnostics`/`watch_diagnostics` (300 ms debounce here, private and outside files dropped,
  >200 → empty list, reload takeover, revocation stops). LoadOptions `diagnostics_source`.
  The next commit adds the `wit_context::Host` methods, `diagnostics-changed`, probe commands
  `diag`/`diag-all`/`watch-diag` and tests `test_v0_4_diagnostics*`. It needs W0's staged WIT:
  the spec block in `context` plus `use types.{file-ref};` in `io` for
  `diagnostics-changed(list<file-ref>)`. Until it is on origin/integration, the WIT edit in
  `crates/ui_ext/wit/since_v0.4.0/fyx-ui.wit` is **local only, never commit** (discard it,
  then merge integration and re-run the tests).
- **Local-only, never commit:** fyx_ext_harness v0_3.rs/v0_4.rs arms `IoEvent::Listener`,
  `Context`, `DiagnosticsChanged`, `ReviewDecided` => None (drop once W0's arms are merged).
- **Told W3:** ReviewHost, ContextHost, close(ids), DiagnosticsSource contracts.
- **Gotchas:** prefix every cargo command with `CARGO_TARGET_DIR=D:/fyx-targets/w2c`. Bash
  heredocs break here: write Python scripts with the Write tool. `cargo fmt` skips
  `t2_shared.rs` (use `rustfmt --edition 2024 --check`). Close listeners in spec order. T1
  calls go through the broker's `check`. io tests: permission changes arrive on the io channel
  too. Loading the same id shuts the old instance down.
  `test_reload_kills_the_process_tree` (0.3) flakes under full-crate load, passes alone.
