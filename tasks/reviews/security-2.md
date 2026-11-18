# Security review 2: consent, install, packages, registry, review gate, shield

**Scope.** Everything added or changed since the audits in `tasks/reviews/` (network-audit-1,
license-audit-1, upstream-diff-audit-1, w5-deep-1, integration-4, triage-2, and W7's
`w9-step1.md`). Each branch was reviewed at its head:

| Area | Branch | Head |
|---|---|---|
| Review gate, `fyx_ext`, `ui_ext_wasm` (as merged) | `fyx/main` | `571c6826a5` |
| Interim consent, install path checks, depth cap, link filter, 256 MiB memory | `claude/fyx-w3-renderer-e20d2b` | `6ecd479` |
| `fyx_guard` with the W7 fixes F1–F5, F9, F10 | `claude/fyx-w9-fixes` | `0b3ac8b` |
| `fyx_registry` | `claude/fyx-registry-core` | `a5007f7` |
| `fyx_ext_harness`, `fyx-ext test` | `claude/fyx-w4-sdk-demos-5d1497` | `2b9b01c` |

File and line references are at those heads. On the scratch branch, the line numbers above each
test appendix are unchanged: tests were only appended, or added as new files.

**Threat actors:**
- a malicious UI extension (guest WASM, manifest, package);
- a malicious repository the user opens;
- a malicious or prompt-injected AI agent;
- a tampered registry index or package.

**Method.**
- **Merge.** A local scratch branch (`scratch/security-2`, never pushed) merged the four branches
  onto `fyx/main`.
- **Confirmed findings.** For each one I wrote a test there and ran it with `cargo test`. The test
  code is in appendix A, and the output it printed is in appendix B.
- **Theoretical findings.** These come from reading the code and were not reproduced.
- **Product code.** No product code changed on any pushed branch. Defensive only: the tests show
  that a check can be bypassed. They are not tools for doing it.

**Severity.** Shield (`fyx_security.mode`) and review (`agent_review.enabled`) are both off by
default. A finding rated High is High for a user who turned the feature on and relies on it.

## Verdict

The new code has sound structure:
- the consent store fails closed;
- install ids are validated and cannot reach `cmd`;
- registry signatures cover a canonical, injective encoding;
- the shield's repository file can only tighten.

The weak points are paths and freshness:
- **Lexical paths.** The shield and the review rules both decide on the path the agent typed. They
  never look at the file the write will reach. One committed symlink defeats both (S1).
- **Rules from disk.** The integration-4 #1 regression is still open: a project rules file the
  agent rewrote applies at the next thread start (S2).
- **Packaging.** `fyx-ext package` packs whatever an `assets` symlink points at (S3).
- **Freshness.** The registry design cannot detect a frozen index or a replayed block list
  (S5, S6).

## Top 5

1. **S1 (High).** Shield and review rules check the lexical path. A repository symlink lets an
   agent write `.git/hooks/pre-commit` and `.claude/settings.json` under shield, read a private
   `.env`, and write a file a `deny` rule covers.
