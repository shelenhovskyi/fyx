# W1: 5.17 SSH terminals and the remote listener tunnel (design, 2026-11-19)

Coordinator said go (2026-11-19). No code yet. Read with `crates/project/src/terminals.rs`
(`create_remote_shell`, ~line 644) and `crates/remote/src/transport/ssh.rs`
(`ssh_command_template`, ~line 2183).

## Facts

- An SSH terminal is `ssh [options] [-L …] -o LogLevel=ERROR -t <dest> <exec>`, built by
  `RemoteClient::build_command`. The env goes into `<exec>` (the remote command line). The env hook
  in `TerminalBuilder::new` reaches only the local `ssh` process (`EnvRequest::remote`), and so do
  `FYX_TERMINAL_ID/TOKEN` (`TerminalExt::insert_env` runs after `create_remote_shell`).
- Anything in `<exec>` is visible in remote `ps` to other users, so **no token may go there**.
- `project` depends on `terminal`, which depends on `terminal_ext`: a `fyx:` hook in
  `create_remote_shell` can call through a `terminal` re-export with no new Cargo line.
- `RemoteClient::build_command(Some("sh"), ["-c", script], …, Interactive::No)` gives a
  command template we can run locally with piped stdin: content (token) goes over the encrypted
  channel on stdin, not argv. `RemoteClient::upload_directory` (scp) exists but loses the mode
  from Windows hosts, so stdin with `umask 077` is the better path.

## Plan

1. **Forwards (hook in `create_remote_shell`, `RemoteConnectionOptions::Ssh` only):** prepend
   `-o ExitOnForwardFailure=no -R <p>:127.0.0.1:<p>` for each open listener that the env of
   extensions allowed into every new terminal names. The same port on both ends keeps W2c's
   `Host: 127.0.0.1:<port>` check valid. A busy remote port means no forward, so the tool fails
   open. With ControlMaster (Unix), the forward goes through the master; check that it outlives a
   closed terminal.
2. **Remote env:** resolve `all_new_env` with no package, no accounts, no relay and no tokens:
   drop any variable using `${package}`, `${account.*}`, `${relay}` or `${listen.*.token}`. Merge
   the result into `env` before `build_command`. Needs a `terminal_host` fn behind a
   `terminal_ext` hook (like the env hook) returning `{ env, forward_ports }`.
3. **Remote discovery files:** the token goes only in a lock file written through stdin
   (`umask 077; mkdir -p dir; cat > f.tmp && mv f.tmp f`), and is removed with `rm -f` when the
   listener closes or Fyx quits. **Research first:** Claude Code and Gemini check the lock file's
   `pid`, and Fyx's pid does not exist on the remote host. Find out whether Claude Code drops a
   lock file whose pid is dead. If it does, write the pid of the remote terminal's shell (`$$`
   survives `exec`) from the exec string, reading the token from a 0600 staging file uploaded
   through stdin.
4. **Relay over SSH:** out of scope for now. The remote host has no `fyx-relay` and remote shells
   have no `FYX_TERMINAL_TOKEN`, so remote relay hooks fail open. Later: upload the binary with
   the remote server and forward `FYX_RELAY_PORT`.

Test (plan 5.17, "a remote Claude reaches the listener over SSH"): the home-server can be the SSH
host.
