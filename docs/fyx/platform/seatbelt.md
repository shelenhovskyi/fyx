# Seatbelt for extensions and agents, with network policy (plan 7.5)

Status: W5's audit notes and a proposal for W2c (2026-11-20), who owns 7.5 and decides the design.
Nothing here is built. The Mac checks are the owner's, since no Mac is in the swarm. Plan row: [plan.md P7](plan.md#p7-differentiators). Pairs with 7.4
(`docs/fyx/agent-review.md`, "Secrets for agents").

Goal (plan's done-when): a spawned tool cannot read outside its declared paths or reach an
undeclared host, and a secret sent to a host outside its list is refused.

## 1. What upstream already has (checked in this tree)

- **`crates/sandbox`** builds a platform-neutral `SandboxPolicy { fs, network }` and wraps a
  command (`Sandbox::wrap` → `WrappedCommand`, spawn it any way). macOS: `sandbox-exec` with a
  generated profile (`macos_seatbelt.rs`); Linux: bwrap; Windows: bwrap inside WSL.
- **Filesystem**: `SandboxFsPolicy::Unrestricted` or `Restricted { writable_paths, protected_paths,
  protected_file_names }`. Both **allow reading the whole disk**: the profile starts
  `(deny default)` then `(allow file-read*)`. Only writes are confined.
- **Network**: `SandboxNetPolicy::{Unrestricted, Blocked, Restricted { allowed_domains }}`.
  `Restricted` allows outbound TCP only to an in-process proxy on `localhost:<port>`
  (`crates/http_proxy`), which checks the `CONNECT`/request host against the list (exact or
  `*.` wildcard). Programs that ignore `HTTPS_PROXY` fail closed: they can reach nothing.
- **Who uses it**: only the native agent's terminal tool (`agent/src/tools/terminal_tool.rs`,
  `agent/src/sandboxing.rs`) and ACP terminals created with a `SandboxRequest`
  (`acp_thread/src/terminal.rs`). The flag `sandboxing` is on for all
  (`enabled_for_all`), so it works cloud-off. Grants: per-thread prompts plus persistent
  `agent.sandbox_permissions` (`network_hosts`, write paths, `allow_unsandboxed`).
- **Not sandboxed**: MCP servers (`context_server/src/transport/stdio_transport.rs` spawns the
  binary directly), `process:spawn` children of Fyx extensions (`ui_ext_wasm/src/process.rs`:
  PATH filter, filtered env, job object / process group, no Seatbelt), tasks, external ACP agents
  themselves (Claude Code, Codex run unconfined; only terminals Fyx creates for them are wrapped).

## 2. Gaps against the done-when

1. **Reads are not confined.** A sandboxed command can read `~/.ssh`, `~/.aws`, browser profiles,
   other projects. "Cannot read outside its declared paths" needs a read scope.
2. **`process:spawn` and MCP servers do not go through `crates/sandbox`.**
3. **No per-project egress list.** Network grants live in user settings and the thread, not in
   the project.
4. **Secrets ignore the network.** 7.4 puts a value into a command's env; that command may send it
   anywhere the policy allows. The proxy sees only the host (TLS hides the body), so the check
   has to be on the command, not on the request.

## 3. Changes

### 3.1 A read scope for Seatbelt (`crates/sandbox`, Fyx file + one hook)

`crates/sandbox/src/fyx_read_scope.rs` (macOS only, `cfg(target_os = "macos")`):

```rust
/// Where a sandboxed command may read, in addition to the system locations every program needs.
pub struct FyxReadScope {
    pub readable_paths: Vec<HostFilesystemLocation>,
}
/// The Seatbelt rules for `scope`: the system set, then one `(allow file-read* (subpath ...))`
/// per readable path, with the same quoting `macos_seatbelt.rs` uses for write paths.
pub(crate) fn read_rules(scope: &FyxReadScope) -> String;
```

The system set (read-only, literal or subpath): `/usr`, `/bin`, `/sbin`, `/System`, `/Library`
(not `~/Library`), `/opt/homebrew`, `/usr/local`, `/private/etc`, `/private/var/db/timezone`,
`/private/var/db/dyld`, `/dev`, `/Applications/Xcode.app` and `/Library/Developer` (toolchains),
the resolved `TMPDIR` of the command, `file-read-metadata` on every ancestor of an allowed path
(path walking), and the program's own resolved binary. Writable paths are readable too.

One `fyx:` hook in `macos_seatbelt.rs`: where the profile pushes `(allow file-read*)`, push
`fyx_read_scope::read_rules(scope)` instead when the `Sandbox` carries a scope. The scope rides
next to the policy, so the enum stays upstream's:
`Sandbox::fyx_with_read_scope(self, FyxReadScope) -> Self` (in the Fyx file, an `impl` block;
one field `fyx_read_scope: Option<FyxReadScope>` added to `Sandbox` under `fyx:`). Linux and
Windows ignore it for now and say so in the UI (plan: Windows keeps job objects).

