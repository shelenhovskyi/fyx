# Upstream diff audit 1 (`v1.28.1..integration` @ `f3718ca`)

Report only; no code changed. Goal: a ranked plan to shrink Fyx's diff outside its own crates before
the next rebase, per `ARCHITECTURE.md` "Fork discipline" ("a few dozen lines").

## Summary

- The pathspec from the task measures **3,020 diff lines (+2,974/−46) in 35 files**. Of these:
  - **2,380** (79%) are the `fyx:ui` host bindings and their tests and fixture, filed under
    `crates/extension_host/src/wasm_host/fyx_ui*`.
  - **105** are `crates/acp_thread/src/fyx_review.rs`.
  - **204** are new root docs and scripts, **93** are `Cargo.lock`, and **42** are
    `assets/settings/fyx.json`.
  - That leaves **196 hand-edited lines in 21 upstream files**, the same figure as
    integration-3's +150/−46.
- The big block costs little at rebase time. It is all new files, so it cannot conflict textually.
  It also no longer needs to live in `extension_host`: it runs its own `Engine`, its own epoch
  thread and its own `Store`, and borrows only two private helpers (about 40 lines). Moving it
  takes 2,382 lines off the surface (proposal 1).
- The edits to upstream files are robust today. Every Fyx hunk except `.cargo/config.toml` still
  applies cleanly to v1.27.0 and v1.26.2 (see Method). The real risks are the few hunks in hot
  files (`acp_thread.rs`, `workspace.rs`, `zed.rs`, root `Cargo.toml`) and `.cargo/config.toml`.
- **No settings schema was added.** `crates/settings*` and `default.json` are untouched. Fyx
  defaults go through the `fyx.json` overlay, and Fyx crates read env vars (`FYX_AGENT_REVIEW`,
  `FYX_AGENT_REVIEW_SCRIPT`, `FYX_UI_DEV_EXTENSIONS`).
- With every in-Fyx proposal applied, the hand-edited surface drops from 196 to about 115 lines.
  The new files in upstream directories drop from about 2,530 lines to 0. Getting to "a few dozen"
  hand-edited lines (about 50) also needs three small upstream PRs (proposals 4, 9 and 10).

## Ranked proposals

Score = lines saved × rebase-conflict risk. The risk scale (1–5) is calibrated on upstream churn
between v1.26.2 and v1.28.1 and on the back-apply test in Method:

- **1:** a new file, or a Fyx-owned module, inside an upstream directory. It cannot conflict
  textually; only renames or API breaks hurt.
- **2:** an edit in a file upstream did not touch in two releases.
- **3:** an edit in a file with low or moderate churn.
- **4:** an edit in a high-churn file or a sorted list, or a hunk that already fails to back-apply.
- **5:** an edit in a hot function or file (`acp_thread.rs`, `workspace.rs`).

Proposals marked *upstream* depend on a Zed PR being accepted.

| # | Proposal | Lines saved | Score | Needs |
|---|---|---|---|---|
| 1 | Move `wasm_host/fyx_ui*` out of `extension_host` into `ui_ext_host` | 2,382 | **2,385** | integrator (Cargo) |
| 2 | One app-glue crate `crates/fyx` behind two init hooks: settings overlay, app identity, URL mapping, `agent_review::init` | 75 net | **142** | integrator |
| 3 | *upstream:* derive product strings and ids from one constant | ~50 | **~133** | Zed PR |
| 4 | Move the write-gate trait to a bottom-of-graph `crates/fyx_hooks` | 103 net | **103** | integrator |
| 5 | Move `registry_init_makes_no_request` into a Fyx crate's tests | 23 | **46** | — |
| 6 | Replace the `.cargo/config.toml` block with env vars in a Fyx script | 11 | **44** | integrator |
| 7 | Root `Cargo.toml`: two trailing `# fyx:` blocks, globbed members | ~0 (risk 4 → 2) | **~26** | integrator |
| 8 | *upstream:* `Dock::panel_index_for_id` | 7 | **21** | Zed PR |
| 9 | Revert the `zed.iss` hunk; brand installers at bundle time | 10 | **20** | — |
| 10 | *upstream:* fetch the ACP registry lazily | 11 | **22** | Zed PR |
| 11 | Put all Fyx actions in one `fyx` namespace | 1 | **4** | — |
| 12 | Measurement hygiene: move docs and `gate.sh`, one canonical pathspec, marker convention | 189 off the metric | 0 (no rebase effect) | — |

