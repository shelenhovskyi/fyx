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

`cargo test -p fyx_gemini_cli` replays the transcripts against `companion.rs`.

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

This is the point of 5.16. Each gap names who it is for.

1. **Typed discovery content (W1, with the JSON-aware templates).** The CLI wants three things
   the string-only `content` table cannot write:
   - `port` as a JSON number;
   - `ideInfo` as a nested object `{name, displayName}`;
   - `workspacePath` as the roots joined with the OS path delimiter (`:` or `;`), not a JSON
     array.

   Without `ideInfo`, `detectIde` falls back to `TERM_PROGRAM` and refuses IDE mode in Fyx. So
   this gap blocks the extension, not just polish. `companion::discovery_content` is the
   reference shape.
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
4. **Withdrawing a review proposal (W2c, `review`).** `closeDiff` must close the open diff and
   return its current text, with the user's edits. `review` has `propose` and no `withdraw`, so
   the extension cannot close the proposal or read its text. For now, the proposal's own text
   stands in. Claude's `close_tab` and `closeAllDiffTabs` need the same: today they answer
   without closing anything in Fyx. Proposal: `review.withdraw(id) -> result<string,
   host-error>`, returning the current text and ending the proposal without a `review-decided`.
5. **Focus times and trust in the editor context (W2c, `context`).** `ide/contextUpdate` orders
   files by last-focus time and carries `isTrusted`. `editor-context` has neither. The extension
   can stamp files itself as `context-changed` reports them active, if the guest has a clock.
   Trust it cannot know. Proposal: `editor-context.trusted: option<bool>` (the worktree's
   trust), and an MRU order for `open`.
6. **Non-blocking outcomes on the right connection: no gap.** The decision goes back as
   `listeners.notify(listener, some(connection), …)`, and `mcp-call` carries the connection.
7. **Listener details (W2c 5.11):**
   - The CLI posts to `/mcp`, so the `mcp-streamable-http` listener must serve that path.
   - In Gemini's sandbox (Docker) the CLI uses `host.docker.internal`, which the `Host` check
     refuses. That is a known limitation and stays one.

Not in this skeleton: Gemini's hooks, and therefore agent turns. Gemini has no plugin-dir env
like `CLAUDE_CODE_PLUGIN_DIRS`, only `GEMINI_CLI_SYSTEM_DEFAULTS_PATH` (a settings file) or an
installed extension. That is a separate design question for the coordinator. Until then,
Gemini's edits outside `openDiff` show as "changed without review".
