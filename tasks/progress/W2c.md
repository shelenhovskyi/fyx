# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, fourth context clear)

- **Branch:** claude/fyx-w2c-host-0-2-cbde2c, pushed at 54df51e48b.
- **At W0:** 0e7d6a00bb (5.5 step 3) and 54df51e48b (the 0.4 import side) go in one gate with
  W4's harness fix. The harness needs the two `V0_4_0` arms, `limits_match` (MAX 0.4) and
  `net::IoEvent::Terminal(_) => None` in `fyx_ext_harness/src/v0_3.rs` `io_event`.
- **Done in 54df51e48b:** `fyx_ui_v0_4.rs` (bindgen, `terminals` Host); the 0.3/0.4 shared
  imports in `t2_shared.rs` (`include!`d); `terminals.rs` (checks; a sink to
  `IoEvent::Terminal`; close_all on drop and on revoking create/env); fixture `probe_v0_4`;
  `tests_v0_4.rs`. `terminal` nodes are refused until 5.7; T3 calls answer `not-permitted`.
- **Next:**
  1. 5.6, the broker. I sent W3 (local_31b10de5-…) an API sketch on 2026-11-19 and am waiting
     for its OK; check for its reply first. In the sketch:
     - in ui_ext_wasm: `Tier::T3`; `ConsentItem` {Capability, Program{capability,name},
       Listener, DiscoveryFile, EnvAllNew} with `key()`; `ItemLine`; `consent_items(&UiManifest)`
       and `consent_diff(old, new)` (§C.5; extend the existing `UiManifest::consent_items`);
       `CapabilityCheck::item_state` and `t3_request` as default methods; `T3Prompt` and
       `T3Answer` {Once, UntilClosed, Deny}; `journal::JournalStore` (`LoadOptions::journal`,
       `read`/`clear`, 2000 entries / 30 days, never terminal content);
     - in ui_ext_host `permissions.rs`/`consent.rs`: `ConsentRecord.items` (serde default);
       `install(ext, record, ticked)`; `set_item`; `item_state`; `items`;
       `set_always_allow(ext, cap, sha)`, reset when the sha changes; `set_t3_prompter`;
       `NeedsApproval { diff }`.
  2. Then wire T3 in `fyx_ui_v0_4.rs` `request_read`/`request_write`. Allow `terminal:read` and
     `terminal:write` in `world_capabilities` V0_4 and `permissions.rs` T3. The TerminalHost
     trait has no read/write yet: agree it with W1.
  3. `reason` for `[[fyx_ui.processes]]` and `[fyx_ui.network]`, 0.4-only. `fyx_ext/src/manifest.rs`
     (W4) also parses `processes`. Plan: a `deserialize_with` that strips `reason` before
     `ProcessSpec` (deny_unknown_fields), with the reasons kept beside it; refused below 0.4.
  4. `fyx_registry` tiers and `capability-tiers.json` for the 0.4 capabilities, with W10's 5.21
     (local_d0b39c8a-…): T3 counts as T2 for the release tier.
- **Gotchas:** for ui_ext_wasm tests, patch `fyx_ext_harness` locally with the arms above, then
  `git checkout -- crates/fyx_ext_harness`. Targets: `E:/fyx-targets/w2c` (ui_ext_wasm,
  ui_ext_host) and `E:/fyx-targets/w2c-light` (ui_ext, fyx_registry). Bash heredocs with
  apostrophes can break: write scripts with the Write tool into the scratchpad.
  `cargo fmt` skips `t2_shared.rs` (it is `include!`d), so run `rustfmt --edition 2024` on it.
