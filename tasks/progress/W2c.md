# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, tenth context)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c (push every ready before reporting it). W0 has up
  to 83f07c47e4 (5.11 wiring, 5.13 context).
- **Ready after that:** 50171cd30b §B.8 typed discovery content for W1 (`DiscoveryValue` tree,
  `worktrees.joined`, `platform.windows`); f4cedd3b96 cherry-pick of W0's 442b742a4e
  (withdraw -> result<string>; it was not on origin/integration yet); bcadfde5fb 5.13 review:
  `proposals::ReviewHandle` + `wit_review::Host`, LoadOptions `review`, process-unique ids,
  outcome once and never after withdraw, revoke/drop -> `ReviewHost::close(ext, ids)` (replaced
  close_all). Tests `test_v0_4_review`, `test_v0_4_review_rules_revocation_and_reloads`.
- **Told:** W3 the ReviewHost and ContextHost contracts (bridge in ui_ext_host is W3's), then
  the close(ids) change. agent_review gaps W3 may hit (list for W5 via the coordinator):
  extension review source with an outcome callback, review even when review mode is off
  (`review_write` short-circuits), current text on close, unified-diff apply.
- **Next:** whatever the coordinator sends. 5.13 runtime side is done.
- **Local-only, never commit:** fyx_ext_harness v0_3.rs/v0_4.rs arms `IoEvent::Listener`,
  `Context`, `ReviewDecided` => None (drop once W0's arms are merged here).
- **Gotchas:** target `D:/fyx-targets/w2c`. Python via Write-tool scripts. `cargo fmt` skips
  `t2_shared.rs` (use `rustfmt --edition 2024 --check`). Close listeners in spec order. T1 calls
  go through the broker's `check`, not `state`. io tests: permission changes arrive on the io
  channel too. Loading the same id shuts the old instance down.
  `test_reload_kills_the_process_tree` (0.3) flakes under full-crate load, passes alone.
