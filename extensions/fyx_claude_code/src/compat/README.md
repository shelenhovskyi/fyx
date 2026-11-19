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
| `mention` | `path`; `lines`: `[start, end]`, zero-based | The user sends lines to Claude; the next `out` must be the extension's `at_mentioned`. |
| `diagnostics` | `files`: `{path, diagnostics}`, each diagnostic `{range: {start: [line, column], end}, severity, message, source}` | What the host's diagnostics hold from now on; `getDiagnostics` answers from it (one file, or every file without a `uri`). |

At the end, no `out` message may be left unsent and no `openDiff` left open.

## The transcripts

| File | Mode | Recorded | Covers |
|---|---|---|---|
| `turn-with-diff.jsonl` | interactive | synthetic | handshake, selection, one turn: diagnostics, an `Edit` with `openDiff` partly accepted, `close_tab`, a wait, `Stop`, session end |
| `live-interactive.jsonl` | interactive | 2026-11-19 | three turns on Windows: diagnostics (with a permission prompt), an `Edit` accepted with the user's own change, an `Edit` rejected, a selection and an `at_mentioned`, session end |
| `live-print-hooks.jsonl` | print | 2026-11-19 | `claude -p --ide`: hooks only (print mode does not connect to the IDE), an `Edit` denied by the permission mode |

The two live transcripts come from the recordings in `recorded/` (every frame both ways, every
hook, times in ms, Windows paths under `C:\work\app`). In the transcripts the paths are under
`/work/app`, and the extension's answers and the hooks' effects were filled by `compat::fill`,
then read against the recording.

`turn-with-diff.jsonl` is synthetic. Its shapes were read from the 2.1.326 binary:
- the lock file's fields (`pid`, `workspaceFolders`, `ideName`, `transport`, `authToken`,
  `runningInWindows`);
- the `X-Claude-Code-Ide-Authorization` header and the `mcp` WebSocket subprotocol;
- `openDiff`'s arguments and the answers Claude checks (`FILE_SAVED` with the text,
  `DIFF_REJECTED`, `TAB_CLOSED`);
- the `selection_changed` params;
- the hook event names.

Fields that Claude 2.1.326 reads but the binary does not pin down are marked "assumed" below.
The live transcripts confirm them (see "What the live recording showed").

## Recording

Done for 2.1.326 on 2026-11-19, with the tools in `recorder/`. A recording needs a signed-in
Claude Code; never use the real profile:
1. Make a throwaway `CLAUDE_CONFIG_DIR` and sign it in. Signing in is the owner's decision; the
   coordinator asks. Delete the folder afterwards, which signs that copy out.
2. Start the recording IDE server, `recorder/recorder.py` (Python, with `aiohttp`, `pywinpty`
   and `pyte`). It plays the IDE: a WebSocket on `127.0.0.1` with subprotocol `mcp` that checks
   `X-Claude-Code-Ide-Authorization`, the lock file `<CLAUDE_CONFIG_DIR>/ide/<port>.lock`,
   scripted answers (the first `openDiff` accepted with a change of the user's own, the next
   rejected), and a log of every frame. `recorder/relay.sh` stands in for `fyx-relay`
   (`FYX_RELAY`) and posts each hook's stdin to the recorder.
3. Run an interactive `claude --ide --permission-mode default` in a ConPTY through the
   recorder's `/ctl/*` API, in a scratch folder, with `CLAUDE_CODE_SSE_PORT`,
   `ENABLE_IDE_INTEGRATION=true`, `CLAUDE_CODE_PLUGIN_DIRS=<this extension>/claude-plugin`,
   `DISABLE_AUTOUPDATER=1` and `CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL=1`. Trust the folder in
   Claude's dialog. `--permission-mode default` matters: in `auto`, now Claude's default, and in
   `acceptEdits`, Claude writes without `openDiff`.
4. `recorder/convert.py` writes the sanitized recording (`recorded/`) and a draft. It strips the
   project and config dirs, session, prompt and tool-use ids, other UUIDs and pids; the recorder
   never logs the token. `FYX_CLAUDE_DRAFT=<draft> FYX_CLAUDE_TRANSCRIPT=<file> cargo test -p
   fyx_claude_code fill_draft -- --ignored` fills the draft. Read the result against the
   recording before adding it to `TRANSCRIPTS`.

