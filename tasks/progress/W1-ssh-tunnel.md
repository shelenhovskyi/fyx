# W1: 5.17 SSH terminals and the remote listener tunnel (proposal, 2026-11-19)

Not started; needs the coordinator's go on the upstream hooks below.

## How SSH terminals are built today

`project/src/terminals.rs` turns a remote shell into `ssh [options] <dest> <exec>` with
`RemoteConnection::build_command` (`remote/src/transport/ssh.rs`). The env goes into `<exec>`, so
the env hook in `TerminalBuilder::new` only reaches the local `ssh` process; it already skips
extension env for remote terminals (`EnvRequest::remote`). `build_command` takes one `-L` forward
(debuggers), no `-R`.

## Proposal

1. **Reverse forward with the same port.** For an SSH terminal, add `-R <p>:127.0.0.1:<p>` for each
   open listener of an extension whose env reaches the terminal, and for the relay endpoint, with
   `-o ExitOnForwardFailure=no`. Same port on both ends, so W2c's `Host: 127.0.0.1:<port>` check
   passes unchanged. If the remote port is taken, ssh warns and the forward is missing, so the tool
   fails open.
   Hook: one `fyx:` call in `create_terminal_shell_internal` (SSH transport only, not WSL/Docker)
   that inserts the `-R` args at the front of the command template's args and merges the env
   before `build_command`. It needs `terminal_ext::remote_terminal_additions(cx)`
   (env + forwards), which `terminal_host` fills.
2. **Env on the remote side.** `${listen.*}`, `FYX_RELAY_PORT` and `FYX_TERMINAL_TOKEN` go into
   `<exec>`. `FYX_RELAY` has no remote binary, so remote relay hooks stay unsupported (fail open)
   unless we upload `fyx-relay` with the remote server (larger; later).
3. **Remote discovery files** (Claude Code reads `~/.claude/ide/<port>.lock` on the remote host).
   Open question: writing them needs a remote write path. The remote server has fs RPCs for
   worktrees, not for arbitrary home paths. Putting the token on the ssh command line leaks it to
   `ps` on shared hosts, so that is refused. Options: (a) a new remote-server request "write host
   file 0600" (proto change, W0); (b) skip lock files and rely on `CLAUDE_CODE_SSE_PORT` plus a
   token in env, if Claude Code accepts it without a lock file (research first).

Test from plan 5.17 ("a remote Claude reaches the listener over SSH") needs an SSH host: the
home-server (`~/fyx-light`) can serve as one.
