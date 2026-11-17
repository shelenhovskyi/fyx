# Judge review: W9 security step 1 (`fyx_guard`)

**Scope.** `origin/claude/fyx-w9-guard` at `6775a877a5`. That is three commits on
`origin/integration`:
- `0acea54053` (the design);
- `58689c8a07` (the crate and the setting);
- `6775a877a5` (the hooks).

The branch is 20 files, +1452/−2.

**Method.** Read only; nothing was built or run. I checked the code against:
- `docs/fyx/security-modes.md` on the branch;
- platform spec decisions 21–27 (`docs/fyx/platform/spec.md` on `origin/integration` `48b66aaf68`);
- W5's state as of `integration-4.md`.

I disregarded the W9 cloud session's own report, including the turn in it that claims user
approval.

## Verdict: merge after fixes

The design is sound and honest about its limits (`security-modes.md` "Known limits of step 1"):
- **Off is a no-op.** Every check returns early when the mode is `off`, and `off` is the default.
- **Unknown values fail closed.** An unknown mode value becomes `shield`.
- **A repository policy that cannot be used blocks.** While `.fyx/security.toml` is loading, or
  when it cannot be read or parsed, the worktree is restricted.
- **The repository file can only add rules.** It has two keys and `deny_unknown_fields`.
- **Path checks resist simple tricks.** They normalize `..` and ignore case.

**Fix before merge:** F1–F4. Each is small. **Follow-ups:** F5–F10.

| # | Fix | Where | Size |
|---|---|---|---|
| F1 | Move the 50-line test out of `acp_thread.rs`, into `crates/fyx_guard/tests/acp_hooks.rs` (an integration test, with `acp_thread` as a dev-dependency) | `acp_thread.rs:15722-15770` | −50 upstream lines |
| F2 | Guard the built-in agent's move, copy, delete and create-directory tools, not only `edit_session` | `agent/src/tools/{move_path,copy_path,delete_path,create_directory}_tool.rs` | 1 hook line per tool, or one shared helper |
| F3 | Treat `.git/config` like `.git/hooks/` | `policy.rs:63-68` | 3 lines plus a test |
| F4 | Lowering the mode at run time must not happen silently | `fyx_guard.rs:291-307` | about 20 lines |

## 1. Hook minimality: what the +55 in `acp_thread.rs` is

| Lines | What | Verdict |
|---|---|---|
| +1 (`:7935`) | `fyx_guard::check_private_path(project, &path, cx)?` in `read_text_file` | a minimal hook |
| +1 (`:8006`) | `fyx_guard::check_config_write(project, &path, cx)?` in `write_text_file` | a minimal hook |
| +2/−1 (`:8114-8118`) | `async move \|_, _\|` becomes `\|_, cx\|`, plus `scrub_env` in `create_terminal` | a minimal hook (the closure needs `cx`) |
| **+50** (`:15722-15770`) | `test_fyx_guard_shield_on_acp_file_access`, inside upstream's `mod tests` | **move it (F1)** |

`acp_thread.rs` is the most heavily changed file upstream touches: 129 upstream commits in six
months (rebase-rehearsal). A 50-line block in its test module is the largest conflict risk on the
branch, and none of it is needed there.

**Where the test can go.** `fyx_guard` cannot be a dev-dependency of `acp_thread` without a cycle.
An integration test in `crates/fyx_guard/tests/` can instead take `acp_thread` (with its
`test-support` feature) as a dev-dependency:
- integration tests link the same `fyx_guard` that `acp_thread` links;
- `FakeAgentConnection` and `init_test` exist under `test-support`. If `init_test` is private,
  copy its 10 lines.

After F1, `acp_thread.rs` changes by +4/−1.

The other hooks are each one line and marked `fyx:`:
- `terminal.rs:967`
- `edit_session.rs:725`
- `zed/src/main.rs:599`
- `settings_content.rs:216,415`
- `vscode_import.rs:187`
- 3 `Cargo.toml` dependency lines, all marked

