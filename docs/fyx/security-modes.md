# Security modes for agents

Fyx can protect your secrets and your machine from the coding agents it runs: the built-in agent,
external ACP agents (Claude Agent, Codex, Gemini CLI, …) and the MCP servers they use. Every
protection is **off by default**, and with the mode off Fyx behaves exactly like Zed. You turn a
mode on knowingly, in your user settings.

Code: `crates/fyx_guard`. Task file: [`tasks/W9-security.md`](../../tasks/W9-security.md).

## Threat model

An agent works with your user's rights. A prompt injection in a README, an issue, a web page or a
dependency can make it:

1. **Read secrets**: `.env`, keys, cloud credentials, tokens in the shell environment.
2. **Exfiltrate them**: a `curl` in the terminal, a fetch, a DNS lookup, a remote image or link
   rendered in the chat, an MCP server that talks to the internet.
3. **Persist**: write to its own config (`.claude/settings.json` hooks, `.mcp.json`, `.codex/`,
   `.zed/` tasks, `.git/hooks`) so that code runs later, outside any approval flow.

Prior art: the "repository config runs code" class has had public CVEs in several agent CLIs and
editors in 2025–2026. **Unverified:** the exact CVE numbers are not collected yet; this doc must
not cite one until it is checked against the NVD entry.

What Zed already has, and its gaps (checked on the current tree):

| Zed feature | Covers | Gap |
|---|---|---|
| `private_files` (`**/.env*`, `*.pem`, `*.key`, `*.cert`, `*.crt`, `secrets.yml`) | built-in agent's `read_file` / `list_directory` (`crates/agent/src/tools/read_file_tool.rs`) | ACP `fs/read_text_file` does not check it (`crates/acp_thread/src/acp_thread.rs`, `read_text_file`) |
| `crates/sandbox` + `crates/http_proxy` | the built-in agent's terminal and fetch, behind the `sandboxing` feature flag | reads are allowed everywhere (`SandboxFsPolicy`); external ACP agents and MCP servers are not wrapped; on Windows WSL + bwrap only, network all-or-nothing (`windows_wsl.rs`) |
| Sensitive-settings prompt | built-in agent's edits of `.zed/` and global settings | does not cover `.claude/`, `.codex/`, `.mcp.json`, `.fyx/`, `.git/hooks`; external agents' `fs/write_text_file` is not covered |
| Terminal environment | — | agent terminals inherit the full shell + direnv environment (`crates/acp_thread/src/terminal.rs`, `create_terminal_entity`) |

An external ACP agent also reads and writes files with its own tools, straight from disk, without
asking Fyx. Modes 1–3 cover what passes through Fyx (the ACP client methods, terminals Fyx starts,
the network Fyx proxies); only mode 4 confines what the agent process does on its own.

## Modes

Each mode includes everything below it.

| # | Mode | What it adds | Status |
|---|---|---|---|
| 0 | **off** | nothing; Zed's behaviour | default |
| 1 | **secret shield** | `private_files` enforced on ACP reads; secret variables stripped from agent terminals; agent configs read-only for agents | step 1 (this branch), setting value `"shield"` |
| 2 | **egress control** | agent processes, terminals and MCP servers reach the network only through `http_proxy`, with an allowlist and approval of new hosts; DNS/UDP and `169.254.169.254` blocked; `Authorization` overwritten on LLM hosts | step 3 |
| 3 | **phantom secrets** | agents only ever see placeholders; the real value is substituted on the wire for the bound host | step 4 |
| 4 | **isolated** | the agent and MCP process trees run in an OS sandbox that denies reads outside the project | step 2 |

`audit` is mode 1 as a dry run: everything that would be blocked is logged (`log::warn!` with a
`fyx_guard audit:` prefix) and nothing is blocked. Use it to see what shield would break before
turning it on.

Step 2 (isolation) is built before step 3 (egress) because egress control is only enforceable once
the process cannot open its own sockets; the numbering of the modes follows strength, not build order.

### Cross-cutting items