Keep as they are, already minimal:

- the `acp_thread.rs` call-site hooks (8 lines, risk 5; see the file table)
- `paths::APP_NAME` (upstream documents it as the fork knob)
- `client::ZED_URL_SCHEME` (but see finding A)
- the binary rename in `crates/zed/Cargo.toml`
- `windows_resources` (build-time, env-keyed)

### 1. Move the `fyx:ui` runtime into `ui_ext_host` (2,382 lines, score 2,385)

**Today.** These files sit inside `extension_host`:

| File | Lines |
|---|---|
| `fyx_ui.rs` | 895 |
| `fyx_ui/tests.rs` | 521 |
| `fyx_ui/fixtures/probe` (`Cargo.lock` 854, `Cargo.toml` 23, `probe.rs` 86, `LICENSE-APACHE` 1) | 964 |

Two hook lines connect them: `wasm_host.rs:1` (`pub mod fyx_ui`) and `extension_host/Cargo.toml:53`
(`ui_ext`). The ARCHITECTURE rationale is "the one place that already owns the wasmtime engine".
That no longer holds:

- `fyx_ui.rs` builds its own `Engine` and epoch thread (`fyx_ui.rs:315-340`) and its own
  `Store`/`Linker`.
- It touches `extension_host` in exactly two places, both private helpers:
  - `super::parse_wasm_extension_version_custom_section` (`wasm_host.rs:847`, 10 lines) decodes
    three big-endian `u16`s.
  - `super::cache_store()` (`wasm_host.rs:606`) is the shared `IncrementalCompilationCache`, a
    28-line moka wrapper at `wasm_host.rs:1009`.
- The only consumer is `ui_ext_host`: `backend.rs:5,9`, `registry.rs:11` and `wasm_tests.rs:13`.
  `ui_ext_host` uses `extension_host` for nothing else.