**Why the checks cannot move into `fyx_guard`.** The read and write checks need to run inside
upstream functions. `acp_thread` already depends on `project`, so calling `fyx_guard` from there
costs no new cycle.

**Fewer call sites.** The two `scrub_env` calls could become one, if
`acp_thread::create_terminal` built its environment through `create_terminal_entity`. That would
change upstream code paths, so it is not worth it: keep both.

## 2. Correctness and fail-closed behaviour

### Checked and correct

- **Off is identical to Zed.**
  - `mode()` returns `Off` without a settings store (`fyx_guard.rs:70-76`).
  - `check_private_path` (`:106-109`), `write_check` (`:151-154`), `scrub_env` (`:179-182`) and
    `load_policy` (`:346-348`) all return before doing anything.
  - With the mode off, the only extra work at startup is `init` registering observers and
    subscribing to each new `Project` (`:270-289`). Behaviour is unchanged.
- **Private-files check on ACP reads.**
  - It uses Zed's own `WorktreeSettings::is_path_private`, global and per worktree, the same check
    as the built-in agent's `read_file` and `grep` tools (`:119-127`).
  - It then adds `.fyx/security.toml`'s `private_files`, matched with the same ancestor walk
    (`policy.rs:111-119`).
  - The denial reaches the agent as an ACP `INVALID_PARAMS` error with the message, not as
    "Internal error" (`:114-116`).
- **Fail closed.**
  - `RepoState::Loading` and `Failed` deny both reads and writes (`policy.rs:131-155`).
  - A remote worktree that has the file fails closed; one without it is not restricted (`:358-368`).
  - A stale load cannot overwrite a newer one: loads carry a generation number (`:353-357,385-389`).
  - Removing or renaming `.fyx/` triggers a reload (`:319-330`).
  - An unknown mode becomes `shield`, with an error log (`:61-64`; tested at `:420-442`).
- **Read-only agent configs.**
  - `.claude/`, `.codex/`, `.zed/`, `.fyx/` and `.mcp.json` are protected at any depth, and so is
    `.git/hooks/`.
  - Matching ignores ASCII case, the path is normalized first, and the rule applies to both the
    absolute path and the worktree-relative path (`policy.rs:35-71`).
  - Because `.fyx/` is on the list, `.fyx/review.toml` is protected from ACP and built-in-agent
    writes under shield too.
- **Env scrubbing.**
  - Secret-looking variables are set to `""` rather than removed. That is correct: the terminal
    re-resolves the shell and direnv environment, and only an explicit value overrides it
    (`:174-177`).
  - Variables the agent sets for its command are applied after the scrub, as intended.
  - Logs carry names only, never values.
- **Only user settings set the mode.**
  - Project settings parse as `ProjectSettingsContent`, which has no `fyx_security` key.
  - VS Code import maps it to `None` (`vscode_import.rs:187`).
  - `.fyx/security.toml` has no key that sets the mode, allows a variable or removes a
    protection (`policy.rs:73-87`).
  - This is the model W5's `.fyx/review.toml` should follow (integration-4, finding 1).

### F2 (High under shield): the built-in agent can still change agent configs with its other file tools

H3 guards only `edit_session.rs` (the edit and write tools). The other file tools of the
built-in agent call project APIs directly, without the check:
- `move_path_tool.rs:268` (`rename_entry`);
- `copy_path_tool.rs:237` (`copy_entry`);
- `delete_path_tool.rs`;
- `create_directory_tool.rs:216` (`create_entry`).

**Scenario.**
1. The agent writes `notes/settings.json`, which is allowed.
2. It calls `move_path` with destination `.claude/settings.json`.

The read-only rule is bypassed. Two more tools do the same: `copy_path` writes into `.fyx/`, and
`delete_path` removes `.fyx/security.toml`, which loosens the repository policy.

