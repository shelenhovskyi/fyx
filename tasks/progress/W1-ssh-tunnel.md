# W1: 5.17 SSH terminals and the remote listener tunnel (2026-11-19)

Implemented; module doc in `crates/terminal_host/src/discovery/remote.rs` is the reference.

## Research: the pid in lock files

- **Claude Code 2.1.328** (strings in the bundled `claude.exe`): at start it deletes a lock file
  whose `pid` is not running (`process.kill(pid, 0)`, not on WSL). A lock with no pid (or 0) is
  deleted only if `127.0.0.1:<port>` refuses. When picking an IDE, the pid must be running and be
  its parent or an ancestor (10 levels) **only** in a "supported terminal" (VS Code family,
  JetBrains, `FORCE_CODE_TERMINAL`) and only if the port is not `CLAUDE_CODE_SSE_PORT`. Zed/Fyx
  terminals are not "supported", so remote Claude connects with `--ide`, `/ide` or
  `autoConnectIde`; `workspaceFolders` must contain its cwd.
- **Gemini CLI** (main, `ide-connection-utils.ts`): never deletes. Sorts
  `gemini-ide-server-<pid>-<port>.json` by pid match (its IDE pid from the process tree or
  `GEMINI_CLI_IDE_PID`), then alive, then picks the file whose `workspacePath` matches the cwd.
  Every file must be the user's and `mode & 077 == 0`; the new dir `$TMPDIR/gemini-ide-<uid>`
  must be 0700, the legacy `$TMPDIR/gemini/ide` (ours) is not checked.
- So: `${pid}` = 0 on the host. No staging file, no `$$` from the terminal's exec string.

## What was built (changes the earlier plan)

- **One tunnel per SSH host, not forwards per terminal.** `ssh -o ExitOnForwardFailure=yes
  -o BatchMode=yes -R 127.0.0.1:p:127.0.0.1:p… <dest> sh -c <script>` (`project::fyx_remote`,
  built by `RemoteClient::build_command`). Per-terminal `-R` breaks on Windows (no ControlMaster:
  the second terminal's forward fails, the first one's dies with it) and a failed forward would
  leave a lock pointing at a port another user holds. With `ExitOnForwardFailure`, `ssh` exits
  before the script runs, so no token goes to someone else's port.
- The script writes the files from stdin (one JSON line each, shell `read`/`printf`, `umask 077`,
  folders the user owns, no symlinked folder under the temp dir, write beside + `mv`), prints
  `fyx-tunnel-ready`, waits for stdin EOF and removes what it wrote (trap). Fyx closing stdin,
  quitting or dying all end it; a stopped tunnel gets 3 s, then `ssh` is killed.
- `discovery::remote::refresh` runs on worktree/settings/project changes and on listener
  open/close (`DiscoveryWriter::changes`). Restart when the plan changes; an ended tunnel is
  retried on a later change after 60 s. Hosts keyed by `display_name()`; projects on one host
  share a tunnel (worktrees merged).
- **Remote env:** `TerminalEnvHook::remote_env` + `terminal_ext::apply_remote_env_hook`, called
  by a `fyx:` line in `create_remote_shell` (through `pub use terminal_ext` in `terminal`). Only
  all-new env; only `${listen.<n>.port}` of a ready tunnel; no token, package, account, relay,
  pid. Claude's env is profile-scoped, so remote shells get none of it by default.

## Not verified / open

- No real sshd: `ExitOnForwardFailure` stopping the command, ControlMaster forwards (they stay in
  the master after the tunnel exits; a duplicate `-R` is accepted by the master), `BatchMode` with
  Zed's askpass. Test on a Linux host with a fake `~/.claude`: `claude --ide` in a remote shell.
- `a_symlinked_temp_folder_gets_no_file` is unix-only (not run on Windows).
- Not for WSL, Docker or Windows hosts. Password-only SSH hosts get no tunnel (BatchMode).
- Relay over SSH still out of scope: remote relay hooks fail open.
