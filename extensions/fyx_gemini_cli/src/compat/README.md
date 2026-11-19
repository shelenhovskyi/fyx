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

   What the host needs, exactly:
   - **W1 (manifest check, `ui_ext::integration`):** allow `${uid}` inside a `dir` segment of
     `[[fyx_ui.discovery_files]]` (only `${uid}`: a number, never a path), so the manifest can
     say `${tmp}/gemini-ide-${uid}`. A per-OS `dir` (`dir_windows`/`dir_unix`) would do too, but
     one placeholder is smaller.
   - **W2c (discovery writer, `ui_ext_host`):** expand `${uid}` to the real uid on Unix (on
     Windows the spec's dir has none, so the manifest keeps `${tmp}/gemini/ide` there); and,
     before writing under a shared `/tmp`, refuse each folder on the way that Fyx's user does
     not own or that others can write to (no sticky bit), not only a symlinked one, and
     create the folders it makes private to the user.
3. **The IDE's pid in env (W2c).** The CLI looks for `gemini-ide-server-<IDE pid>-*.json`, and
   finds that pid with a heuristic:
   - Unix: the shell's grandparent;
   - Windows: the third process from the root of its process tree.

   In Fyx the shell is Fyx's direct child, so neither heuristic finds Fyx's pid. The CLI then
   falls back to any file whose workspace contains its cwd, which works but can pick the wrong
   window. `GEMINI_CLI_IDE_PID` would fix it, but needs an env template for the host's pid
   (`${pid}`, which exists only in discovery templates). `GEMINI_CLI_IDE_SERVER_PORT` (set) is
   the spec's tie-breaker, so the gap is small.

   What the host needs, exactly (W2c, with W1 for the check): `${pid}` in `[fyx_ui.terminal_env]`
   `vars`, expanded to the Fyx process's pid (the same number the discovery file's name uses).
   The manifest then adds `GEMINI_CLI_IDE_PID = "${pid}"`.
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

## Hooks and turns: built

Gemini has no plugin-dir env like `CLAUDE_CODE_PLUGIN_DIRS`, so, as the coordinator decided
(2026-11-19), the hooks go in a settings file shipped in the package:
`GEMINI_CLI_SYSTEM_DEFAULTS_PATH = "${package}/gemini-settings/system-defaults.json"`, in
profile terminals only (`package:files`). System defaults have the lowest precedence, nothing is
written into the user's `~/.gemini`, and there is no installed Gemini extension.

`src/hooks.rs` reads the hooks, checked against Gemini CLI's `packages/core/src/hooks` (the
source the transcripts pin):
- Seven channels, one per event: `SessionStart`, `BeforeAgent` (begins a turn), `BeforeTool`
  (the tool on the tab), `AfterTool` (the files `write_file` and `replace` wrote, a relative
  `file_path` joined to `cwd`), `Notification` (the tab waits for you), `AfterAgent` (ends the
  turn with those files) and `SessionEnd`. A `BeforeAgent` while a turn is open ends that turn
  first. The turn's session names the `openDiff` proposals made in it.
- Every hook is answered at once with exit 0 and no output: the hooks only report.
- Gemini runs a command hook in PowerShell on Windows and in `bash -c` elsewhere, so each hook
  is `node -e "<script>" -- --channel fyx-gemini-cli/<name>`: Gemini runs on Node, and the
  script has no `$`, backtick or double quote. Outside Fyx (`FYX_RELAY` unset) it exits 0.

Known limits, none of which the extension can change:
- Hooks from settings run only in a **trusted folder** (`folderTrust` is on by default).
  Elsewhere Gemini has no turns in Fyx, and its edits outside `openDiff` show as "changed
  without review".
- With `security.environmentVariableRedaction.enabled` (off by default; on in GitHub Actions)
  the hook's env loses names with `TOKEN`, so `FYX_TERMINAL_TOKEN` is gone and the relay fails
  open: Gemini goes on, without turns. Fyx does not change that setting.
- A settings file of the user's or the project's with its own `hooks` adds to these (the lists
  are concatenated); it does not replace them.
