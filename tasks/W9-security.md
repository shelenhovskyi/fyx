# W9 Security modes for agents

**Owns:** `crates/fyx_guard/`, `docs/fyx/security-modes.md`, the `fyx_security` settings content in
`crates/settings_content/src/fyx.rs`, one-line `fyx:` hooks listed in the design doc (H1–H11).

**Design:** [docs/fyx/security-modes.md](../docs/fyx/security-modes.md). Every mode is off by
default and identical to Zed while off; only user settings raise it; `.fyx/security.toml` only
tightens; `audit` logs and blocks nothing; a protection that cannot start fails closed.

## Steps

1. **Quick wins** (`"mode": "shield"` / `"audit"`). Branch `claude/fyx-w9-guard`.
   - `check_private_path`: Zed's `private_files` on ACP `fs/read_text_file` (H1).
   - `scrub_env`: secret variables stripped from agent terminals, `allow_env` override (H4, H5).
   - `check_config_write`: `.claude/`, `.codex/`, `.mcp.json`, `.zed/`, `.fyx/`, `.git/hooks/`
     read-only for ACP `fs/write_text_file` and the built-in agent's edit tools (H2, H3).
   - `.fyx/security.toml` with tighten-only `private_files` / `read_only`.
   - `init` from `crates/zed` (H6); stubs `wrap_agent` / `wrap_mcp`, not wired.
   - **Done when:** in shield, an ACP agent's read of `.env` fails and its write to
     `.claude/settings.json` fails; with the mode off both work as in Zed.
2. **Process-tree isolation for ACP agents and MCP servers** (mode 4). Wire H8 and H9.
   - macOS Seatbelt and Linux bwrap with deny-read outside the project, toolchains and the agent's
     own state directory; reuse `crates/sandbox` where it fits, but its `SandboxFsPolicy` allows
     reads everywhere, so it needs a deny-read policy.
   - Windows: MXC process container via the `mxc-sdk` crate; raw AppContainer as fallback. A
     restricted token cannot deny reads.
   - Fail closed: if the sandbox cannot start, the agent does not start; the error names the reason.
   - Resolve the `context_server` → `fyx_guard` dependency cycle (design doc, H9).
   - **Unverified:** MXC's minimum Windows build; whether it needs admin rights; whether it filters
     the network per host.
3. **Egress control** (mode 2), through `crates/http_proxy`.
   - Allowlist per agent; approval prompt for a new host (once / always / deny).
   - Block direct DNS/UDP and `169.254.169.254` (cloud metadata).
   - Overwrite `Authorization` on requests to LLM hosts, so an agent cannot send a key it read to
     another account on the same host.
   - No auto-rendering of remote images and links in agent output (H11).
4. **Phantom secrets** (mode 3).
   - First: a base-URL reverse proxy. `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` / `GITHUB_API_URL`
     point at it; the agent sees format-like placeholders (`sk-ant-fyx-phantom-…`); real values come
     from the OS credential store and are substituted only on requests to the bound host.
   - Later: TLS termination with a sandbox-only CA, for tools that do not honour base URLs.
5. **Canary keys and transcript redaction.**
   - Canary credentials in the agent environment and a decoy `.env`; any use alerts and stops the
     turn.
   - Redact known secret values from transcripts, tool output, logs and exports.
   - **Unverified:** the CVE numbers cited as motivation; collect and check them against NVD before
     they go into the docs.

## Rules

- Fyx code only in `crates/fyx_guard`; upstream gets one-line `// fyx:` hooks.
- `Cargo.toml` / `Cargo.lock` changes are listed for the integrator.
- Each step lands with unit tests in `fyx_guard` and one test at its hook.