- **Read-only agent configs** (step 1): agents may not write `.claude/`, `.codex/`, `.mcp.json`,
  `.zed/`, `.fyx/` or `.git/hooks/`, anywhere in the absolute path (so also `~/.claude/` for the
  built-in agent, which can edit files outside the project). Matching is by path component after
  resolving `.` and `..`, and ignores ASCII case, so `.Claude/settings.json` on a case-insensitive
  file system is caught too.
- **Redaction** (step 5): known secret values (from the OS credential store, the stripped
  environment, matched `private_files`) are replaced in transcripts, tool output shown to the model,
  logs and exported threads.
- **Canary keys** (step 5): fake credentials in the agent's environment and in a decoy `.env`; any
  use of one (seen by the egress proxy or by a canary service) raises an alert and stops the turn.
- **No auto-rendering of remote content** (step 3): remote images in agent output are not fetched,
  links show their full target and need a click, so a rendered markdown image cannot carry a secret
  out in its URL.

## Settings

```jsonc
// ~/.config/fyx/settings.json
{
  "fyx_security": {
    "mode": "shield",              // "off" (default) | "audit" | "shield"
    "allow_env": ["GITHUB_TOKEN"]  // stripped variables an agent terminal may keep
  }
}
```

Rules:

- **Only user settings may raise the mode.** `fyx_security` is a top-level key of the user settings
  file; a project's `.zed/settings.json` is parsed as project settings, which have no such key, so a
  repository can neither turn a mode on nor off.
- **A repository may only tighten.** `.fyx/security.toml` at a worktree root can add patterns,
  never remove protections, set the mode or allow a variable (below). It has no effect with the mode
  off.
- **Unknown values fail closed.** A `mode` string Fyx does not know (a typo, a mode from a newer
  build) is treated as `shield` and logged as an error.
- **Fail closed when a protection cannot start.** A `.fyx/security.toml` that cannot be read or
  parsed, or that is still loading, denies agent reads and writes in its worktree under shield (and is
  logged under audit). From step 2 on, an agent whose sandbox cannot start is not started.

### `.fyx/security.toml`

```toml
private_files = ["config/credentials/**", "*.p12"]  # extra private paths, relative to the worktree
read_only     = ["infra/**", "Makefile"]            # extra paths agents may not write
```

Patterns use the same matcher as Zed's `private_files` (`util::paths::PathMatcher`, matched
against the path and each ancestor, relative to the worktree root). Any other key, including `mode`
and `allow_env`, makes the file invalid, which fails closed.

## Environment scrubbing

Applied to the environment an agent terminal inherits (shell + direnv), before the variables the
agent passes for the command are added. Names are compared in ASCII upper case.

A matching variable is **set to an empty value**, not removed. `Project::create_terminal_task`
resolves the directory environment a second time and the PTY inherits Fyx's own process
environment, so a removed key would come back; an explicit empty value overrides both. The names
of Fyx's own process environment are therefore checked as well. Tools treat an empty `GH_TOKEN`,
`AWS_*` or `SSH_AUTH_SOCK` as unset.

