# Gemini CLI compat transcripts, and the kit's gaps

`fyx_gemini_cli` pins **Gemini CLI 0.69.0** (`PINNED_GEMINI_CLI` in `../compat.rs`). Its sources
are:
- the companion spec, `docs/ide-integration/ide-companion-spec.md` at google-gemini/gemini-cli
  `17720a507b` (2026-11-18);
- the CLI's `packages/core/src/ide/` at that date.

Where the source and the spec differ, the source wins:
- `closeDiff`'s text is JSON `{"content": …}`;
- `closeDiff` takes `suppressNotification`;
- the legacy `ide/diffClosed` notification is still read.

`cargo test -p fyx_gemini_cli` replays the transcripts against `companion::Server`, and
`fyx_ext_harness`'s `gemini_cli_replays_the_compat_transcripts_on_its_listener` replays them
through the built 0.4 component on the fake `ide` listener.

## Format: `fyx-gemini-transcript`, version 1

JSON Lines, the same shape as `fyx_claude_code`'s `fyx-claude-transcript`
([its README](../../../fyx_claude_code/src/compat/README.md)), without hooks.

**The header** has `format`, `version`, `gemini_cli`, `recorded` (`synthetic` or
`YYYY-MM-DD`) and `root`.

**The events:**
- `mcp`: `dir` (`in` or `out`), `msg`, and `conn`, the connection id (default 1).
  `out` messages must match what the extension produced, connection included.
- `review`: `call` (the `openDiff` call's id, used as the proposal id), `decision` and
  `final_content`. The outcome becomes `ide/diffAccepted` or `ide/diffRejected` on the
  connection that opened the diff.
- `editor`: `files` as `[worktree path, focused_at ms]`, `selection` as `{line, column, text}`
  (zero-based, for the active file), and `trusted`. The extension sends `ide/contextUpdate`.

| File | Recorded | Covers |
|---|---|---|
| `two-diffs.jsonl` | synthetic | handshake; context with a 1-based cursor; one diff accepted in Fyx with an edit; one answered in the CLI (`closeDiff` with `suppressNotification`) |

**Recording** (not done yet): run `gemini` in a throwaway `GEMINI_CLI_HOME` with a recording
companion server and its discovery file. This needs a signed-in test account, which is the
owner's decision, as for Claude. Before each release, check by hand against the newest CLI.

## Gaps: what Gemini needs that the 0.4 kit (and Claude) did not

This is the point of 5.16. Each gap names who it is for; the closed ones stay listed with what
closed them.

1. **Typed discovery content: closed (W1, 5.12).** A whole `"${port}"` is written as a number,
   `"${worktrees.joined}"` joins the roots with the OS path delimiter, and `content` nests, so the
   manifest writes `ideInfo` (without it `detectIde` refuses IDE mode in Fyx).
   `companion::discovery_content` is the reference the harness test checks the template
   against.
2. **A folder name with a placeholder (W1/W2c).** The spec's POSIX dir is
   `${tmp}/gemini-ide-${uid}`, and the kit refuses placeholders inside a folder name. The
   manifest uses `${tmp}/gemini/ide`, which the CLI reads on every OS. On Windows that is the
   spec's own dir; on POSIX it is the CLI's legacy fallback, which it reads without its
   permission checks.
   - In a shared `/tmp`, the host must refuse a `gemini` folder it does not own, not only a
     symlink.
   - If Gemini drops the fallback, the kit needs `${uid}` in folder names, or a dir per OS.
3. **The IDE's pid in env (W2c).** The CLI looks for `gemini-ide-server-<IDE pid>-*.json`, and
   finds that pid with a heuristic:
   - Unix: the shell's grandparent;
   - Windows: the third process from the root of its process tree.

   In Fyx the shell is Fyx's direct child, so neither heuristic finds Fyx's pid. The CLI then
   falls back to any file whose workspace contains its cwd, which works but can pick the wrong
   window. `GEMINI_CLI_IDE_PID` would fix it, but needs an env template for the host's pid
   (`${pid}`, which exists only in discovery templates). `GEMINI_CLI_IDE_SERVER_PORT` (set) is
   the spec's tie-breaker, so the gap is small.
4. **Withdrawing a review proposal: closed (W2c, `review.withdraw`).** `closeDiff` withdraws the
   proposal and returns its current text, with the user's edits.
5. **Focus times and trust in the editor context: trust closed (W2c, `editor-context.trusted`).**
   `open` is most recently focused first but has no times, so each file's `timestamp` is its
   rank (`wired::context_update`). The spec asks for Unix times, but the CLI only sorts by it
   (`ideContext.ts`), and `client.ts` does not send it to the model.
6. **Non-blocking outcomes on the right connection: no gap.** The decision goes back as
   `listeners.notify(listener, some(connection), …)`, and `mcp-call` carries the connection.
7. **Listener details (W2c 5.11):**
   - The CLI posts to `/mcp`, so the `mcp-streamable-http` listener must serve that path.
   - In Gemini's sandbox (Docker) the CLI uses `host.docker.internal`, which the `Host` check
     refuses. That is a known limitation and stays one.
8. **The context on a new connection: closed (W0, `io-event::mcp-initialized`).** The CLI
   connects when `/ide enable` runs (or at start); once it finishes the handshake, the
   extension sends it the last `ide/contextUpdate` (unless no file is open).

## Hooks and turns: decided, built after the skeleton

Gemini has no plugin-dir env like `CLAUDE_CODE_PLUGIN_DIRS`. The coordinator decided
(2026-11-19):
- The hooks go in a settings file shipped in the package. `GEMINI_CLI_SYSTEM_DEFAULTS_PATH =
  "${package}/gemini-defaults.json"` goes into `[fyx_ui.terminal_env]`, in profile terminals
  only. It needs `package:files` too.
- System defaults have the lowest precedence, and nothing is written into the user's
  `~/.gemini`, which is an account dir and protected.
- There is no installed Gemini extension.
- The hooks are command hooks through `fyx-relay`, as in `fyx_claude_code`. They need
  `relay:hooks` and relay channels. They give Gemini turns:
  - `BeforeAgent` begins a turn;
  - `AfterAgent` ends it.

  The event names must be checked against the pinned CLI when this is built.

Until then, Gemini's edits outside `openDiff` show as "changed without review".
