# P5 security check of the fyx:ui 0.4 host (plan 5.19)

W2c, 2026-11-20, on integration 212119f189 plus branch `claude/fyx-w2c-host-0-2-cbde2c`.

## Verdict

**WIT 0.4 is safe to freeze.** No finding needs a WIT change. Every fix below is in the
manifest rules, the consent lines or the host. They can change after the freeze without an ABI
change.

## Scope

- `ui_ext`: the terminal, integration and process rules.
- `ui_ext_wasm`: listeners, relay, terminals with T3, editor context, diagnostics, proposals,
  consent items and `check_world`.
- Read, not changed: `terminal_host` (discovery writer, `profiles.rs` env resolution),
  `ui_ext_host` (`is_private_path`), and the registry template workflows.
- Re-checked integration-6's open items in code.

## Earlier findings, re-checked

| # | State | Where |
|---|---|---|
| N1 | **Closed** before this check (4458b3abd9). `[fyx_ui.process].env` is an allowlist (`ALLOWED_ENV_NAMES`, `LC_*`). `terminal:env` stays a denylist by design (§B.4) and is widened here (P1, P2). | `ui_ext/src/process.rs` |
| N3 | **Closed** (W10). The decision reads `[fyx_ui].component`. `publish.yml` runs `publish-check` and refuses a T2 or Zed-extension release without a T2 decision and the owner's approval. | `registry.py:298-357`, `publish.yml:81-101` |
| N4 | **Closed**, both halves. The block-list floor survives a cache drop. An expired list with no cache is refused. Blocks lift only on `blocks_can_lift` plus a fresh list. | `blocklist.rs:13-16,447-497`, `client.rs:310`, `fyx_extensions_page.rs:593-599` |
| R8 | **Closed.** `urgent` skips the wait only when the active version is blocked. | `update.rs:21-24,381-395`, test `age_gate_waits_24_hours_unless_urgent` |
| R9 | **Closed**, all the blocklist items: <br>• sequence floor and expired list (as N4); <br>• hash-only blocks match installs with no recorded hash (`is_none_or`, `blocklist.rs:319-327`); <br>• role separation is checked once on the compiled-in keyset (`signing.rs:188-215`, recorded there); <br>• "daily" is left only for snapshots, which are daily. | `fyx_registry` |
| S6 | **Closed** by N1's allowlist. | `ui_ext/src/process.rs` |
| M2/N5 | **Closed** (W3). `impl UiBackend for UiExtension` forwards `decoration_closed`. | `ui_ext_host/src/backend.rs:279` |
| R7 | Still partial by decision: process `args`/`env` wait until T2 opens in the registry. | — |

## New findings

