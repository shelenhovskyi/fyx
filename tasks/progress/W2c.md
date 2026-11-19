# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, fifth context clear)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c, pushed at 409504e1b2. Ready at W0 (gate with W3's
  host arms and W4's 1068bcf7ff): 4ecc5ae40e WIT `approvals.ask(tool, …)`; 7862b91cab approvals
  runtime; 3b26fdd9c5 `Props::Terminal`; 4e89dc1e8d 5.6 wasm side (not yet announced "ready");
  409504e1b2 `${package}` needs `package:files`.
- **W3 must add arms** in ui_ext_host: registry.rs `HostEffect::AskApproval`, and
  `Props::Terminal` in decorations.rs:706, inspector.rs:92, view.rs:248, view.rs:924 (told).
- **5.6 wasm side done** (`permissions.rs` + `consent_items.rs` + `journal.rs`): ConsentItem/key,
  ItemLine, consent_items, consent_warnings, consent_diff (granted lines vs new manifest; T1/T2
  held; the Denied-widened rule is the broker's), T3Capability/T3Prompt/T3Answer (terminal id is
  `HostTerminalId = u64` until W1 agrees), `item_state`/`t3_request` defaults, `Journal` sink in
  `LoadOptions::journal` (no serde_json in ui_ext_wasm, so the file store goes to ui_ext_host).
- **Next:**
  1. 5.6 host side in ui_ext_host `permissions.rs`/`consent.rs` per `W2c-broker-api.md`, plus
     `ui_ext_host::journal::JournalStore` (JSONL, 2000/30 days, `read`/`clear`, implements
     `Journal`). Then tell W3 the commit and that JournalStore lives in ui_ext_host.
  2. T3 wiring with W1 (TerminalHost read/write; `world_capabilities` V0_4 gets terminal:read/
     write; update the `terminal:read` refusal in tests_v0_3 `test_v0_4_manifest_tables`).
  3. `reason` on `[[fyx_ui.processes]]` and `[fyx_ui.network]` (0.4 only; W4 parses processes
     too), then feed it into `consent_items`.
- **Gotchas:** harness local patch: `sh <scratchpad>/harness_patch.sh` (recreate from the
  arms above if the scratchpad is gone), revert with `git checkout -- crates/fyx_ext_harness`;
  `limits_match` fails until W4's MAX-0.4 arm lands (known). Targets `E:/fyx-targets/w2c` and
  `w2c-light`. `cargo fmt` skips `t2_shared.rs`: `rustfmt --edition 2024` it. Edit scripts:
  Write a python file to the scratchpad; never make edits "skip if applied" (it doubled them).