**Change.** Move `fyx_ui.rs` to `crates/ui_ext_host/src/runtime.rs` (or a new `ui_ext_runtime`
crate, if keeping wasmtime out of the GPUI crate's build matters). Move `tests.rs` next to it, and
the probe to `crates/ui_ext_host/fixtures/probe` beside the existing `status_item` fixture. Then:

- Copy the 10-line section parser.
- Either give the runtime its own copy of the cache store (about 28 lines plus a `moka` dep), or
  drop incremental compilation for UI components. They are compiled once per load.
- In `ui_ext_host/Cargo.toml`, add `wasmtime`, `wasmtime-wasi` and `semver`, and remove
  `extension_host`.
- Drop the two hook lines.

**Saved.** 2,382 lines off the surface. About 40 lines are added inside Fyx.

**Side benefits:**
- Upstream `extension_host` changes stop rebuilding Fyx's runtime.
- `cargo test -p extension_host` goes back to upstream's tests only.
- The "shared `<target>/fyx-ui-fixtures`" coupling noted in `ui_ext_host/src/wasm_tests.rs:24`
  goes away.
- The two fixtures can share one fixture workspace and one lockfile, which saves about 850
  duplicated lockfile lines inside Fyx's own tree.

**Risk.** Low. Update the ARCHITECTURE line "Host-side WIT bindings live in a new module inside
`extension_host`". The `bindgen!` path becomes `../ui_ext/wit` relative to `ui_ext_host`, which is
the same depth.

### 2. A `crates/fyx` app-glue crate (75 net lines, score 142)

**Today.** The glue is scattered across five upstream files:

- `main.rs:1822-1837` `fyx_default_settings` (17 lines)
- `main.rs:498-499` `set_app_identity` (2)
- `main.rs:505` settings hook (1)
- `main.rs:756` `ui_ext_host::init` (1)
- `main.rs:214-217` argument rewrite (6 diff lines)
- `open_listener.rs:125-131` `fyx_url_to_internal` (8)
- `agent_ui/Cargo.toml:31` and `agent_ui.rs:687` for `agent_review::init` (2)
- `zed/Cargo.toml:220-221` (2)
- the overlay itself, `assets/settings/fyx.json` (42)

**Change.** Add a Fyx crate `crates/fyx` (lib root `fyx.rs`) that depends on `settings`,
`release_channel`, `ui_ext_host` and `agent_review`, and exposes:
- `fyx::init_early(cx)`, called once after `settings::init(cx)`. It sets the app identity from
  `ReleaseChannel` and layers the overlay, embedded with `include_str!`, so `fyx.json` leaves
  `assets/`.
- `fyx::init(cx)`, called where `ui_ext_host::init` sits now. It runs `ui_ext_host::init` and
  `agent_review::init`. `agent_review::init` does not depend on anything `agent_ui::init` sets up
  (`agent_review.rs:38-59`), so `agent_ui` loses both of its Fyx lines.
- `fyx::url_to_internal(url)`. `open_listener.rs` keeps a one-line call.

Also replace the six-line argument loop in `main.rs` with `|| arg.starts_with("fyx://")` in
`parse_url_arg` (`main.rs:1844`). URLs reach `OpenRequest::parse` either way, and the rewrite
happens there. **Verify** this with the Windows single-instance forwarding path before dropping the
loop.

**After.** `main.rs` keeps about 9 diff lines (the product-name lines from proposal 3 plus the two
init calls), `open_listener.rs` keeps 1, `agent_ui` keeps 0, and `zed/Cargo.toml` keeps 1 dependency
line. Root `Cargo.toml` gains 2 lines (covered by proposal 7's glob).

While moving it, make `fyx_default_settings` return a `Result` and log the error. It currently
uses `panic!` and `expect` (`main.rs:1828,1835`), against the repo's no-panic rule.

### 3. *Upstream:* one product constant (about 50 lines, score about 133)

**Today.** About 52 diff lines are literal "Zed"→"Fyx" swaps:

| File | Lines | What | Risk |
|---|---|---|---|
| `release_channel/src/lib.rs` | 27 | the `app_identifier`, `display_name` and `app_id` matches | 2 |
| `app_menus.rs` | 8 | | 2 |
| `zed.rs` | 6 | About title and two scheme toasts | 4 |
| `main.rs` | 6 | two "failed to launch" strings and the User-Agent | 3 |
| `workspace.rs` | 5 | window-title test expectations | 5 |

**Change.** Open a Zed PR that derives these from `paths::APP_NAME`, plus an app-id prefix constant
and `ZED_URL_SCHEME`. Upstream already documents `APP_NAME` with "Forks should change this", so a
"fork-friendly branding" PR has a precedent. The test would compare against
`ReleaseChannel::Stable.display_name()` rather than a literal.

**After.** About 2 lines (the app-id prefix) on top of the existing `APP_NAME` and
`ZED_URL_SCHEME` lines.

**If upstream declines,** keep the edits. These files had zero upstream churn in two releases.

### 4. Bottom-of-graph `crates/fyx_hooks` for the write gate (103 net lines, score 103)

**Today.** `acp_thread/src/fyx_review.rs` (105 lines) holds the `AgentWriteGate` trait, its
global, `gate_agent_write` and `feedback_error`. It cannot live in `agent_review`, because
`agent_review` depends on `acp_thread`.

**Change.** Move it into a tiny Fyx crate `crates/fyx_hooks` that depends on `gpui`, `language`
and `agent-client-protocol`. Pass `session_id` instead of `&WeakEntity<AcpThread>`, which is the
only reason it references `AcpThread`. `acp_thread` then has one `Cargo.toml` line instead of
`pub mod fyx_review`, and its two call sites are unchanged.

**After.** The new file leaves the upstream directory, at the cost of +1 dependency line and +2
root `Cargo.toml` lines.

This is the place for every future "trait object registered at init" hook. W6's editor decoration
hook should land here rather than as a new module inside `editor`.

### 5. Move the new registry test out of `project` (23 lines, score 46)

`project/tests/integration/agent_registry_store.rs:11-32` is a new 23-line test that guards a Fyx
invariant. It uses only public API: `AgentRegistryStore::init_global` and `is_fetching`, with
`FakeHttpClient`. Move it to `crates/fyx/tests/` and widen it to "no request at startup".

The three `store.refresh(cx)` lines (45, 126, 178) must stay while `init_global` stays changed.

### 6. `.cargo/config.toml` → env vars (11 lines, score 44)

This is the only hunk that fails to back-apply onto v1.26.2: upstream edited the `rustflags`
context. It forces `rustc-wrapper = "sccache"` on anyone who builds the fork, including future
release CI, and changes the dev profile for everyone.

**Change.** Put `RUSTC_WRAPPER=sccache`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only` and
`CARGO_INCREMENTAL=0` in a sourced `script/fyx-env`, used by `script/gate.sh` and documented in
`.claude/CLAUDE.md`. The Cargo-level alternative is a user `$CARGO_HOME/config.toml`.

### 7. Root `Cargo.toml` layout (about 0 lines, risk 4 → 2, score about 26)

**Today.** 11 lines in 5 hunks, all **unmarked** for `git grep 'fyx:'`. The demo lines carry
`# fyx` with no colon. All five hunks sit inside upstream's alphabetical lists (`crates/agent_*` and
`crates/ui_*` both change most releases; the file churned +33/−16 over two releases).

**Change.** Use one `# fyx:` block at the end of `members`: `"crates/agent_review"`,
`"crates/fyx*"`, `"crates/ui_ext*"` and `"extensions/fyx_*"`. Cargo accepts globs, and the globs
cover proposals 2 and 4 too. Add one `# fyx:` block at the end of the path dependencies.

About 12 lines in 2 hunks at block ends instead of 11 lines in 5 mid-list hunks. Check that Zed's
tooling (`script/clippy`, license checks) is fine with glob members.

### 8. *Upstream:* `Dock::panel_index_for_id` (7 lines, score 21)

`dock.rs:550-555` sits right next to upstream's `panel_for_id` and `panel_index_for_*`. Upstream
has no public way to find the index of one of several panels of the same type:
`panel_index_for_persistent_name` matches per type. This is a natural one-function Zed PR.

### 9. Revert the `zed.iss` hunk (10 lines, score 20)

The hunk registers `fyx://` but launches `{app}\Zed.exe` (integration-3 finding 5). Two more
installer surfaces still say `zed`:
- the macOS bundle metadata (`crates/zed/Cargo.toml:295,304,313,322`: `osx_url_schemes = ["zed"]`)
- the Linux desktop file (`crates/zed/resources/zed.desktop.in:16`: `x-scheme-handler/zed`)

So installer rebranding is a third done, by in-tree edits. Revert the hunk now. When packaging is in
scope, apply installer branding from a `script/fyx-bundle-*` step that templates copies at bundle
time, so none of it is an upstream edit.

### 10. *Upstream:* lazy ACP registry fetch (11 lines, score 22)

`agent_registry_store.rs:149` drops the eager `refresh()` (−6/+2), and the tests add three explicit
`refresh` lines. Dropping a startup request to `cdn.agentclientprotocol.com` until the registry is
needed is a reasonable upstream PR.

### 11. One `fyx` action namespace (1 line, score 4)

`zed.rs:6208,6238` adds `agent_review` and `fyx_ui` to the namespace-list test, a sorted list in a
file that churned +392 over two releases. Merging them into one `fyx` namespace saves a line. It is
cheap only before keymaps depend on the names.

### 12. Measurement hygiene (189 lines off the metric, no rebase effect)

- Move `ARCHITECTURE.md` (78) and `PLAN.md` (64) to `docs/fyx/`. Rename `script/gate.sh` (47) to
  `script/fyx-gate` so it matches `script/fyx-*`. Keep `.claude/CLAUDE.md` and `FYX_UPSTREAM`.
- One canonical pathspec, for example a `script/fyx-diff-stat`. This task's pathspec and
  integration-3's disagree: integration-3 also excludes `fyx_ui*`, `*.md`, `Cargo.lock`, `fyx.json`
  and `gate.sh`. Reviews will keep reporting different numbers until there is one. Suggested:
  exclude Fyx crates, docs and scripts, and count `Cargo.lock` separately.
- Marker convention: `git grep 'fyx:'` also matches the WIT namespace `fyx:ui`, which gives 37 false
  hits in `fyx_ui.rs` and its tests. Standardize on `fyx: ` (with a space; every real marker but
  `Cargo.toml`'s already uses it) and document `git grep -n 'fyx: '`.

## Per-file inventory

"Marker" means a `fyx: ` comment on or directly above the change.

| File | ± | What changed / why | Marker | How to shrink |
|---|---|---|---|---|
| `.cargo/config.toml` | +11 | sccache, line-tables debug info, no incremental (parallel worktree builds) | yes (26) | #6 env vars |
| `.claude/CLAUDE.md` | +14 | agent rules for Fyx | n/a (new doc) | keep |
| `ARCHITECTURE.md`, `PLAN.md` | +78, +64 | Fyx docs at the root | n/a | #12 move to `docs/fyx/` |
| `FYX_UPSTREAM` | +1 | pinned base tag | n/a | keep |
| `script/gate.sh` | +47 | merge gate | n/a | #12 rename `script/fyx-gate` |
| `Cargo.lock` | +93 | generated | n/a | regenerate on rebase; exclude from metric |
| `Cargo.toml` | +11 | members and path deps for Fyx crates and demos | **no** (`# fyx` without colon, 4 lines bare) | #7 |
| `assets/settings/fyx.json` | +42 | Fyx default overlay (server_url to loopback, telemetry and updates off, etc.) | header comment | #2 embed from `crates/fyx` |
| `crates/acp_thread/src/acp_thread.rs` | +7/−1 | `mod fyx_review`; gate before applying a write; feedback error after save | yes (9, 8017, 8089) | **keep the call sites** (two lines in a hot function, already minimal); #4 removes the `mod` line |
| `crates/acp_thread/src/fyx_review.rs` | +105 | gate trait and global | yes (1) | #4 `fyx_hooks` |
| `crates/agent_ui/Cargo.toml`, `agent_ui.rs` | +1, +1 | `agent_review` dep and `init` | yes | #2 call from `fyx::init` |
| `crates/client/src/client.rs` | +1/−1 | `ZED_URL_SCHEME = "fyx"` | yes | keep (see finding A) |
| `crates/extension_host/Cargo.toml`, `wasm_host.rs` | +1, +1 | `ui_ext` dep, `mod fyx_ui` | yes | #1 |
| `crates/extension_host/src/wasm_host/fyx_ui.rs` | +895 | `fyx:ui` wasmtime runtime | file is Fyx-owned | #1 |
| `…/fyx_ui/tests.rs` | +521 | runtime tests | Fyx-owned | #1 |
| `…/fyx_ui/fixtures/probe/*` | +964 | guest fixture, own workspace and lockfile | Fyx-owned | #1, then share one fixture lockfile |
| `crates/paths/src/paths.rs` | +1/−1 | `APP_NAME = "Fyx"` | yes | keep (upstream's fork knob) |
| `crates/project/src/agent_registry_store.rs` | +2/−6 | no eager registry fetch | yes | #10 upstream |
| `crates/project/tests/integration/agent_registry_store.rs` | +26 | **test expectations**: new no-request test (23), explicit `refresh` (3) | yes | #5 move the 23; the 3 go with #10 |
| `crates/release_channel/src/lib.rs` | +15/−12 | mutex/pipe names, display name, bundle id | yes (block markers 46, 208, 231) | #3 upstream |
| `crates/windows_resources/src/windows_resources.rs` | +6/−5 | exe version-info product and company | partly: CompanyName (104) only by reference | keep; mark line 104 |
| `crates/workspace/src/dock.rs` | +7 | `panel_index_for_id` | yes | #8 upstream |
| `crates/workspace/src/workspace.rs` | +3/−2 | **test expectations**: window title says Fyx | yes | #3 (test uses `display_name()`) |
| `crates/zed/Cargo.toml` | +4/−2 | bin and default-run `fyx`; `ui_ext_host` dep | yes | keep the bin rename; #2 makes the dep `fyx` |
| `crates/zed/resources/windows/zed.iss` | +5/−5 | registers `fyx://` (still `Zed.exe`) | yes | #9 revert |
| `crates/zed/src/main.rs` | +29/−4 | product strings, args rewrite, identity, settings overlay, User-Agent, `ui_ext_host::init` | yes | #2, #3 |
| `crates/zed/src/zed.rs` | +5/−3 | scheme toasts, About title; **test expectations**: two action namespaces | yes | #3, #11 |
| `crates/zed/src/zed/app_menus.rs` | +4/−4 | menu product names | **partly**: only line 71; 74, 106 and 112 are bare | #3; until then mark them |
| `crates/zed/src/zed/open_listener.rs` | +9 | `fyx://` → `zed://` mapping | yes | #2 move the function body |

Test-expectation edits total 33 lines: `agent_registry_store.rs` 26, `workspace.rs` 5, `zed.rs` 2.
All but the 3 `refresh` lines can go via proposals 3, 5 and 11.

## Findings outside the size question

**A. `fyx://` channel links can never parse.** `ZED_URL_SCHEME` is `"fyx"`, so `parse_zed_link`
only accepts `fyx://channel/...` (`client.rs:1955`). But `OpenRequest::parse` rewrites `fyx://` to
`zed://` first (`open_listener.rs:166`), then calls `parse_zed_link` (`open_listener.rs:206`). The
same applies to CLI arguments, rewritten at `main.rs:214` and then checked at `main.rs:1847`. So
neither `fyx://channel/…` nor `zed://channel/…` parses.

The impact is nil while collaboration is off. Still, the two mechanisms (renaming the const and
rewriting at the edge) contradict each other. Pick one:
- Keep `ZED_URL_SCHEME = "zed"` as the internal namespace and give OS registration its own Fyx
  scheme (`install_cli/src/register_zed_scheme.rs:13`). This is the same size and coherent.
- Or rewrite after `parse_zed_link`.

**B. Rebrand gaps.** macOS bundle `osx_url_schemes` and the Linux `.desktop` handler still register
`zed://` (proposal 9). Integration-3 finding 5 lists the rest.

**C. Unmarked edits.** All 11 `Cargo.toml` lines, `app_menus.rs:74,106,112` and
`windows_resources.rs:104`. Everything else is marked, at least per block.

## Method

- Surface: `git diff v1.28.1 --stat` with the task's pathspec; markers with `git grep -n 'fyx:'`.
  Unmarked hunks were found by scanning `git diff -U0` for hunks without a `fyx: ` line.
- Upstream churn per touched file, as diff lines from v1.26.2 and v1.27.0 to v1.28.1:

  | File | 1.26.2→1.28.1 | 1.27.0→1.28.1 |
  |---|---|---|
  | `acp_thread.rs` | +1653/−1218 | +588/−76 |
  | `workspace.rs` | +959/−214 | +297/−176 |
  | `zed.rs` | +392/−38 | +1/−1 |
  | root `Cargo.toml` | +33/−16 | +9/−5 |
  | `dock.rs` | +20/−9 | +6/−1 |
  | `main.rs` | +13/−7 | +12/−6 |
  | `open_listener.rs` | +3/−7 | +3/−7 |
  | `zed/Cargo.toml` | +6/−6 | +2/−2 |
  | `.cargo/config.toml` | +1/−1 | 0 |

  `client.rs`, `paths.rs`, `release_channel`, `windows_resources`, `app_menus.rs`, `zed.iss`,
  `agent_registry_store.rs` and its tests, `wasm_host.rs` and `agent_ui/Cargo.toml` had no churn.
- Back-apply test: each file's Fyx patch (`git diff v1.28.1 HEAD -- <file>`) checked with
  `git apply --check` against v1.27.0 and v1.26.2 worktrees, at context 3 and 1. Every hunk applies
  to both, with offsets of up to −413 lines in `acp_thread.rs`, except `.cargo/config.toml` on
  v1.26.2. Two releases of upstream churn would not have conflicted with today's hooks. The
  scores therefore weight churn (likelihood of a future conflict) more than current breakage.
