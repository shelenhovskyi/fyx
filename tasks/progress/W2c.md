# W2c progress (ui_ext model, 0.2–0.4 imports, ui_ext_wasm dispatch, fyx_guard, fyx_registry)

Older history: `archive/W2c.md`.

## Handoff (2026-11-19, seventh context clear)

- **Branch** claude/fyx-w2c-host-0-2-cbde2c; fyx/main merged at cb5b9d7e1d (4d23d31ffb).
- **Done since the last clear** (ready at W0 with a4fcba01b1):
  - 8a5bfc9968 T3 runtime: request-read/write (declared, gesture, lines, check_write_text,
    t3_target, untrusted cwd), queued per call, started after it on tokio: broker.t3_request
    (no card on own terminals), host.read/write, journal TerminalRead/Write; aborted on drop.
  - 8c7ac0c31f `${pid}` (TemplateContext::pid defaults to this process).
  - 75d2762cf0 `reason` on processes/network, required from 0.4, refused below.
  - WIT batch for W0: side-session-notes `W2c-wit-0.4-batch.diff` (review.withdraw,
    editor-context.trusted + MRU, context.to-file-ref/absolute-path). Runtime after W0 applies.
- **Next:**
  1. 5.11 listeners + MCP wrapper (§B.7): subprotocol "mcp" + `X-Claude-Code-Ide-Authorization`
     for mcp-websocket, `/mcp` for streamable-http; confirm mcp.respond (full CallToolResult) to
     W4; revocation of `network:listen#<n>`/`fs:discovery-file#<n>` closes listeners and removes
     discovery files (network.rs watch_permissions).
  2. context/review runtime for the WIT batch, once applied.
  3. fyx_guard: account dirs as protected paths, once W1's 4637d43836 (fyx.accounts) lands in
     fyx/main: `FyxAccountsSettingsContent::accounts()[].dir` into `write_denial_of` and reads.
  4. 5.13 (with W4: IDE connection to hook session_id).
- **Local-only, never commit:** the ReadDone/WriteDone arms in W4's `fyx_ext_harness`
  (terminals.rs event_handle, v0_4.rs terminal_io_event); `ui_ext_host` does not build on
  fyx/main + this branch until W3's consent port (e66159d3c6, W3 branch) is integrated.
- **Gotchas:** target `D:/fyx-targets/w2c`. Bash heredocs break on quotes: write python
  scripts with the Write tool; open files with `encoding='utf-8'`. `cargo fmt` skips
  `t2_shared.rs`: `rustfmt --edition 2024` it.