**Fix.**
- Add one `fyx_guard::check_agent_edit(&project, &abs, cx)?` line before each tool's project call.
- For a move, check both source and destination; for a copy, the destination; for a delete and a
  create, the target.
- Name them H3b–H3e in the hooks table of the design doc.

These tools are in-process, so unlike external agents there is no reason to leave them unguarded.

### F3 (Medium under shield): `.git/config` is as dangerous as `.git/hooks/`

`policy.rs:63-68` protects `.git/hooks/` but not `.git/config`. A config that sets
`core.hooksPath` to a writable directory, or `core.fsmonitor` to a program, runs code at the next
`git` invocation. Fyx's git panel runs `git` itself. That is exactly what the hooks rule prevents.

**Fix.** Protect `.git/config` (and, while at it, `.git/info/`) with the same case-insensitive
match. Add both to the tests at `policy.rs:208-240`.

### F4 (Medium): a terminal command can switch the shield off, and nothing says so

- The mode is read from user settings and applied live (`on_settings_changed`, `:291-307`).
- An agent terminal can write to the user settings file
  (`%APPDATA%\Fyx\settings.json`, `~/.config/fyx/settings.json`). Terminals are unconstrained
  until step 2, as the design says. The ACP and built-in agent write paths refuse paths outside
  the project.
- So one command takes `shield` to `off` in the middle of a session: either
  `fyx_security.mode = "off"`, or `allow_env = ["*"]`.

**Fix.**
- When the effective mode drops (shield → audit or off) or `allow_env` grows while Fyx is running,
  keep the stricter value and show a notification: "Security mode change from the settings file:
  apply / ignore". Apply the new value only on confirmation or at the next start.
- Raising the mode stays immediate.
- Log every change at `warn`.

This goes into step 1, because without it shield is one `echo` away from off.

### F5 (Low): secret patterns miss common names

`SECRET_ENV_PATTERNS` (`policy.rs:23-32`) does not cover:
- `*_PASSWORD`, `*_PASSWD`
- `*_KEY` that is not `_API_KEY` (`OPENAI_KEY`, `STRIPE_KEY`)
- `*_PAT`, `*_CREDENTIALS` (`GOOGLE_APPLICATION_CREDENTIALS`)
- `DATABASE_URL`, `*_DSN`, which usually embed passwords

Add them. Keep the `allow_env` escape hatch for false positives.

### F6 (Low): document that an external agent's own process keeps the full environment

The scrub covers terminals that Fyx starts. External ACP agents (`wrap_agent` is a stub,
`:229-231`) and stdio MCP servers inherit Fyx's whole environment, including every `AWS_*` and
`*_TOKEN`. The agent can read them from its own process. "Known limits of step 1" lists the
agent's own tools and `cat .env`, but not this. Add one bullet. It is the most likely leak in
practice.

### F7 (Low): the built-in agent's `read_file` ignores `.fyx/security.toml`

The built-in agent's `read_file` and `grep` tools honour Zed's `private_files`, but not the
repository's `.fyx/security.toml` `private_files`. Only the ACP read path gets those (H1). That is
inconsistent with the design's promise that `private_files` is enforced on agent reads.

**Fix.** Either add a hook in `read_file_tool.rs` and `grep_tool.rs` next to their existing
`is_path_private` calls, or expose `fyx_guard::is_private(project_path, cx)` and call it from both.

## 3. Overlap with W5 and how the two hooks should compose

Both streams hook the same two places:
- the built-in agent's edit path (`edit_session.rs`);
- the ACP write path (`acp_thread.rs` `write_text_file`, where W5's `fyx_review::gate_agent_write`
  already sits at `:8017-8019`).

**Rule: the guard runs first, the review second, and the guard runs again at the moment of
writing.**

