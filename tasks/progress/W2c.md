# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, eighth context clear)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c, on fyx/main b481177ef9 (04bb5cd126 merged there).
- **Done since the last clear** (not at W0 yet):
  - d1b1a881e6 fyx_guard: `fyx.accounts` dirs are protected (§C.6 rule 5), reads and writes.
  - 98777859a4 integrator lines: ui_ext_wasm deps async-tungstenite (tokio-runtime), rand,
    serde_json.
  - a7255411a9 5.11 core: `ui_ext_wasm/src/listeners.rs` `ListenerSet` (admission, MCP over
    websocket and streamable HTTP, raw kinds; 7 tests). `ui_ext::integration` ListenerEvent,
    McpCall, ToolDef, McpError, OpenListener, ListenerObserver; IoEvent::Listener.
  - WIT fix for W0: side-session-notes `W2c-wit-0.4-withdraw.diff`: `review.withdraw ->
    result<string, host-error>` (current text, no review-decided; staged dc04464ee8 was
    `result<_>` + err("withdrawn")). Asked W4 (local_8c72db38) to confirm; no answer yet.
- **Next:**
  1. Wire 5.11: LoadOptions `listener_observer: Option<Arc<dyn ListenerObserver>>`; a
     ListenerSet per instance in ui_ext_wasm.rs (next to TerminalsHandle ~:1962), sink to
     `running.deliver(IoEvent::Listener(..))` like terminals; open each granted
     `network:listen#<name>` (ConsentItem::Listener) at start on the tokio runtime, with its
     granted `fs:discovery-file#<spec.name>` files; `impl wit_listeners::Host` in
     fyx_ui_v0_4.rs (+ add to `add_to_linker`'s list ~:431); watch_permissions (network.rs
     ~:979): revoke of `network:listen[#n]` -> close, grant -> open, discovery item change ->
     reopen with the new list; `${listen.<n>.port/token}` in terminals.rs PackageContext.
  2. 5.13 review/context runtime (after W0 applies the withdraw diff).
- **Local-only, never commit:** `IoEvent::Listener(..) => None` in W4's fyx_ext_harness
  v0_3.rs/v0_4.rs (W0 adds it at integration).
- **Gotchas:** target `D:/fyx-targets/w2c` (a sweep once emptied .fingerprint mid-build:
  rerun). Write python scripts with the Write tool, not heredocs. async-tungstenite's
  `split()` halves have inherent `send`/`close(None)`. `cargo fmt` skips `t2_shared.rs`.
