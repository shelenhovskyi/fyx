# Claude Code compat transcripts

Claude Code's IDE protocol is reverse-engineered, so `fyx_claude_code` pins one Claude Code
version and replays recorded protocol transcripts against the extension
([design, "The first extensions", item 5](../../../../docs/fyx/wit-0.4/design.md)). When
Claude changes the protocol, a transcript stops replaying and the test fails loudly.

- **Pinned version:** Claude Code **2.1.326** (`PINNED_CLAUDE_CODE` in `../compat.rs`).
  Every transcript must name it. Moving the pin means recording again.
- **Replayed by:** `cargo test -p fyx_claude_code` (`compat::tests`), against the protocol core
  (`ide.rs`, `hooks.rs`). Once the SDK and harness have listeners and the relay (plan 5.14), the
  same files are replayed through `fyx_ext_harness` against the built component.
- **Not shipped:** this folder is under `src/`, which neither `script/fyx-build-demos` nor
  `fyx-ext package` copies.

## Format: `fyx-claude-transcript`, version 1

JSON Lines, UTF-8, one JSON object per line. Blank lines are skipped.

**Line 1, the header:**

```json
{"format":"fyx-claude-transcript","version":1,"claude_code":"2.1.326","mode":"interactive",
 "recorded":"synthetic","root":"/work/app","note":"..."}
```

- `mode`: `interactive` or `print` (`claude -p`). An interactive session runs the plugin's hooks
  only after the folder is trusted in Claude. `-p` and SDK runs run them at once (research
  question 1, tested on 2.1.326).
- `recorded`: `synthetic` for a transcript written by hand from the version's schemas, or the
  recording date as `YYYY-MM-DD`.
- `root`: the one workspace root. The transcript's absolute paths are under it, and the replay
  maps it to worktree 1.

**Every other line is an event, by `t`:**

| `t` | Fields | Replay |
|---|---|---|
| `mcp` | `dir`: `in` (Claude → Fyx) or `out` (Fyx → Claude); `msg`: the JSON-RPC message as sent | `in` `tools/call` goes to `IdeCall::parse`. The host answers every other `in` message itself (`initialize`, `notifications/initialized`, `ide_connected`). An `out` message must be the next one the extension produced, exactly. For the host's `tools/list` answer, the tool names must match `ide::tools()`, in order. |
| `hook` | `channel`: as the plugin passes it (`fyx-claude-code/<name>`); `stdin`: Claude's hook input; `expect`: the effects | `HookInput::parse`, then `Sessions::apply`. The effects must equal `expect`, written as `{"begin-turn":<session>}`, `{"end-turn":{"session":…,"changed":[<worktree path>…]}}`, `{"status":{"state":…,"label":…}}` or `{"status":null}`. |
| `review` | `call`: the `openDiff` call's JSON-RPC id; `decision`: `accepted`, `partial`, `rejected` or `closed`; `final_content` | The user's decision in agent review. It answers that call (`FILE_SAVED` + text, `DIFF_REJECTED` or `TAB_CLOSED`). |
| `editor` | `path`: the worktree path; `selection`: `start` and `end` as `[line, column]`, and `text` | `io-event::context-changed`; the next `out` must be the extension's `selection_changed`. |

At the end, no `out` message may be left unsent and no `openDiff` left open.

## The transcripts

| File | Mode | Recorded | Covers |
|---|---|---|---|
| `turn-with-diff.jsonl` | interactive | synthetic | handshake, selection, one turn: diagnostics, an `Edit` with `openDiff` partly accepted, `close_tab`, a wait, `Stop`, session end |

`turn-with-diff.jsonl` is synthetic. Its shapes were read from the 2.1.326 binary:
- the lock file's fields (`pid`, `workspaceFolders`, `ideName`, `transport`, `authToken`,
  `runningInWindows`);
- the `X-Claude-Code-Ide-Authorization` header and the `mcp` WebSocket subprotocol;
- `openDiff`'s arguments and the answers Claude checks (`FILE_SAVED` with the text,
  `DIFF_REJECTED`, `TAB_CLOSED`);
- the `selection_changed` params;
- the hook event names.

Fields that Claude 2.1.326 reads but the binary does not pin down are marked "assumed" below.
A live recording replaces it.

## Recording (not done yet)

A recording needs a signed-in Claude Code. Never use the real profile:
1. Make a throwaway `CLAUDE_CONFIG_DIR` and sign it in to a test account. Signing in is the
   owner's decision; the coordinator asks.
2. Run a recording IDE server: a WebSocket on `127.0.0.1`, subprotocol `mcp`, that checks the
   header, writes `<CLAUDE_CONFIG_DIR>/ide/<port>.lock`, and logs every frame both ways. It also
   logs `fyx-relay`-style hook stdin through the plugin, with `FYX_RELAY` pointing at a logger.
3. Start `claude --ide` in a scratch, trusted folder with `CLAUDE_CODE_SSE_PORT`,
   `ENABLE_IDE_INTEGRATION=true` and `CLAUDE_CODE_PLUGIN_DIRS=<this extension>/claude-plugin`.
   Drive one turn that edits a file. Accept part of the diff in the recorder.
4. Write the log in this format, with `recorded` set to the date, and the absolute paths under
   one `root`.
5. Record both modes: `interactive` (hooks only after trust) and `print` (`claude -p`, hooks at
   once).

**Before each release**, run the same steps by hand against the newest Claude Code. If the new
transcript does not replay, the protocol changed: fix the extension, then move the pin.

## Known gaps

- In `acceptEdits` and `auto` modes Claude writes files without `openDiff`. Those edits reach
  Fyx only through `PostToolUse` and show as "changed without review".
- Assumed: `getDiagnostics`' answer (a text item holding `[{uri, diagnostics}]`, as the VS Code
  extension answers), `ide_connected`'s params, and that `CLAUDE_CODE_PLUGIN_DIRS` names plugin
  roots.
- Claude 2.1.326 accepts `http` hooks only with a literal URL, and the listener's port changes
  every run. So the plugin uses command hooks through `fyx-relay` instead of the `http` hooks
  the design names.