**Before each release**, run the same steps by hand against the newest Claude Code. If the new
transcript does not replay, the protocol changed: fix the extension, then move the pin.

## What the live recording showed (2.1.326)

Confirmed: the lock file and header, the `mcp` subprotocol (path `/`), `ide_connected {pid}`
after `notifications/initialized`, `openDiff`'s arguments and tab name, `FILE_SAVED` + text
(Claude notices the user's change and re-reads the file), `DIFF_REJECTED`, `selection_changed`
(shown as "Selected N lines from app.py in Fyx", with `ideName`), and the hook inputs.

New, each with who it is for:
1. **Print mode never connects to the IDE** (W4, design). `claude -p --ide` with
   `CLAUDE_CODE_SSE_PORT` runs the hooks but opens no WebSocket. Print and SDK runs get turns
   and status from hooks only, and their edits show as "changed without review".
2. **`auto` is now Claude's default permission mode** (design, W4). The known `acceptEdits` and
   `auto` gap is therefore the common case: `openDiff` only comes in manual (`default`) mode.
3. **No `Stop` after a rejected diff** (W4). Claude ends the turn without the `Stop` hook. The
   extension's turn stays open, with the status on the tool, until the next prompt ends it. The
   extension should end the turn when it answers `DIFF_REJECTED`.
4. **Windows paths and URIs** (W4 `ide.rs`, W2c `context.to-file-ref`). Paths come with
   backslashes (`C:\work\app\app.py`) in `openDiff`, `at_mentioned` and the lock file.
   `getDiagnostics` gets a URI in two forms: `file:///C:/work/app/app.py` (the model's call) and
   `file://C:\work\app\app.py` (Claude's own call before an edit). `ide.rs` strips `file://`,
   so the first form becomes `/C:/...`. Its `fileUrl` and `uri` answers would be
   `file://C:\...`. The transcripts map everything to `/work/app`, so they do not cover this.
5. **`getDiagnostics` is called about four times a turn** (W4, 5.14): around each edit, for one
   file and for all files without arguments. Claude diffs the answers ("Found 1 new diagnostic
   issue"), so they must be fast and stable.
6. **`closeAllDiffTabs` at every prompt, and `close_tab` twice after each answered diff** (W4,
   with W2c's `review.withdraw`). Both come after the review was decided, so the extension must
   answer from its own state and not withdraw a decided proposal.
7. **The IDE's tool descriptions reach the model** (W4). The model loads
   `mcp__ide__getDiagnostics` through `ToolSearch`, with the description from `tools/list`, and
   the call needs the user's permission in manual mode. The hooks' status labels show raw names
   such as `mcp__ide__getDiagnostics`.
8. **`initialize` uses protocol `2025-11-25`** (W2c, 5.11), with client capabilities `roots`,
   `elicitation` and the `io.modelcontextprotocol/tasks` extension. The listener's MCP must
   accept that version.
9. **`at_mentioned` lines are zero-based**; Claude shows `lineStart` 10 as `#L11`. That matches
   `ide::at_mentioned`.
10. **New hook fields**, which `HookInput::parse` ignores: `prompt_id`, `permission_mode`,
    `effort`, `model`, `scratchpad_dir`, `tool_use_id`, `last_assistant_message`, and
    `notification_type` (`permission_prompt`). A tool the permission mode denies has a
    `PreToolUse` and no `PostToolUse`.

## Known gaps

- In `acceptEdits` and `auto` modes Claude writes files without `openDiff`. Those edits reach
  Fyx only through `PostToolUse` and show as "changed without review".
- Assumed, and accepted by Claude in the live recording: `getDiagnostics`' answer (a text item
  holding `[{uri, diagnostics}]`, as the VS Code extension answers). Confirmed:
  `ide_connected {pid}`, and that `CLAUDE_CODE_PLUGIN_DIRS` names plugin roots (the plugin's
  hooks ran).
- Claude 2.1.326 accepts `http` hooks only with a literal URL, and the listener's port changes
  every run. So the plugin uses command hooks through `fyx-relay` instead of the `http` hooks
  the design names.
- `close_tab` and `closeAllDiffTabs` answer without closing anything in Fyx. W2c is staging
  `review.withdraw(id) -> result<string, host-error>` (the current text); the extension wires it
  in plan 5.14.