Unit tests (run on any OS, they test the string): the blanket rule is gone, each path appears
once, quoting matches the write rules, ancestors get metadata only.

### 3.2 Who gets which policy

| Caller | Reads | Writes | Network |
|---|---|---|---|
| `process:spawn` (4.7) | worktrees the request may use (`resolve_cwd`'s roots) + resolved program dir | the `cwd` worktree only if the manifest declares `fs:write`, else none (+ the command's `TMPDIR`) | `Blocked`, or the extension's `network:fetch` domains (`DomainList`) when it declares them |
| MCP server (stdio) | project worktrees + its executable's install dir (npm/uv caches read-only) | none, + `TMPDIR` | project egress list (3.3) ∪ hosts in the server's settings `allowed_hosts` |
| Agent terminal tool / ACP terminal | worktrees + upstream grants | upstream (`Restricted`) | upstream grants ∩ project list when one exists (3.3) |
| Any command given a secret (7.4) | as above | as above | **narrowed** to the secret's hosts (3.4) |

Hooks: `ui_ext_wasm/src/process.rs` wraps the `std::process::Command` it builds with
`Sandbox::wrap` on macOS (Fyx crate, no upstream edit) and keeps the `Sandbox` in the process
entry so the proxy lives as long as the child. `stdio_transport.rs` gets one `fyx:` hook: when a
`FyxMcpSandbox` global is set, the command goes through it (W2c's file in `project` or
`context_server`). A server that fails under the sandbox shows "blocked by sandbox" with the denied
path from `log stream --predicate 'sender=="Sandbox"'` hints, and a per-server
`"sandbox": false` escape hatch in settings (logged, shown in the privacy report 7.6).

### 3.3 Per-project egress list

`.fyx/egress.json` in the project root (read-only to agents: added to `protected_paths`):

```json
{ "hosts": ["api.github.com", "*.crates.io", "registry.npmjs.org"] }
```

Read by W2c's settings glue; empty or missing means "no project list" (upstream behaviour). When
present, every sandboxed command in the project gets `Restricted { allowed_domains: list ∩ grants }`
(intersection, so a thread grant cannot widen it). Editing it is a user action (the review panel
holds agent edits to it like 7.3 holds secrets).

### 3.4 Per-secret host list (7.4)

Each vault entry gets optional `hosts` (stored in the vault index, not with the value):
`secret://github` → `["api.github.com", "*.githubusercontent.com"]`. Edited in "agent review:
manage secrets". When a command receives secrets `S1..Sn`, its network policy becomes
`policy ∩ hosts(S1) ∩ … ∩ hosts(Sn)`; a secret with no list keeps the current behaviour but the
approval prompt says "may be sent to any allowed host". If the intersection is empty the command
is refused before it starts with "secret://github may only go to api.github.com". The proxy logs
each refused host with the secret names (never values) to the 7.4 journal.

Limit (shown in the prompt, as 7.4 does): a command allowed `api.github.com` can still put the
token in a GitHub gist. The list stops sending it to other hosts, not misuse of an allowed one.
Without a sandbox (Windows today, `allow_unsandboxed`), a secret with a host list is refused.

## 4. Tests

- `crates/sandbox`: string tests for `read_rules` (any OS); on macOS CI-less, the owner runs the
  existing `#[cfg(target_os = "macos")]` exec tests plus new ones: `cat ~/.ssh/known_hosts`
  fails, `cat <worktree>/README.md` works, `ls /usr/bin` works.
- `ui_ext_wasm`: the spawn path builds the expected `SandboxPolicy` from a manifest (pure, any OS).
- Policy intersection (project list, grants, secret hosts): pure unit tests in W2c's file.
- 7.4: fake vault with host lists, a command with two secrets gets the intersection; an empty
  intersection is refused before spawn. Fake values only.

## 5. Mac checks for the owner (done-when)

1. In a project with `.fyx/egress.json` = `["api.github.com"]`, the agent runs
   `curl -sI https://api.github.com` (works) and `curl -sI https://example.com` (refused by proxy).
2. The agent runs `cat ~/.ssh/known_hosts`: "Operation not permitted".
3. A Fyx extension with `process:spawn` for `git` runs `git status` (works) and cannot read a file
   outside the worktree.
4. An MCP server (`npx @modelcontextprotocol/server-filesystem <worktree>`) starts and lists the
   worktree; pointed at `~`, it gets denials.
5. `secret://github` with hosts `api.github.com`: `curl -H "Authorization: Bearer
   secret://github" https://example.com` is refused before it runs.

## 6. Owners

| Owner | Work |
|---|---|
| W2c | 3.1–3.4 code, tests that run on any OS |
| W5 | 3.4 vault fields and the prompt text in `agent_review` |
| W0 | No Cargo change expected (`sandbox` and `http_proxy` are already in the workspace); adds `sandbox` as a dependency of `ui_ext_wasm` if W2c asks |
| Owner | Section 5 on a Mac |