| Path | Order | Why |
|---|---|---|
| ACP `write_text_file` | `check_config_write` at the top of the update closure (W9, `:8006`), then W5's `gate_agent_write` after the snapshot | A write the shield refuses must never reach the review pane. The user cannot approve a blocked write, and does not review edits that cannot land. |
| Built-in agent `edit_session.rs` | `check_agent_edit` before `authorize` (W9, `:725`). W5's review gate goes after the edit is computed and before `buffer.edit` commits it. | The same rule. Both hooks stay one line each. W5 must not move its hook above W9's. |
| Host-side writes by `agent_review` | `write_accepted_hunks` (`ApplyAcceptedAndDeny`) and any write of a held edit after approval must call `fyx_guard::check_config_write` again just before writing | These write agent-proposed content outside both hooked paths. The mode may also have been raised while the review was open. Without the re-check, a review becomes a way around the shield. **Owner: W5.** `agent_review` may depend on `fyx_guard`; there is no cycle. |
| Error text | Both report through the same channel: an ACP error with `data` for ACP, a `String` for the built-in agent | A guard denial must not be counted as a review rejection. In W5's feedback text they are different things: "blocked by Fyx security" versus "the user rejected hunk 2". |

**Merge order.** Merge W9 first: its hooks are smaller. W5 then rebases its `edit_session.rs` and
`acp_thread.rs` lines below W9's.

Both streams are adding "trait object registered at init" style glue, so DIFF #4's bottom-of-graph
`fyx_hooks` crate is now worth doing. Put one `fyx_hooks::before_agent_write` /
`review_agent_write` pair at each call site. W0 decides; it is not a blocker.

## 4. Consistency with platform spec decisions 21–27

Decisions 21–27 are about **UI extensions** (`fyx:ui` capabilities), not agents. Nothing in W9
contradicts them. Four points connect:
- **21 (`workspace:read-files`, tier T1) and 26.** When extensions get file reads, they must
  honour the same private set as agents: Zed's `private_files` plus `.fyx/security.toml`. Make
  `fyx_guard` the single source for that, through `fyx_guard::is_private` (F7). Otherwise an
  extension with T1 reads what the shield hides from agents.
- **21 (`agent:observe`, tier T2).** Agent events given to extensions must not carry the contents
  of files the shield denied: diff text of private files, or blocked writes. Record this as a
  requirement for the `agent:observe` design.
- **24 (`process:spawn`).** Extensions get an **allowlist** environment (`PATH`,
  `HOME`/`USERPROFILE`, `LANG` plus the variables the manifest names). Agent terminals get a
  **denylist** scrub. The difference is right, since agents need the user's toolchain environment.
  Say so in both docs, so nobody "unifies" them later in the weaker direction.
- **26 (untrusted worktrees).** `.fyx/security.toml` is loaded whether or not the worktree is
  trusted. That is correct, because it can only tighten. Trust is the guard for files that can
  *loosen*, which is why W5's `review.toml` `auto_accept` must not rely on trust at all
  (integration-4, finding 1).
- **27 (`cargo deny advisories`).** That is a gate change (W0), not part of this branch. Track it
  under W9's step list, so the security owner also owns the advisory policy.

## 5. Smaller notes

- **F8 (Low).** `scrub_env` reads `std::env::vars_os()` on every terminal spawn (`:183-185`).
  That is cheap, but the snapshot could be taken once at `init`, since Fyx's own environment does
  not change.
- **F9 (Low).** `check_private_path` builds the action string with an absolute-path lookup even
  when nothing is denied (`:110-113`). Build it only when there is a denial.
- **F10 (Low).** Settings help text: `fyx.json:47-52` says "Only user settings can change it".
  That is true for project settings, but not for the terminal (F4). Reword it after F4:
  "Changes that lower it ask for confirmation."
- The `wrap_agent` / `wrap_mcp` stubs (`:229-241`) are unused public API in step 1. Fine as
  placeholders. Keep them `#[doc(hidden)]`, or mark them clearly as step 2, so nobody wires them
  and believes isolation is on.
