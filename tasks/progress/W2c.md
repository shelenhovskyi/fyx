# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, sixth context clear)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c. fyx/main merged at d354266a64. Sent W0 "ready 409504e1b2"
  (5.6 wasm side and the `${package}` check).
- **Done since the last clear:**
  - f3f8c731bc: 5.6 broker host side in `ui_ext_host::consent` (now `pub`): install/set_item/
    item_state/items/diff/state_with (`NeedsApproval { new_capabilities, diff }`)/always-allow.
    Item answers live in the file's `items` map, not in ConsentRecord. `read_file` returns
    `(records, Answers)`. Item changes stream as keys (`process:spawn#git`); network.rs
    watch_permissions kills that program. Also `ui_ext_host::journal::JournalStore`. W3 has the
    API and the wiring list (LoadOptions::journal, set_journal, state_with in check_consent).
  - 695499d96f: T3 model: `TerminalHost::t3_target/read/write` (defaults), `TerminalEvent::
    ReadDone/WriteDone`, `T3Target`, `check_write_text`. Broker: share_terminal/unshare/
    terminal_closed/shared. The API went to W1; no answer yet. A menu share only pre-approves and
    pushes no text (my decision).
- **Next:**
  1. Runtime T3 in `ui_ext_wasm/src/terminals.rs` and `fyx_ui_v0_4.rs`:
     - request_read/request_write: declared, gesture (`api.take_activation`), lines 1..=500,
       `check_write_text`, `t3_target`, and an untrusted cwd gives UntrustedWorktree. Own terminals
       need no card.
     - Queue ops; start them after the call next to `network.start`
       (ui_ext_wasm.rs ~2247) on tokio.
     - Await `broker.t3_request`. On Deny, send `ReadDone/WriteDone(id, Err("denied"))`.
       Otherwise call host.read/write with a sink that journals TerminalRead{terminal,scope,bytes}
       and TerminalWrite.
     - Abort the tasks on drop.
     - Add terminal:read/write to `world_capabilities` V0_4 (ui_ext_wasm.rs:377). Change the
       refusal in tests_v0_3.rs:1111 to an accepted manifest.
  2. `reason` on `[[fyx_ui.processes]]` and `[fyx_ui.network]` (0.4 only), fed into consent_items.
  3. 5.11 loopback listeners + MCP wrapper (§B.7), with item revocation for `network:listen#<name>`
     and `fs:discovery-file#<name>` in network.rs watch_permissions.
  4. Whole-crate tests, clippy and fmt, then "ready" to W0 and the coordinator.
- **Ready at W0: a4fcba01b1** (f3f8c731bc, 695499d96f, and set_integration; 477 tests pass). W1 and W3 are told.
- **Queued by the coordinator** (do these after the T3 runtime, step 1):
  - fyx_guard: the `fyx.accounts` dirs (W1 4637d43836) become protected paths (§C.6).
  - WIT diffs for since_v0.4.0, sent to W0 together:
    - `context.to-file-ref(abs) -> option<file-ref>` and `absolute-path(file-ref)`, both behind
      context:editor;
    - `review.withdraw(id) -> result<string>`;
    - `editor-context.trusted: option<bool>`, with files in most-recent-focus order.
  - A `${pid}` template (TemplateContext, together with W1).
  - In 5.11: mcp-websocket takes subprotocol "mcp" and `X-Claude-Code-Ide-Authorization`;
    streamable-http serves `/mcp`. Confirm the mcp.respond shape (full CallToolResult) to W4.
  - Settle with W4 for 5.13/5.14: IDE connection ↔ hook session_id.
- **Local-only patches** (never commit):
  - `<scratchpad>/host_local.diff` (W3's Props::Terminal arms) and `harness_local.diff`. Apply
    with `git apply --ignore-whitespace`.
  - The `HostEffect::AskApproval { .. } => {}` arm at registry.rs `match effect {`.
  - `<scratchpad>/host_patch.py` recreates the host arms.
  - Revert: `git checkout -- crates/fyx_ext_harness crates/ui_ext_host/src/{decorations,inspector,view}.rs`
    and remove that arm. registry.rs holds my committed `..` hook, so do not checkout registry.rs.
- **Gotchas:**
  - Targets `D:/fyx-targets/w2c` and `w2c-light` (E: is the same storage).
  - Edit files with python scripts written by the Write tool (bash heredocs break on quotes). Keep
    CRLF and use `chr(9)` for tabs.
  - `cargo fmt` skips `t2_shared.rs`: run `rustfmt --edition 2024` on it.