2. **S2 (High, integration-4 #1 still open).** A `.fyx/review.toml` rewritten by the agent's
   terminal is applied without confirmation at the next thread start. With no trust global, the
   worktree counts as trusted, so `auto_accept = ["**"]` waves every later edit through.
3. **S3 (Medium; High once registry CI builds authors' repositories).** `fyx-ext package` follows
   an `assets` symlink and ships files from outside the extension inside the package.
4. **S5 and S6 (Medium).** A replayed block list stays "fresh" for ever, and the index as a whole
   is unsigned. A mirror can withhold a block entry or a security fix without being noticed.
5. **T2 (Medium).** The shield's read-only list misses other agents' configs and Fyx's own task
   sources: `.gemini/` (MCP server commands), `.vscode/tasks.json` and `launch.json`, `.agents/`
   skills, and the rules files Fyx feeds to every agent (`AGENTS.md`, `CLAUDE.md`, …).

## Status of earlier findings (re-checked)

| Earlier item | Status at the heads above |
|---|---|
| integration-4 #1: project `review.toml` loosens review | **Open, confirmed (S2).** Thread start still calls `set_rules` from disk (`agent_review.rs:145,1399-1402`). Trust still fails open (`:1367-1372`, `None => true`). A read error still loads as "no rules" (`:1394`). |
| integration-4 #2: id to `cmd /C mklink`, traversal | **Fixed** (W3 `6ecd479`): `validate_id` in `UiManifest::read` (`ui_ext_wasm.rs:244`), `link_target` (`install.rs:62-72`), `CMD_SPECIAL` on both paths (`:47-58,98-106`). |
| integration-4 #3: installed extensions load without consent | **Implemented (W3 `6ecd479`).** What remains is S4, S7 and T1. |
| integration-4 #7: store limits do not bound host memory | **Partly fixed.** There is now a 16 MiB per-call cap on pushed and logged text (`ui_ext_wasm.rs:134`). Still open: <br>• 8 memories × 256 MiB, so 2 GiB per guest (`:141,147`); <br>• reply trees from `mount`/`event` are not counted toward the cap; <br>• no byte cap on the retained tree; <br>• an unbounded WASI `ResourceTable` (`:319,340`). |
| w9-step1 §3 row 3: host-side writes must re-check the guard | **Open.** `write_accepted_hunks` (`agent_review.rs:1468`) writes agent-chosen content to an agent-chosen path, and `agent_review` does not depend on `fyx_guard` yet. Fix it once both are merged. |
| w9-step1 F7: built-in `read_file`/`grep` ignore `.fyx/security.toml` | **Open.** `3851a95` lists F2–F5, F9 and F10. `git grep fyx_guard:: crates/agent` shows no read hook. |
| w9-step1 F2, F3, F4 | **Fixed** (`3851a95`), apart from T3 below. |
| triage-2 W9: environment-armed features | **Open.** `FYX_AGENT_REVIEW=0` overrides the user setting (`agent_review.rs:62-63,84-93`). `FYX_UI_DEV_EXTENSIONS` loads without consent (`consent.rs:10`). New scenario: a repository's `.envrc` exports either variable, and the user starts Fyx with `fyx .` from a direnv shell while no instance is running, so the app inherits it. Fix: honour both only in dev builds, or only when Fyx was not started from a project directory, and show a banner while either is in effect. |
| w5-deep H2 (built-in agent not reviewed), H3 (terminal writes not blocked) | Open by design and stated in the UI (`NOT_REVIEWED`, `agent_review.rs:59-60`). Not repeated here. |
| w5-deep M4: stale base | **Fixed.** `gate_agent_write` refuses when the buffer version changed, or the file was moved or deleted, during review (`fyx_review.rs:79-104`). |

## Confirmed findings

### S1. High: the shield and the review rules check the path the agent typed, not the file the write reaches

**Where.**
- **Shield (`claude/fyx-w9-fixes`):**
  - `protected_config` and `protected_config_rel` match path components after a lexical
    `normalize_path` (`policy.rs:44-74`). `write_denial` uses them (`fyx_guard.rs:280-297`).
  - `private_denial` checks the project path string (`fyx_guard.rs:140-148`).
  - The hooks are at `acp_thread.rs:7935` (read) and `:8006` (write).
- **Review rules (`fyx/main`):**
  - `ReviewRules::evaluate` takes the agent's absolute path, `strip_prefix`es the root, and globs
    case-sensitively (`review_rules.rs:85-97,153-172`).
  - `rule_for` and `review_write` use the agent's `write.abs_path` (`agent_review.rs:536-542,858`).

**Scenario (malicious repository plus an agent with file tools only).**
1. The repository commits relative symlinks: `docs/hooks -> ../.git/hooks`,
   `docs/agent -> ../.claude` and `docs/env -> ../.env`. Git allows a symlink that points into
   `.git`.
2. The agent writes `docs/hooks/pre-commit` with ACP `fs/write_text_file`, or with the built-in
   agent's edit tool, which uses the same check.
3. Under shield the write is allowed, and the file lands in `.git/hooks/`. The next `git commit`,
   including from Fyx's git panel, runs it.
4. In the same way:
   - `docs/agent/settings.json` rewrites the agent's own config;
   - `docs/env` reads the private `.env`;
   - with review rules `deny = ["secrets/**"]`, a write to `docs/s/key.txt` lands in `secrets/`
     unreviewed.

Case does the same on macOS and Windows: `rule_for("/p/.ENV")` is `None` under `deny = ["*.env"]`.

**Confirmed.** Appendix A1 and A2, output in B1 and B2:
- `shield_write_through_a_symlink_reaches_protected_dirs`;
- `shield_read_through_a_symlink_reaches_private_files`;
- `security2_deny_rules_match_the_agent_path`.

`docs/fyx/security-modes.md:164` lists "Paths are checked lexically" as a known limit, but only for
a symlink the agent makes itself. Two things argue for fixing it now:
- the variant that comes with the repository needs no terminal at all;
- the review rules make no such disclaimer.

**Fix.**
- **One resolver for both gates.** `fyx_guard` and `agent_review` each resolve the target before
  matching:
  1. canonicalize the deepest existing ancestor and append the rest;
  2. re-match the canonical worktree-relative path as well as the typed one;
  3. deny if either matches;
  4. deny under shield if the canonical path leaves the worktree.
- **Reuse upstream's helper.** Upstream already does this for the built-in agent's sensitive
  paths: `canonicalize_with_ancestors` in `crates/agent/src/tools/tool_permissions.rs:331-351`.
  Copy those ~20 lines into `fyx_guard`, since `agent` depends on `fyx_guard`.
- **Make the hook async.** The write hook runs synchronously inside `project.update`. Add a
  `check_config_write_async(&project, &path, cx).await` before that closure. It stays one hook line.
- **Match case like the volume does.** On case-insensitive volumes (`Fs::is_case_sensitive`), build
  the review globs with `GlobBuilder::case_insensitive(true)`.
- **Windows aliases.** See T4.

### S2. High (integration-4 #1, still open): a rewritten `.fyx/review.toml` applies at the next thread start

**Where.**
- `on_thread_event` calls `reload_rules(.., RulesLoad::ThreadStarted)` for every new thread
  (`agent_review.rs:145`).
- `ThreadStarted` calls `set_rules` and clears the pending change (`:1399-1402`, `:545-551`).
- Trust is `None => true` without a `TrustedWorktrees` global (`:1367-1372`).

**Scenario.**
1. Review mode is on.
2. The agent runs `printf 'auto_accept = ["**"]\n' > .fyx/review.toml` in its terminal. A
   "rules changed, apply?" toast appears.
3. The agent then starts a subagent thread, or the user opens a new thread.
4. The file applies without the click, and the old `deny` rules are gone. Every later edit lands
   unreviewed.

Under shield the ACP and built-in write paths cannot write `.fyx/`, but the terminal can.

**Confirmed.** `security2_rewritten_rules_apply_at_next_thread_start` (A2, B2). After the second
thread starts, `.env` and `src/main.rs` both resolve to `AutoAccept`, nothing is pending, and a
write passes with no review.

**Fix.** Do what integration-4 recommended:
- `auto_accept` only from user settings;
- trust fails closed;
- a thread start re-applies the last *confirmed* rules, not the file on disk;
- a read error keeps the previous rules.

The decision is still the user's (integration-4, "Decisions" 1).

### S3. Medium (High for registry CI): `fyx-ext package` follows an `assets` symlink

**Where.** `fyx_ext/src/package.rs:33-50` (`fyx/main`, unchanged in `2b9b01c`):
- `assets.is_dir()` follows a link, and so does `WalkDir::new(&assets)` at the root;
- `strip_prefix(dir)` still succeeds on the link path.

**Scenario.**
1. An extension repository contains `assets -> ~/.config`, the CI workspace or a mounted secrets
   directory.
2. `fyx-ext package` puts those files into the `.fyxext` as `assets/...`.
3. Platform decision 15 has the registry CI build authors' repositories, so the result is a public
   package that the registry has signed.

**Confirmed.** `assets_symlink_packs_files_from_outside_the_extension` (A4, B4): the package lists
`assets/credentials`.

**Fix.**
- Refuse `assets`, `extension.toml` or the component when it is a symlink (`symlink_metadata`).
- Use `WalkDir::follow_root_links(false)`.
- Require every canonical file to be under `canonicalize(dir)`.
- Stream with a running byte cap.

### S4. Low: consent records are unauthenticated JSON

**Where.** `consent.rs:76-79,148-159`. `<data dir>/fyx-ui-consent.json` maps an id to
(capabilities, SHA-256 or folder). Nothing ties a record to Fyx having written it.

**Scenario.** One command run as the user, for example by an agent's terminal, which shield leaves
unconstrained in step 1:
1. it drops an extension folder into `extensions/installed/`;
2. it writes a matching record.

The extension loads at the next scan with no prompt.

**Confirmed.** `security2_hand_written_consent_record_loads_without_a_prompt` (A3, B3).

This is not a boundary against local processes, which can also drop the folder itself. Consent
stops a user who was tricked into copying a folder, and it stops Zed's "Install Dev Extension".

**Fix.**
- Say this limit in `consent.rs` and in `extension-authoring.md`.
- Best effort until the permission broker exists:
  - sign each record with HMAC-SHA256 under a random key kept in the OS credential store
    (`credentials_provider`), which an unsigned process cannot read silently on macOS;
  - show "N UI extensions were enabled outside Fyx" when records appear that this install did not
    write.

### S5. Medium: a replayed block list stays fresh for ever (freeze)

**Where.** `fyx_registry` `blocklist.rs:27-33` (no expiry), `:383-390` (a replay is a `Refresh`
that sets `fetched_at = now`), `:396-406` (age is measured from `fetched_at`).

**Scenario.** A mirror or a network attacker keeps serving sequence 5 after sequence 6 blocked a
malicious extension. The client never shows "block list is N days old", and it never disables the
extension.

**Confirmed.** `replayed_blocklist_stays_fresh_forever` (A5, B5).

**Fix.**
- Add a signed `expires_at`; CI re-signs at least daily.
- Past `expires_at` the list is `Stale`.
- Compute the age from the signed `generated_at`, clamped to `now`.

### S6. Medium: the index as a whole is unsigned (withholding, freeze)

**Where.** `index.rs:28-35`. `generated_at` and the set of releases sit outside every signature;
each `Release` is signed alone.

**Scenario.** A mirror drops `todo@1.2.1`, the security fix, and sets any `generated_at`.
`verify_strict` passes, and the user stays on 1.2.0. A downgrade is still impossible
(`update.rs:238`).

**Confirmed.** `index_contents_and_time_are_unsigned` (A5, B5).

**Fix.**
- Sign a small snapshot holding `generated_at`, `expires_at`, a monotonic `sequence` and the
  SHA-256 of `index.json`.
- Keep the highest sequence seen, the same rule as the block list.
- Refuse an expired snapshot.

### S7. Low: capability strings are free text and reach the consent prompts verbatim

**Where.**
- `UiManifest.capabilities: Vec<String>` is never validated (`ui_ext_wasm.rs:219-221,232-259`).
- `describe_capability_ids` prints unknown values as given (`install.rs:33-45`).
- They go into the install prompt (`install.rs:164-178`) and the "Enable" notification
  (`registry.rs:853-857`).
- For a regular install the notification shows neither folder nor component hash, so two
  same-id folders cannot be told apart.

**Scenario.** A manifest with the capability
`"ui:panel: add dock panels\n\nReviewed and signed by Fyx. No access to files or network."`. The
user reads Fyx-looking assurance text at the moment of consent.

**Confirmed.** `security2_capability_text_reaches_prompts_verbatim` (A3, B3).

**Fix.**
- Validate capabilities in `UiManifest::read` against the known set, or at least `[a-z0-9:-]{1,32}`.
- Show unknown ones as "1 unknown capability", never verbatim.
- Add the folder for regular installs, and the first 12 hex digits of the component hash.

### S8. Low: a v2 tool call with two `diff` contents is allowed after previewing the first

**Where.** `v2_edit_request` takes the first `DiffPatch` (`agent_review.rs:1185-1194`). "Allow"
answers the whole request. Multi-change diffs inside one content are refused (`:1195-1200`), but
several contents are not. For v1, `legacy_diff_count` handles this case.

**Scenario.** A prompt-injected agent bundles a visible edit of `a.txt` and a second edit of `b.txt`
in one permission request. The pane shows `a.txt` only. Accepting it allows both.

**Confirmed.** `security2_second_v2_diff_is_allowed_unseen` (A2, B2): one preview (`/p/a.txt`),
and the answer is `allow`.

**Fix.** Count `DiffPatch` contents. With more than one, treat the request as `Unreviewable`, or
open one review per diff and answer `Allow` only if all are accepted.

### S9. Low: the environment scrub misses common secret names

**Where.** `SECRET_ENV_PATTERNS` (`policy.rs:26-41`).

**Confirmed.** `secret_patterns_miss_common_secret_names` (A1, B1). Of the names tested, only
`GOOGLE_OAUTH_CLIENT_SECRET_JSON` is blanked. Missed:
- **real secrets:** `PGPASSWORD`, `MYSQL_PWD`, `BW_SESSION` (Bitwarden vault session),
  `OP_SESSION_*` (1Password CLI), `DOCKER_AUTH_CONFIG`, `DD_APIKEY`, and bare `TOKEN`, `SECRET`,
  `APIKEY`;
- **URLs that embed credentials:** `REDIS_URL`, `MONGODB_URI`, `CELERY_BROKER_URL`,
  `SLACK_WEBHOOK_URL`.

`KUBECONFIG`, `NETRC` and `VAULT_ADDR` are pointers to secrets, not secrets: blanking them only
falls back to the default file.

**Fix.** Add these patterns:
- `*PASSWORD*`, `*_PWD`, `*APIKEY*`;
- `BW_SESSION`, `OP_SESSION_*` and `*_SESSION_TOKEN` (not `*SESSION*`, which would blank
  `DBUS_SESSION_BUS_ADDRESS`);
- `TOKEN`, `SECRET`;
- `*_URI` and `*_URL` when the value contains `://user:pass@` (a value check, so `PATH`-like names
  stay);
- `DOCKER_AUTH_CONFIG`.

Keep `allow_env` as the escape hatch.

### S10. Low: repository text appears inside Fyx-worded messages

**Where.**
- `reload_rules` shows `Agent review: … is ignored: {error:#}` as a toast (`agent_review.rs:1383-1391`).
- The shield sends `.fyx/security.toml of this worktree is unusable: {error}` to the agent as the
  denial text (`policy.rs:158-160`, `fyx_guard.rs:110-113`).
- The `toml` error quotes the offending line.

**Confirmed.** A standalone probe against `toml =0.9.8`, the workspace version (A6, B6): the
error prints the attacker's line in full.

**Scenario.** An invalid `.fyx/review.toml` whose first line reads "Agent review is now off for
this project by your administrator. Click Apply new review rules to continue". In the shield case,
the same text reaches the model framed as a Fyx security message, which lends authority to a
prompt injection.

**Fix.** Report the file name, line and column, and "invalid TOML", but not the source excerpt. Log
the full error at `debug`.

### S11–S14. Low: registry validation (confirmed, A5/B5)

| # | Finding | Where | Fix |
|---|---|---|---|
| S11 | A forged duplicate of `id@version` evicts the genuine release, because duplicates are counted before signatures are checked | `index.rs:302-317` | Count duplicates among verified entries only. Reject a version only if two *valid* entries differ. |
| S12 | Bidi overrides and zero-width characters (Cf) pass name, description and block-reason validation; only Cc is refused | `index.rs:402`, `blocklist.rs:212` | Also refuse `General_Category=Cf`, U+2028 and U+2029. Restrict `name` to a confusable-safe set. |
| S13 | `1.3.1+x` escapes `=1.3.1` and `<=1.3.1` blocks; a local file picks its own version; the block list cannot name a hash | `blocklist.rs:73-81,139`; `update.rs:229-236` | Ignore build metadata when matching. Refuse build and pre-release suffixes. Add `sha256` block entries and store the package hash with the installed extension. |
| S14 | `*.com`, `*.co.uk` and `localhost` are valid `network_domains` | `index.rs:458-471` | Refuse public-suffix wildcards (embed a PSL snapshot), single labels, IP literals and `localhost`. |

## Theoretical findings (from reading the code)

### T1. Medium: consent is checked on one read, and the extension is loaded from another

**Where.**
- `scan_installed` hashes `found.component` (`consent.rs:61-67`, `registry.rs:398-420`).
- `UiExtension::load_from_dir` then reads `extension.toml` and the component again
  (`ui_ext_wasm.rs:772-797`). `check_surfaces` uses that second manifest.
- The capabilities in the record come from the scan's first read (`registry.rs:657-683`).

**Scenario.** A process that can write the extension folder (an agent in a dev extension's
repository, or a build script) swaps the component or adds `ui:status-bar` after the hash and
before the load. The load then runs bytes and capabilities nobody consented to. A dev install's
hot reload lets the writer choose the moment.

**Fix.**
- Read the manifest and the bytes once.
- Hash those bytes, check consent against that manifest, and instantiate the same bytes: add
  `UiExtension::load_checked(id, manifest, bytes)`, and have `load_from_dir` call it.

### T2. Medium: the shield's read-only list misses other agents' configs and Fyx's task sources

**Where.** `PROTECTED_DIRS` and `PROTECTED_FILES` (`policy.rs:20-22`) cover `.claude`, `.codex`,
`.zed`, `.fyx`, `.git` and `.mcp.json`. Not covered:
- **`.gemini/`:** Gemini CLI is a first-class ACP agent, and its `settings.json` declares
  `mcpServers` commands.
- **`.vscode/tasks.json` and `launch.json`:** Fyx loads them as task sources when there is no
  `.zed/` (`project/src/task_inventory.rs:101,537`). That is the reason `.zed/` is protected.
- **`.agents/`:** project skills. Upstream already treats them as sensitive
  (`tool_permissions.rs:327`).
- **`.cursor/`.**
- **The rules files Fyx gives every agent:** `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.rules`, …
  (`prompt_store/src/prompts.rs:22-32`). Rewriting them makes a prompt injection persist into
  every later session.

**Fix.**
- Add `.gemini`, `.vscode`, `.agents` and `.cursor` to `PROTECTED_DIRS`.
- For the rules files, ask the user (see "Decisions"). Recommended: read-only under shield, and
  `always_ask` in review by default.

### T3. Medium: a lowering the user declined applies silently at the next start

**Where.**
- At startup the requested settings apply with only an `info` log (`fyx_guard.rs:470-473`).
- A declined lowering is kept "for the next start" (`:548-550,604`).

**Scenario.**
1. An agent's terminal writes `"fyx_security": {"mode": "off"}`.
2. The user clicks "Keep Current Mode".
3. The next launch runs with the shield off, and nothing says so.

**Fix.**
- Persist the last mode the user confirmed in Fyx's data directory.
- At startup, if the settings file asks for less, keep the stricter mode and show the same prompt.
- Show the mode in the status bar while it is lower than the last confirmed one.

The data file is writable by the same terminal, so this raises the bar rather than closing the gap
(step 2 closes it). It does stop the one-command downgrade.

### T4. Low: Windows path aliases defeat lexical matching

`normalize_path` resolves only `.` and `..`. Under Win32 path rules the following each reach a
protected file:
- `.claude.` or `.claude ` (trailing dots and spaces are stripped);
- `.mcp.json::$DATA` (the default data stream);
- 8.3 short names such as `CLAUDE~1`;
- `\\?\` and `\\.\` prefixes.

`fyx_guard` (`policy.rs:44-89`) and the review rules are both affected.

**Fix.**
- Part of the S1 resolver: canonicalize with `GetFinalPathNameByHandleW` (`std::fs::canonicalize`
  does) before matching.
- Refuse components that end in `.` or a space, or that contain `:`.

### T5. Low: guest stderr is inherited raw and unbounded

**Where.** `wasi.inherit_stderr()` (`ui_ext_wasm.rs:696`). Writes go straight to Fyx's own stderr:
- they are not counted toward `MAX_LIFTED_TEXT_PER_CALL`;
- they are not rate-limited;
- escape sequences are not filtered.

**Effects.**
- When Fyx runs from a terminal (`fyx --foreground`, dev builds), a guest can emit escape
  sequences: OSC 52 clipboard writes, title changes, or fake output.
- When stderr is a file or the journal, a guest can fill it.

**Fix.** Give WASI a custom `StdoutStream` that:
- line-buffers;
- caps bytes per call and per minute;
- escapes control characters;
- writes through `log` with the extension id.

### T6. Low: log-line injection

**Where.**
- The `log` host call writes the guest's text, newlines included, into Fyx's log
  (`ui_ext_wasm.rs:462-481`).
- `fyx_guard` logs agent-supplied paths in `warn!` lines (`fyx_guard.rs:104,109`).
- `agent_review` does the same in `record` (`agent_review.rs:519-527`).

**Effect.** A guest or an agent can forge lines such as `fyx_guard: security mode "shield"` or
`fyx: agent review: … Accepted`. Logs are what a user checks after an incident.

**Fix.** One `sanitize_for_log` helper that escapes C0, C1 and bidi controls (as `escape_debug`
does). Use it for every guest- or agent-supplied string.

### T7. Low: the review pane names files by file name only

**Where.** Turn history, file buttons and the tab title use `file_name` (`review_pane.rs:700,716,979`).
`.claude/settings.json` and `src/settings.json` therefore look the same. An agent-chosen name may
also carry bidi or control characters, which GPUI's platform shaping reorders.

**Fix.** Show the worktree-relative path, highlight protected or rule-matched paths, and escape Cf
and control characters in displayed paths.

### T8. Low: data-URL images in extension markdown are decoded in-process

**Where.** `markdown.rs:1361-1381` decodes `data:` images. Remote images are not fetched, because
`view.rs` sets no `image_resolver`; that part was checked and is correct. A guest can still send a
tiny PNG, GIF or WebP that decodes to gigabytes, or an SVG for `resvg` to parse.

**Fix.** Disable images in extension markdown (no images in the 0.1 contract), or cap decoded
dimensions before decoding.

### T9. Low: registry and SDK tooling

From the registry and package part of this review:
- **No key roles and no revocation.** Any keyset key signs anything; one compromised key is a full
  compromise (`signing.rs:122-135`). Add roles (release, block list, snapshot), a signed rotation
  statement and a revoked-key list in the snapshot.
- **No size caps** on `index.json`, `blocklist.json`, the cache, or the counts of releases and
  entries. `verify_package` holds the whole package in memory (`index.rs:204`). Add caller-side
  caps, and check `Content-Length` against the signed size.
- **`fyx-registry sign release` signs without the package** when `--package` is omitted
  (`main.rs:64-66,173-175`). Require it.
- **Key handling.**
  - `write_atomically` writes to a predictable `<path>.tmp` that `fs::write` follows if it is a
    symlink (`main.rs:312-318`): use `NamedTempFile::new_in` and `persist`.
  - `FYX_REGISTRY_SIGNING_KEY` stays in the environment for the whole run.
  - Key-file permissions are not checked.
- **`fyx_ext_harness` sets no store limiter** (`fyx_ext_harness.rs:210-232` in `2b9b01c`), and
  `fyx-ext check`/`package` now run guest code. Reuse `ui_ext_wasm`'s limits. Document that CI runs
  `fyx-ext` in a throwaway sandbox with no secrets mounted; that also contains S3.

### T10. Info: requirements for the future install store

No code unpacks a `.fyxext` yet, so zip-slip cannot happen today. The unpacker must:
- allowlist the entries: `extension.toml`, the component and `assets/**`;
- accept only `Normal` path components: no `..`, no absolute paths, no drive letters, no
  backslashes, no `:`, no duplicate or case-folded names;
- refuse symlink entries (check the external attributes);
- cap the uncompressed size and the number of entries;
- extract into a new temporary directory and rename it into place;
- record the package's SHA-256 (see S13).

`index.rs:435-443` accepts `C:/x` as a `source.path`; refuse `:`.

## Checked and correct

- **Install.**
  - Ids are validated before any path is built.
  - The link must sit directly in the installed directory.
  - `cmd`-special characters are refused in both paths.
  - Every special character `cmd` treats as a separator is covered; `;`, `,` and `=` only split
    `mklink`'s arguments.
- **Consent.**
  - A missing or broken consent file means "ask again".
  - The file is saved with `atomic_write`, which replaces a symlink rather than writing through it.
  - "Enable" re-hashes on the following scan, so a component changed between the notification and
    the click waits again.
  - A dev grant is tied to the canonical folder, so a repointed link waits again.
- **Guest sandbox.**
  - No preopens, environment or arguments.
  - Network is denied: `deny_network`, and wasmtime-wasi 48.0.7's socket defaults deny too.
  - The epoch budget plus a wall-clock deadline.
  - The tree depth cap is 64.
  - Markdown links must be `http(s)://` and need a confirmation that shows the full URL
    (`view.rs:1172-1199`).
  - No remote images.
- **Deserialization.**
  - `toml 0.9.8` rejects nesting beyond about 60 inline tables or 80 arrays with an error, not a
    stack overflow (A6, B6, depth 100 000).
  - `serde_json` keeps its 128-level limit for the consent file and the registry.
  - `deny_unknown_fields` is set on `RulesFile`, `SecurityFile` and every registry type.
  - Registry duplicate JSON keys fail the typed parse.
- **Registry signatures.**
  - The canonical encoding is length-prefixed and domain-tagged, and it is recomputed from the
    parsed struct, so JSON spelling cannot change what was signed.
  - ring Ed25519, with the key looked up by a validated id and duplicate ids refused.
  - Size and SHA-256 of a package are checked against the signed release.
  - Block-list sequences are monotonic.
  - There is no downgrade in `plan_updates`.
- **Shield.**
  - Off is a no-op.
  - An unknown mode becomes shield.
  - A repository file can only tighten, and while it is loading or unusable the worktree is
    restricted.
  - Lowering at run time asks first (F4).
  - The path tools check the whole moved or copied tree (F2).
- **Review.**
  - A held write is not applied if the file changed, moved or was deleted during review (M4).
  - `deny` and `always_ask` apply with review mode off.
  - `always_ask` beats "accept all in turn".

## Decisions for the user

1. **Project rules (S2, repeated from integration-4).** May `.fyx/review.toml` ever auto-accept?
   Recommended: no. Only user settings loosen review.
2. **Rules files (T2).** Should agents be able to edit `AGENTS.md`, `CLAUDE.md` and the like under
   shield? Recommended: no. In review mode, `always_ask` them by default.
3. **Consent records (S4).** Sign them with a keychain-held key now, or document the limit and wait
   for the permission broker (plan 1.5)? Recommended: document now, and sign them with the broker.

## Fix order

1. **W9 and W5 together:** the S1 resolver in `fyx_guard`, used by `agent_review`'s rule matching.
2. **W5:** S2 and S8. **W9:** T2 and T3.
3. **Registry:** S5 and S6, in the format before any client ships, plus S11–S14.
4. **W4:** S3 and the harness limits (T9).
5. **W3:** T1, S7, S4 docs, T5 and T6, and the rest of integration-4 #7.

## Appendix A: test code (scratch branch `scratch/security-2`, never pushed)

Every test was run with `cargo test` on the scratch merge described under "Scope".

### A1. `crates/fyx_guard/tests/security2.rs` (new file; S1, S9)

```rust
//! security-2 review, scratch only: the shield's path checks are lexical, so a symlink inside the
//! worktree reaches a protected or private file; and secret names the patterns miss.

use std::{path::Path, rc::Rc};

use acp_thread::{AcpThread, AgentConnection as _, StubAgentConnection};
use gpui::{Entity, TestAppContext, UpdateGlobal as _};
use project::{FakeFs, Project};
use serde_json::json;
use settings::{FyxSecurityMode, SettingsStore};
use std::sync::Arc;
use util::{path, path_list::PathList};

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut settings_store = SettingsStore::test(cx);
        settings_store.register_setting::<feature_flags::FeatureFlagsSettings>();
        cx.set_global(settings_store);
        fyx_guard::init(cx);
    });
}

fn set_shield(cx: &mut TestAppContext) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.fyx_security.get_or_insert_default().mode = Some(FyxSecurityMode::Shield);
            });
        });
    });
    cx.run_until_parked();
}

/// A repository as it could be cloned: `docs/hooks` and `docs/env` are committed symlinks into
/// `.git/hooks` and onto `.env`.
async fn thread(cx: &mut TestAppContext) -> (Arc<FakeFs>, Entity<AcpThread>) {
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/repo"),
        json!({
            ".env": "API_KEY=1",
            ".git": { "hooks": {}, "config": "" },
            ".claude": {},
            "docs": { "readme.md": "" },
        }),
    )
    .await;
    fs.insert_symlink(path!("/repo/docs/hooks"), path!("/repo/.git/hooks").into())
        .await;
    fs.insert_symlink(path!("/repo/docs/env"), path!("/repo/.env").into())
        .await;
    fs.insert_symlink(path!("/repo/docs/agent"), path!("/repo/.claude").into())
        .await;
    let project = Project::test(fs.clone(), [path!("/repo").as_ref()], cx).await;
    let connection = Rc::new(StubAgentConnection::new());
    let thread = cx
        .update(|cx| {
            connection.new_session(project, PathList::new(&[Path::new(path!("/repo"))]), cx)
        })
        .await
        .unwrap();
    (fs, thread)
}

async fn write(
    thread: &Entity<AcpThread>,
    path: &str,
    content: &str,
    cx: &mut TestAppContext,
) -> Result<(), String> {
    thread
        .update(cx, |thread, cx| {
            thread.write_text_file(path.into(), content.into(), cx)
        })
        .await
        .map_err(|error| format!("{error:#}"))
}

async fn read(
    thread: &Entity<AcpThread>,
    path: &str,
    cx: &mut TestAppContext,
) -> Result<String, String> {
    thread
        .update(cx, |thread, cx| {
            thread.read_text_file(path.into(), None, None, false, cx)
        })
        .await
        .map_err(|error| error.message)
}

#[gpui::test]
async fn shield_write_through_a_symlink_reaches_protected_dirs(cx: &mut TestAppContext) {
    init_test(cx);
    let (fs, thread) = thread(cx).await;
    set_shield(cx);

    // The direct paths are refused...
    let error = write(&thread, path!("/repo/.git/hooks/pre-commit"), "x", cx)
        .await
        .unwrap_err();
    assert!(error.contains(".git/"), "{error}");
    write(&thread, path!("/repo/.claude/settings.json"), "{}", cx)
        .await
        .unwrap_err();

    // ...the same files through committed symlinks are not.
    let via_hooks = write(&thread, path!("/repo/docs/hooks/pre-commit"), "#!/bin/sh\n", cx).await;
    let via_agent = write(&thread, path!("/repo/docs/agent/settings.json"), "{}", cx).await;
    eprintln!("SECURITY2 write via docs/hooks: {via_hooks:?}");
    eprintln!("SECURITY2 write via docs/agent: {via_agent:?}");
    let hook = fs.read_file_sync(path!("/repo/.git/hooks/pre-commit"));
    let agent_settings = fs.read_file_sync(path!("/repo/.claude/settings.json"));
    eprintln!(
        "SECURITY2 .git/hooks/pre-commit = {:?}",
        hook.as_ref().map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    );
    eprintln!(
        "SECURITY2 .claude/settings.json = {:?}",
        agent_settings
            .as_ref()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    );
    assert!(via_hooks.is_ok() && hook.is_ok(), "the hook was written");
    assert!(
        via_agent.is_ok() && agent_settings.is_ok(),
        "the agent config was written"
    );
}

#[gpui::test]
async fn shield_read_through_a_symlink_reaches_private_files(cx: &mut TestAppContext) {
    init_test(cx);
    let (_fs, thread) = thread(cx).await;
    set_shield(cx);

    let error = read(&thread, path!("/repo/.env"), cx).await.unwrap_err();
    assert!(error.contains("private_files"), "{error}");
    let via_link = read(&thread, path!("/repo/docs/env"), cx).await;
    eprintln!("SECURITY2 read via docs/env: {via_link:?}");
    assert_eq!(via_link.as_deref(), Ok("API_KEY=1"));
}

#[test]
fn secret_patterns_miss_common_secret_names() {
    let names = [
        "PGPASSWORD",
        "MYSQL_PWD",
        "BW_SESSION",
        "OP_SESSION_my_account",
        "REDIS_URL",
        "MONGODB_URI",
        "CELERY_BROKER_URL",
        "HF_HUB_TOKEN_PATH",
        "NETRC",
        "DOCKER_AUTH_CONFIG",
        "KUBECONFIG",
        "VAULT_ADDR",
        "TOKEN",
        "SECRET",
        "APIKEY",
        "DD_APIKEY",
        "SLACK_WEBHOOK_URL",
        "GOOGLE_OAUTH_CLIENT_SECRET_JSON",
    ]
    .map(String::from);
    let blanked = fyx_guard::secret_env_names(&names, &[]);
    let missed = names
        .iter()
        .filter(|name| !blanked.contains(name))
        .collect::<Vec<_>>();
    eprintln!("SECURITY2 blanked: {blanked:?}");
    eprintln!("SECURITY2 not blanked: {missed:?}");
    assert!(missed.contains(&&"PGPASSWORD".to_string()));
    assert!(missed.contains(&&"BW_SESSION".to_string()));
}
```

### A2. Appended to `mod tests` in `crates/agent_review/src/agent_review.rs` (S1, S2, S8)

These use the module's existing helpers `agent_write`, `gate_write`, `OLD` and `NEW`.

```rust
    // security-2 review, scratch only.

    async fn security2_project(
        tree: serde_json::Value,
        cx: &mut TestAppContext,
    ) -> (
        Entity<AgentReview>,
        std::sync::Arc<project::FakeFs>,
        Entity<Project>,
        Rc<acp_thread::StubAgentConnection>,
        Entity<acp_thread::AcpThread>,
    ) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(util::path!("/p"), tree).await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let thread = security2_session(&connection, &project, cx).await;
        cx.run_until_parked();
        (review, fs, project, connection, thread)
    }

    async fn security2_session(
        connection: &Rc<acp_thread::StubAgentConnection>,
        project: &Entity<Project>,
        cx: &mut TestAppContext,
    ) -> Entity<acp_thread::AcpThread> {
        cx.update(|cx| {
            acp_thread::AgentConnection::new_session(
                connection.clone(),
                project.clone(),
                workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                cx,
            )
        })
        .await
        .unwrap()
    }

    /// integration-4 #1 still open: a rules file the agent rewrote mid-session waits for the user,
    /// but the next thread start (a new or sub-agent thread) applies it from disk, and with no
    /// `TrustedWorktrees` global the worktree counts as trusted, so `auto_accept` applies.
    #[gpui::test]
    async fn security2_rewritten_rules_apply_at_next_thread_start(cx: &mut TestAppContext) {
        let (review, fs, project, connection, _thread) = security2_project(
            serde_json::json!({
                ".fyx": { "review.toml": "deny = [\"*.env\"]\n" },
                ".env": "A=1\n",
                "src": { "main.rs": "" },
            }),
            cx,
        )
        .await;
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let kind = |name: &str, cx: &mut TestAppContext| {
            let path = PathBuf::from(util::path!("/p")).join(name);
            review.read_with(cx, |review, _| review.rule_for(&path).map(|rule| rule.kind))
        };
        assert_eq!(kind(".env", cx), Some(RuleKind::Deny));

        // What `printf 'auto_accept = ["**"]' > .fyx/review.toml` in the agent's terminal does.
        fs.save(
            Path::new(util::path!("/p/.fyx/review.toml")),
            &"auto_accept = [\"**\"]\n".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        let root = PathBuf::from(util::path!("/p"));
        assert!(review.read_with(cx, |review, _| review.has_pending_rules(&root)));
        assert_eq!(kind(".env", cx), Some(RuleKind::Deny), "held while pending");

        // The agent opens another thread (or the user starts a new one).
        let _second = security2_session(&connection, &project, cx).await;
        cx.run_until_parked();
        eprintln!(
            "SECURITY2 after thread start: .env -> {:?}, src/main.rs -> {:?}, pending -> {}",
            kind(".env", cx),
            kind("src/main.rs", cx),
            review.read_with(cx, |review, _| review.has_pending_rules(&root))
        );
        assert_eq!(kind(".env", cx), Some(RuleKind::AutoAccept));
        let outcome = gate_write(&review, agent_write(util::path!("/p/src/main.rs"), cx), cx)
            .await
            .unwrap();
        assert_eq!(outcome.content.as_deref(), Some(NEW), "landed unreviewed");
        assert!(review.read_with(cx, |review, _| review.reviews().is_empty()));
    }

    /// `deny` and `always_ask` match the path the agent sent, not the file the write reaches.
    #[gpui::test]
    async fn security2_deny_rules_match_the_agent_path(cx: &mut TestAppContext) {
        let (review, fs, _project, _connection, thread) = security2_project(
            serde_json::json!({
                ".fyx": { "review.toml": "deny = [\"secrets/**\", \"*.env\"]\n" },
                ".env": "A=1\n",
                "secrets": { "key.txt": "k\n" },
                "docs": {},
            }),
            cx,
        )
        .await;
        fs.insert_symlink(util::path!("/p/docs/s"), util::path!("/p/secrets").into())
            .await;
        cx.run_until_parked();

        let write = |path: &'static str, cx: &mut TestAppContext| {
            thread.update(cx, |thread, cx| {
                thread.write_text_file(path.into(), "agent\n".into(), cx)
            })
        };
        let direct = write(util::path!("/p/secrets/key.txt"), cx).await;
        assert!(format!("{:#}", direct.unwrap_err()).contains("deny"));
        let via_link = write(util::path!("/p/docs/s/key.txt"), cx).await;
        eprintln!("SECURITY2 deny via symlink: {via_link:?}");
        let on_disk = fs
            .load(Path::new(util::path!("/p/secrets/key.txt")))
            .await
            .unwrap();
        eprintln!("SECURITY2 secrets/key.txt = {on_disk:?}");
        assert!(via_link.is_ok());
        assert_eq!(on_disk, "agent\n");

        // Case: the same file on macOS's and Windows' default file systems.
        let rule = |path: &str, cx: &mut TestAppContext| {
            review.read_with(cx, |review, _| {
                review.rule_for(Path::new(path)).map(|rule| rule.kind)
            })
        };
        assert_eq!(rule(util::path!("/p/.env"), cx), Some(RuleKind::Deny));
        assert_eq!(rule(util::path!("/p/.ENV"), cx), None);
        assert_eq!(rule(util::path!("/p/SECRETS/key.txt"), cx), None);
    }

    /// A v2 tool call with two `diff` contents: only the first is previewed, and "Allow" answers
    /// the whole request, so the second file is allowed without being shown.
    #[gpui::test]
    async fn security2_second_v2_diff_is_allowed_unseen(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            crate::init(cx);
        });
        let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
        review.update(cx, |review, cx| review.set_enabled(true, cx));
        let fs = project::FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/p"),
            serde_json::json!({ "a.txt": OLD, "b.txt": OLD }),
        )
        .await;
        let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
        let connection = Rc::new(acp_thread::StubAgentConnection::new());
        let thread = security2_session(&connection, &project, cx).await;
        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());
        let patch = |name: &str| {
            format!(
                "--- a/{name}\n+++ b/{name}\n@@ -1,5 +1,5 @@\n-one\n+ONE\n two\n three\n four\n-five\n+FIVE\n"
            )
        };
        let request: acp_v2::RequestPermissionRequest = serde_json::from_value(serde_json::json!({
            "sessionId": session_id,
            "title": "Edit a.txt",
            "options": [
                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                {"optionId": "reject", "name": "Reject", "kind": "reject_once"}
            ],
            "subject": {
                "type": "tool_call",
                "toolCall": {
                    "toolCallId": "edit-2",
                    "title": "Edit a.txt",
                    "kind": "edit",
                    "content": [
                        {"type": "diff",
                         "changes": [{"operation": "modify", "path": util::path!("/p/a.txt")}],
                         "patch": {"format": "git_patch", "text": patch("a.txt")}},
                        {"type": "diff",
                         "changes": [{"operation": "modify", "path": util::path!("/p/b.txt")}],
                         "patch": {"format": "git_patch", "text": patch("b.txt")}}
                    ]
                }
            }
        }))
        .unwrap();
        let (_, answer) = thread
            .update(cx, |thread, cx| thread.request_permission(request, cx))
            .unwrap();
        cx.run_until_parked();
        let (id, previewed) = review.read_with(cx, |review, _| {
            let paths = review
                .reviews()
                .iter()
                .map(|review| review.edit().abs_path().to_path_buf())
                .collect::<Vec<_>>();
            (review.reviews()[0].id(), paths)
        });
        eprintln!("SECURITY2 previewed: {previewed:?}");
        review.update(cx, |review, cx| {
            review.decide_all(id, HunkDecision::Accepted, cx).unwrap();
            review.submit(id, cx).unwrap();
        });
        cx.run_until_parked();
        let outcome = answer.await;
        eprintln!("SECURITY2 answer for both files: {outcome:?}");
        assert_eq!(previewed, [PathBuf::from(util::path!("/p/a.txt"))]);
        match outcome {
            acp_v2::RequestPermissionOutcome::Selected(selected) => assert_eq!(
                selected.option_id,
                acp_v2::PermissionOptionId::new("allow")
            ),
            other => panic!("expected allow, got {other:?}"),
        }
    }
```

### A3. Appended to `crates/ui_ext_host/src/wasm_tests.rs` (S4, S7)

These use the file's existing helpers `status_item_wasm`, `status_item_manifest` and `run_until`.

```rust
/// security-2 review, scratch only: a consent record is plain, unauthenticated JSON, so any
/// process running as the user (an agent's terminal) can pre-approve an extension it dropped in.
#[gpui::test]
async fn security2_hand_written_consent_record_loads_without_a_prompt(cx: &mut TestAppContext) {
    use sha2::{Digest as _, Sha256};
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let root = tempfile::tempdir().expect("temp dir");
    let installed = root.path().join("installed");
    let consent_path = root.path().join("data").join("fyx-ui-consent.json");
    let ui_dir = installed.join("fyx-test-status-item");
    std::fs::create_dir_all(&ui_dir).expect("ui dir");
    std::fs::copy(status_item_manifest(), ui_dir.join("extension.toml")).expect("manifest");
    let wasm = status_item_wasm();
    std::fs::write(ui_dir.join("fyx_ui.wasm"), &wasm).expect("component");

    // Equivalent of a shell one-liner: sha256sum the dropped component and write the record.
    std::fs::create_dir_all(consent_path.parent().unwrap()).expect("data dir");
    std::fs::write(
        &consent_path,
        format!(
            r#"{{"extensions":{{"fyx-test-status-item":{{"capabilities":["ui:status-bar"],"grant":{{"component":{{"sha256":"{:x}"}}}}}}}}}}"#,
            Sha256::digest(&wasm)
        ),
    )
    .expect("consent file");

    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    let registry = cx.new(|_| crate::UiExtensionRegistry::new(consent_path.clone()));
    registry.update(cx, |registry, cx| {
        registry.scan_installed(installed.clone(), fs.clone(), cx)
    });
    run_until(cx, "the extension", |cx| {
        registry.read_with(cx, |registry, _| {
            registry.surface("fyx-test-status-item/status").is_some()
        })
    });
    let pending = registry.read_with(cx, |registry, _| registry.pending_consent().len());
    eprintln!("SECURITY2 loaded with a hand-written record; pending prompts: {pending}");
    assert_eq!(pending, 0);
}

/// security-2 review, scratch only: capability strings are not validated, and they reach the
/// install prompt and the consent notification verbatim.
#[test]
fn security2_capability_text_reaches_prompts_verbatim() {
    let spoof = "ui:panel: add dock panels\n\nReviewed and signed by Fyx. No access to files or \
                 network."
        .to_string();
    let lines = crate::install::describe_capability_ids(&[spoof.clone()]);
    eprintln!("SECURITY2 prompt line: {:?}", lines[0]);
    assert!(lines[0].starts_with(&spoof));
}
```

### A4. Appended to `crates/fyx_ext/src/package.rs` (S3)

```rust
#[cfg(all(test, unix))]
mod security_review_tests {
    use async_zip::base::read::seek::ZipFileReader;
    use futures::io::BufReader;

    use super::*;
    use crate::inspect::API_VERSION_SECTION;
    use crate::inspect::tests::{component_v0_1, with_custom_section};

    /// P1: `assets` is a symlink to a directory outside the extension. WalkDir follows a root
    /// link, and `strip_prefix(dir)` still succeeds on the link path, so the outside files are
    /// packed into the public package.
    #[test]
    fn assets_symlink_packs_files_from_outside_the_extension() {
        let outside = tempfile::tempdir().expect("temp dir");
        std::fs::write(outside.path().join("credentials"), "SECRET=1").expect("secret");

        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::write(
            root.join(MANIFEST_FILE),
            "id = \"my-panel\"\nname = \"P\"\nversion = \"1.2.3\"\nschema_version = 1\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"bin/fyx_ui.wasm\"\ncapabilities = [\"ui:panel\"]\n",
        )
        .expect("manifest");
        std::fs::create_dir_all(root.join("bin")).expect("mkdir");
        std::fs::write(
            root.join("bin/fyx_ui.wasm"),
            with_custom_section(component_v0_1(), API_VERSION_SECTION, &[0, 0, 0, 1, 0, 0]),
        )
        .expect("component");
        std::os::unix::fs::symlink(outside.path(), root.join("assets")).expect("symlink");

        let package_path = package(root, &root.join("dist")).expect("package");
        let names: Vec<String> = smol::block_on(async {
            let file = smol::fs::File::open(&package_path).await.expect("open");
            let reader = ZipFileReader::new(BufReader::new(file)).await.expect("zip");
            reader
                .file()
                .entries()
                .iter()
                .map(|entry| entry.filename().as_str().expect("utf-8").to_string())
                .collect()
        });
        println!("P1: package entries = {names:?}");
        assert!(names.contains(&"assets/credentials".to_string()));
    }
}
```

### A5. `crates/fyx_registry/src/security_review_tests.rs` (new file, plus `#[cfg(test)] mod security_review_tests;` in `fyx_registry.rs`; S5, S6, S11–S14)

```rust
//! Security review (security-2) probes. Scratch branch only; never pushed.

use semver::Version;

use crate::{
    BlocklistCache, CacheUpdate, Freshness, Index, InstalledExtension, InstalledVersion, Origin,
    Permissions, Signed, Timestamp, VerifiedBlocklist,
    blocklist::test_support::{blocklist, entry, signed_json},
    index::test_support::release,
    plan_updates,
    signing::test_keys::{keyset, registry_key},
    update::Action,
};

const DAY: u64 = 24 * 60 * 60;

/// R1: a replayed copy of the newest list the client has seen keeps the cache "fresh" for ever.
/// Freshness is computed from the local `fetched_at`, not from the signed `generated_at`, and the
/// list carries no expiry, so a mirror or network attacker that withholds sequence 6 (which
/// blocks malware) and keeps serving sequence 5 is never noticed.
#[test]
fn replayed_blocklist_stays_fresh_forever() {
    let list = blocklist(5, vec![]); // generated_at = 50 (1970)
    let body = signed_json(list, &registry_key());
    let mut cache: Option<BlocklistCache> = None;
    for day in 0..365u64 {
        let now = Timestamp(1_000_000 + day * DAY);
        match BlocklistCache::apply_fetch(cache.as_ref(), Ok(&body), &keyset(), now) {
            CacheUpdate::Replace(fresh) | CacheUpdate::Refresh(fresh) => cache = Some(fresh),
            CacheUpdate::Keep(reason) => panic!("unexpected keep: {reason:?}"),
        }
        assert_eq!(BlocklistCache::freshness(cache.as_ref(), now), Freshness::Fresh);
    }
    let cached = cache.expect("cached");
    assert_eq!(cached.blocklist.signed().signed.generated_at, Timestamp(50));
    println!(
        "R1: after 365 days of replays the cache is Fresh; signed generated_at = {:?}",
        cached.blocklist.signed().signed.generated_at
    );
}

/// R2: one forged entry (bad signature) that repeats the (id, version) of a genuine release
/// removes the genuine release too, because duplicates are counted before signatures are checked.
#[test]
fn forged_duplicate_evicts_a_genuine_release() {
    let genuine = Signed::sign(release("todo", "1.2.0"), &registry_key());
    let mut forged = genuine.clone();
    forged.signed.description = "tampered".into();
    let mut index = Index::new(Timestamp(0));
    index.releases = vec![genuine, forged];
    let verified = index.verify(&keyset());
    let id = crate::ExtensionId::new("todo").expect("id");
    assert!(verified.latest(&id).is_none());
    println!("R2: rejected = {:?}", verified.rejected());
}

/// R3: names, descriptions and block reasons refuse control characters (Cc) but accept bidi
/// overrides and zero-width characters (Cf), which the extensions page and notifications render.
#[test]
fn bidi_and_zero_width_text_passes_validation() {
    let mut spoof = release("todo", "1.2.0");
    spoof.name = "Todo \u{202E}lanoitcif\u{202C}".into();
    spoof.description = "Official\u{200B} Fyx extension \u{2066}x\u{2069}".into();
    assert!(spoof.validate().is_ok());

    let mut list = blocklist(1, vec![entry("todo", "*")]);
    list.entries[0].reason = "\u{202E}gnihton\u{202C} (safe to ignore)".into();
    assert!(list.validate().is_ok());
    println!("R3: release.validate and blocklist.validate accept U+202E/U+200B/U+2066");
}

/// R4: a block entry is matched by semver equality/order, which includes build metadata. A
/// locally installed package (Origin::LocalFile, version from its own extension.toml) that calls
/// itself `1.3.1+x` is not caught by `=1.3.1` or `<=1.3.1`, and the block list cannot name a
/// package by hash at all.
#[test]
fn build_metadata_escapes_exact_and_upper_bound_blocks() {
    let bytes = signed_json(
        blocklist(1, vec![entry("todo", "=1.3.1"), entry("todo", "<=1.3.1")]),
        &registry_key(),
    );
    let blocklist = VerifiedBlocklist::from_json(&bytes, &keyset()).expect("verifies");
    let id = crate::ExtensionId::new("todo").expect("id");
    let sneaky = Version::parse("1.3.1+x").expect("version");
    assert!(blocklist.is_blocked(&id, &Version::parse("1.3.1").expect("version")));
    assert!(!blocklist.is_blocked(&id, &sneaky));

    let installed = InstalledExtension {
        id: id.clone(),
        active: InstalledVersion {
            version: sneaky,
            origin: Origin::LocalFile,
        },
        previous: None,
        pinned: false,
        approved: Permissions::default(),
    };
    let plan = plan_updates(
        &[installed],
        &crate::VerifiedIndex::default(),
        Some(&blocklist),
        Timestamp(0),
    );
    assert!(
        !plan
            .iter()
            .any(|planned| matches!(planned.action, Action::Disable { .. }))
    );
    println!("R4: plan for 1.3.1+x under =1.3.1 and <=1.3.1 blocks: {plan:?}");
}

/// R5: `generated_at` and the set of releases are not signed. Removing the newest (fixed)
/// release and changing `generated_at` still verifies, so an index freeze is undetectable.
#[test]
fn index_contents_and_time_are_unsigned() {
    let old = Signed::sign(release("todo", "1.2.0"), &registry_key());
    let fix = Signed::sign(release("todo", "1.2.1"), &registry_key());
    let mut full = Index::new(Timestamp(100));
    full.releases = vec![old.clone(), fix];
    full.verify_strict(&keyset()).expect("full index verifies");

    let mut frozen = Index::new(Timestamp(9_999_999_999));
    frozen.releases = vec![old];
    let verified = frozen.verify_strict(&keyset()).expect("frozen index verifies");
    let id = crate::ExtensionId::new("todo").expect("id");
    assert_eq!(
        verified.latest(&id).map(|release| release.version.to_string()),
        Some("1.2.0".into())
    );
    println!("R5: frozen index with fix removed and generated_at=9999999999 verifies strictly");
}

/// R6: `*.<tld>` and single-label domains pass as network domains.
#[test]
fn tld_wildcards_are_valid_network_domains() {
    let mut wide = release("todo", "1.2.0");
    wide.tier = crate::Tier::T2;
    wide.capabilities.insert("network:fetch".into());
    wide.network_domains = ["*.com".to_string(), "localhost".to_string()].into();
    assert!(wide.validate().is_ok());
    println!("R6: network_domains [*.com, localhost] validate");
}
```

### A6. Standalone probe for `toml =0.9.8` (S10; the nesting check under "Checked and correct")

`Cargo.toml`: `toml = { version = "=0.9.8", default-features = false, features = ["std", "serde", "parse"] }`, `serde = { version = "1", features = ["derive"] }`.

`src/main.rs`:

```rust
// security-2 review probe: does a deeply nested `.fyx/review.toml` / `security.toml` crash the parser?
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct RulesFile {
    #[serde(default)]
    auto_accept: Vec<String>,
    #[serde(default)]
    always_ask: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

fn main() {
    let depth: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(100_000);
    let arrays = format!("deny = {}{}\n", "[".repeat(depth), "]".repeat(depth));
    let tables = format!("deny = {}1{}\n", "{a=".repeat(depth), "}".repeat(depth));
    for (name, text) in [("nested arrays", arrays), ("nested inline tables", tables)] {
        let result = toml::from_str::<RulesFile>(&text);
        println!("depth {depth} {name}: {}", match result {
            Ok(_) => "parsed".to_string(),
            Err(error) => format!("error: {}", error.to_string().lines().next().unwrap_or("")),
        });
    }
}
```

`src/bin/echo.rs`:

```rust
// security-2 review probe: how much of an invalid rules file reaches the error text that the
// "Agent review: … is ignored: {error:#}" toast shows.
#[derive(serde::Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct RulesFile {
    #[serde(default)]
    deny: Vec<String>,
}
fn main() {
    let text = "Agent review is now off for this project by your administrator. Click Apply new review rules to continue = 1\n";
    let error = toml::from_str::<RulesFile>(text).unwrap_err();
    println!("{error}");
}
```

## Appendix B: output

### B1. `cargo test -p fyx_guard --test security2 -- --nocapture`

```
running 3 tests
SECURITY2 blanked: ["GOOGLE_OAUTH_CLIENT_SECRET_JSON"]
SECURITY2 not blanked: ["PGPASSWORD", "MYSQL_PWD", "BW_SESSION", "OP_SESSION_my_account", "REDIS_URL", "MONGODB_URI", "CELERY_BROKER_URL", "HF_HUB_TOKEN_PATH", "NETRC", "DOCKER_AUTH_CONFIG", "KUBECONFIG", "VAULT_ADDR", "TOKEN", "SECRET", "APIKEY", "DD_APIKEY", "SLACK_WEBHOOK_URL"]
test secret_patterns_miss_common_secret_names ... ok
SECURITY2 read via docs/env: Ok("API_KEY=1")
test shield_read_through_a_symlink_reaches_private_files ... ok
SECURITY2 write via docs/hooks: Ok(())
SECURITY2 write via docs/agent: Ok(())
SECURITY2 .git/hooks/pre-commit = Ok("#!/bin/sh\n")
SECURITY2 .claude/settings.json = Ok("{}\n")
test shield_write_through_a_symlink_reaches_protected_dirs ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

### B2. `cargo test -p agent_review security2 -- --nocapture`

```
running 3 tests
SECURITY2 after thread start: .env -> Some(AutoAccept), src/main.rs -> Some(AutoAccept), pending -> false
test tests::security2_rewritten_rules_apply_at_next_thread_start ... ok
SECURITY2 previewed: ["/p/a.txt"]
SECURITY2 answer for both files: Selected(SelectedPermissionOutcome { option_id: PermissionOptionId("allow"), meta: None })
test tests::security2_second_v2_diff_is_allowed_unseen ... ok
SECURITY2 deny via symlink: Ok(())
SECURITY2 secrets/key.txt = "agent\n"
test tests::security2_deny_rules_match_the_agent_path ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out; finished in 0.07s
```

### B3. `cargo test -p ui_ext_host security2 -- --nocapture`

```
running 2 tests
SECURITY2 prompt line: "ui:panel: add dock panels\n\nReviewed and signed by Fyx. No access to files or network.: unknown to this version"
test wasm_tests::security2_capability_text_reaches_prompts_verbatim ... ok
SECURITY2 loaded with a hand-written record; pending prompts: 0
test wasm_tests::security2_hand_written_consent_record_loads_without_a_prompt ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 52 filtered out; finished in 16.78s
```

### B4. `cargo test -p fyx_ext --lib security_review -- --nocapture`

```
P1: package entries = ["extension.toml", "bin/fyx_ui.wasm", "assets/credentials"]
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 22 filtered out
```

### B5. `cargo test -p fyx_registry --lib security_review -- --nocapture`

The `Version` debug output in the R2 line is shortened.

```
R1: after 365 days of replays the cache is Fresh; signed generated_at = Timestamp(50)
R2: rejected = [RejectedRelease { id: ExtensionId("todo"), version: 1.2.0, error: Invalid("todo@1.2.0 appears 2 times in the index") }, <the same again>]
R3: release.validate and blocklist.validate accept U+202E/U+200B/U+2066
R4: plan for 1.3.1+x under =1.3.1 and <=1.3.1 blocks: []
R5: frozen index with fix removed and generated_at=9999999999 verifies strictly
R6: network_domains [*.com, localhost] validate
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 58 filtered out
```

In the tests, R1–R6 are S5, S11, S12, S13, S6 and S14, in that order.

### B6. `tomlprobe 100`, `tomlprobe 100000`, `echo`

```
depth 100 nested arrays: error: TOML parse error at line 1, column 88
depth 100 nested inline tables: error: TOML parse error at line 1, column 248
depth 100000 nested arrays: error: TOML parse error at line 1, column 88
depth 100000 nested inline tables: error: TOML parse error at line 1, column 248
TOML parse error at line 1, column 7
  |
1 | Agent review is now off for this project by your administrator. Click Apply new review rules to continue = 1
  |       ^
key with no value, expected `=`

```