| Pattern | Examples |
|---|---|
| `AWS_*` | `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AWS_PROFILE` |
| `GITHUB_TOKEN`, `GH_TOKEN`, `NPM_TOKEN` | |
| `*_API_KEY` | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` |
| `*_SECRET*` | `CLIENT_SECRET`, `DJANGO_SECRET_KEY` |
| `*_TOKEN` | `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_OAUTH_TOKEN` |
| `SSH_AUTH_SOCK` | the SSH agent: in shield, `git push` over SSH from an agent terminal stops working; allow it with `allow_env` if you want that |

`allow_env` entries are names or `*` patterns and win over the list. Variable names only are
logged, never values.

## Known limits of step 1

Shield covers what passes through Fyx; it is not a sandbox. Until step 2:

- An external agent's own tools (Claude Agent's `Read`, `Bash`, …) read and write the disk
  directly, without `fs/read_text_file`.
- A terminal command can still `cat .env`, read `~/.aws/credentials`, or re-export a variable from a
  shell startup file (`.zshenv` runs for every zsh).
- Paths are checked lexically: a symlink named `notes` that points to `.claude/` is not caught.
- `.fyx/security.toml` is not read from remote worktrees; if one exists there, the worktree fails
  closed under shield.

## Hooks

The whole upstream surface is listed by `git grep -n 'fyx:'`. Every hook is a no-op with the mode
off.

| # | Upstream file | Hook | Step |
|---|---|---|---|
| H1 | `crates/acp_thread/src/acp_thread.rs`, `read_text_file` | `fyx_guard::check_private_path(project, &path, cx)?` | 1 |
| H2 | `crates/acp_thread/src/acp_thread.rs`, `write_text_file` | `fyx_guard::check_config_write(project, &path, cx)?` | 1 |
| H3 | `crates/agent/src/tools/edit_session.rs`, before `authorize` | `fyx_guard::check_config_write(..)` for the built-in agent's edit and write tools | 1 |
| H4 | `crates/acp_thread/src/terminal.rs`, `create_terminal_entity` | `fyx_guard::scrub_env(&mut env, cx)`: terminals of external ACP agents | 1 |
| H5 | `crates/acp_thread/src/acp_thread.rs`, `create_terminal` | `fyx_guard::scrub_env(&mut env, cx)`: the built-in agent's terminal | 1 |
| H6 | `crates/zed/src/main.rs` | `fyx_guard::init(cx)` | 1 |
| H7 | `crates/settings_content/src/fyx.rs` (Fyx file) + one field in `settings_content.rs`, one line in `settings/src/vscode_import.rs` | the `fyx_security` settings key | 1 |
| H8 | `crates/agent_servers/src/acp.rs`, `connect` | `fyx_guard::wrap_agent(command, cx)?` | 2 (stub exists, not wired) |
| H9 | `crates/context_server/src/transport/stdio_transport.rs`, `StdioTransport::new` | `fyx_guard::wrap_mcp(command, cx)?` | 2 (stub exists, not wired) |
| H10 | `crates/http_proxy` | allowlist + approval callback, LLM-host `Authorization` overwrite | 3 |
| H11 | `crates/markdown` (agent output only) | no remote image fetch, explicit links | 3 |

H9 needs care: `fyx_guard` depends on `project`, which depends on `context_server`, so
`context_server` cannot call `fyx_guard` directly. Either the wrapper is installed as a GPUI global
by `fyx_guard::init` and called through a two-line module in `context_server` (the pattern of
`acp_thread::fyx_review`), or `fyx_guard` is split into a core crate without the `project`
dependency. Decide in step 2.

## Opt-in UX

1. Off by default; nothing changes, no banner, no prompt.
2. The user sets `fyx_security.mode` in their user settings (later also a toggle in the agent panel's
   options menu, which writes the same setting, like "Review Agent Edits").
3. On start and on every change, Fyx logs the active mode once; the agent panel shows a small shield
   indicator while a mode is on (later step).
4. A blocked action fails with a message to the agent that names the rule ("matches `private_files`",
   "agent configs are read-only in Fyx's shield mode") and tells it not to retry, and is logged with
   the path or variable name.
5. Recommended path: run `audit` for a while, read the log, add `allow_env` entries, then switch to
   `shield`.

## Platform notes for step 2

- macOS: Seatbelt profile with `deny file-read*` outside the project and the toolchain, generated
  per agent; `crates/sandbox/src/macos_seatbelt.rs` is the starting point.
- Linux: bubblewrap with the home directory replaced by an empty tmpfs and the project bound
  read-write; `crates/sandbox/src/linux_bubblewrap.rs`.
- Windows: the MXC process container through the `mxc-sdk` crate; raw AppContainer as fallback. A
  restricted token alone cannot deny reads, so it is not an option. **Unverified:** the minimum
  Windows build MXC needs, whether creating a container needs admin rights, and whether MXC can
  filter the network per host (if not, egress on Windows goes through the proxy with all other
  network denied).