| Sev | # | Finding | Fix | State |
|---|---|---|---|---|
| **High** | P3 | **`terminal:env` with `${package}` runs code without a warning.** `CLAUDE_CODE_PLUGIN_DIRS=${package}/claude-plugin` makes Claude Code load the package's plugin. A plugin's hooks are shell commands that Claude runs with no prompt. The same holds for any tool given a folder of the extension's files. The dialog said only "makes Claude Code load this extension's plugins". | The `terminal:env` line (and its all-new line) adds "It can run any program on your computer" when a value uses `${package}` or a name in `PLUGIN_ENV_NAMES`. | **Fixed** (`consent_items.rs` `env_lines`; test `package_env_and_hook_answers_run_any_code`) |
| **High** | P4 | **Hook answers approve tools' actions.** A `relay:hooks` answer is the hook's stdout and exit code. Claude Code's PreToolUse takes `permissionDecision: "allow"` from it and runs the tool, Bash included, without asking. UserPromptSubmit and SessionStart add the stdout to the model's context, which is prompt injection. The dialog warned only about sending reads. | The relay line now warns "It can run any program on your computer" and "Its answers can approve what those tools run and add to your prompts". | **Fixed** (same test) |
| Medium | P5 | **A T1 profile can be `bash -c "<anything>"` behind a friendly name.** It runs on one click in the "+" menu. The install dialog listed only "Adds terminal profiles". | The T1 line lists each profile's command line. It warns "can run any program" when one names a `RUNS_ANY_CODE` program or passes `${package}`. | **Fixed** (`profile_lines`; test `profiles_show_their_command_lines`). **W3:** the menu's "runs any command you type" mark should say "runs the command in its arguments" when the args hold `-c`/`-e`/`/c`. |
| Medium | P1 | `terminal:env` missed names that run code in the user's shell: <br>• prompt expansion `PS0`–`PS4`, `PROMPT`, `RPROMPT`, `RPS1`/`RPS2` (`PS1='$(cmd)'` runs `cmd`); <br>• `HISTFILE=~/.bashrc`, which appends typed lines to it; <br>• loaders and package sources: `CLASSPATH`, `GCONV_PATH`, `GOFLAGS`, `GOENV`, `GOPROXY`, `GOTOOLCHAIN`, `GOINSECURE`, `GONOSUMDB`, `GONOSUMCHECK`, `GOPRIVATE`, `MAVEN_OPTS`, `GRADLE_OPTS`, `PYTHONWARNINGS`, `PERL5DB`, and the `GIT_`, `LUA_INIT`, `GEM_`, `BUNDLE_`, `PIP_` and `UV_` prefixes. | Refused at load (`SHELL_ENV_NAMES`, `FORBIDDEN_ENV_*`). | **Fixed** (test `forbidden_terminal_env_names`) |
| Medium | P2 | `terminal:env` could move a tool's traffic, with the user's key in it: <br>• `HTTP(S)_PROXY`, `ALL_PROXY`, `FTP_PROXY`; <br>• CA stores: `SSL_CERT_*`, `NODE_EXTRA_CA_CERTS`, `NODE_TLS_REJECT_UNAUTHORIZED`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`; <br>• endpoints: `*_BASE_URL`, `*_API_BASE`, `*_API_URL`, `*_ENDPOINT` (`ANTHROPIC_BASE_URL`). | Refused at load. | **Fixed** (same test) |
| Medium | P6 | **Template values can point outside their folder.** `${package}/../..` named files outside the extension under "files from this extension". `${account.<tool>}/../<other>` named another account's dir, which the extension must never choose (§C.6 rule 5). | `Template::parse` refuses a `..` component after `${package}` or `${account.…}`. | **Fixed** (test `no_parent_after_package_or_account`) |
| Medium | P7 | **`is_private_path` fails open.** It returns false when `EditWorkspaces` is not set, or when no registered project holds the path. Context and diagnostics then treat the file as not private. The timeouts already fail closed. | Return true (private) when no registered project resolves the path, and keep worktree checks in the callers. | **W3** (`ui_ext_host/src/workspace_edit.rs:105-116`) |
| Low | P8 | The discovery writer's `remove_stale` deletes every path in its registry file. The file is in Fyx's data dir, and `remove_file` refuses links and folders. A corrupted or planted registry still deletes arbitrary plain files of the user. | Remove only names that match a discovery name (`.lock`/`.json` with the run's pid or port) under a known target dir. | **W1** (`terminal_host/src/discovery.rs:416-440`) |
| Low | P9 | `default_dir` may be any folder under `~/`. The name must hold `${port}`/`${pid}` and end in `.lock`/`.json`, so no fixed settings file can be written. Still, folders such as `~/.config/autostart` are reachable. | Registry review flags a `default_dir` outside the tool's known dir. No host change. | W10 (review rule), noted |
| Medium | P11 | The discovery writer refused links and other owners in a shared `/tmp`, but not a folder of the user's that others may write to (W4's Gemini gap 3). Another user could swap a file or folder below it. | `check_owned_dir` also refuses group or other write (`mode & 0o022`). New folders were already 0700 and files 0600. On Windows `%TEMP%` is per-user, and ACL write rights are not checked. | **Fixed** (`terminal_host/src/discovery/safe_fs.rs`, unix test `a_folder_others_may_write_to_is_refused`; compiled for Windows only, the Linux build needs a gcc this machine lacks) |
| Info | P10 | Writes to an extension's own created or pseudo terminal need a gesture but no card (§C.3, by design). A created terminal running an agent with its own permissions is then steered by the extension after any click in its UI. | Design decision; keep. The P4 warning now covers the strongest case. | — |

Also checked, with no finding:
- **Listeners:** `127.0.0.1:0` only; 256-bit token compared in constant time; `Origin` refused;
  `Host` pinned to `127.0.0.1`/`localhost` with the port (DNS rebinding); tests cover each.
- **Discovery files:** dir under `${account}`/`${tmp}` with plain components; private dirs and
  files created owner-only; links and foreign owners refused; atomic rename.
- **`terminal:env` resolution:** only declared names are added. User tool env names and refused
  names are dropped again at resolve time (`terminal_host/src/profiles.rs:163-176`).
- **T3:** cards for other terminals; untrusted-worktree refusal; journal; gesture for writes.
- **Context and diagnostics:** worktree files only, private and account-dir files left out.
  `fyx_guard::extension_read_denial` also refuses links out of the project.
- **`check_world`:** 0.4 lacked `workspace:diagnostics` (fixed in 53d48fc363). The test
  `test_shipped_manifests_pass_their_worlds` now reads every shipped manifest in each world it is
  built for.

## Owner actions

- **W3:**
  - P7;
  - the P5 menu mark.
- **W1:** P8.
- **W10:**
  - P9's review rule;
  - `registry.py comment` has its own consent lines (`registry.py:202`), so mirror P3/P4/P5's
    warnings there.
- **W0:** nothing for WIT.
