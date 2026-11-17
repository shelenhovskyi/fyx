# Judge review: integration-3

**Scope.** `git diff 9cdf8a61b5...integration` at `377d5ca39f`. That is `fyx/main` `df50e7bc7c` plus
W5's `03d37b8167`: 65 commits, 110 files, +15336/−431.

**Streams in scope:**
- W1: rebrand and no network at startup.
- W3: renderer and containers.
- W4b: gallery demo and demo packaging.
- W5 phase B: review pane and script driver.
- W2b: property tests.
- W8b: adapter security fixes.
- Author docs in `docs/fyx/`.
- W0: gate lockfile check, `script/fyx-prune-target`, and test-expectation edits for the rebrand.

**Method.** Nothing was compiled. Two separate reviewers read W3 (`crates/ui_ext_host`) and
`docs/fyx/` against the code. Their findings are marked as theirs below. Everything else was read
here.

## Verdict

Phase 1 is functionally complete, and the architecture rules hold (see the W3 section). The
upstream surface grew with W1, as expected: +150/−46 in 21 files. Every hand edit carries a
`fyx:` marker except the `Cargo.toml` member lines.

New findings that need action:
- **Finding 1 (W5):** a "dev-only" script driver that can accept agent edits and dispatch any
  action is compiled into every build and armed by an environment variable.
- **Finding 2 (W3/W4):** UI extensions load only from `FYX_UI_DEV_EXTENSIONS`. There is no
  installed-extension path, so phase-4 test T1 has to be run in dev mode.

The W3 reviewer found that the renderer meets all seven integration-1 contract points, and that
12 of the 14 earlier W3 items are fixed. They found one new Medium bug (finding 8: an endless
`list-rows` loop after a resync), which was confirmed here.

## Status of earlier findings

### integration-1

| # | Finding | Status |
|---|---|---|
| 1 | Budget not enforced while blocked in WASI | Resolved (integration-2). |
| 2 | A rejected update desyncs the SDK mirror | SDK side resolved; W3 side: **resolved**: remount on failure (`surface.rs:436-462`), bounded at 3 (`:444-458`). |
| 3 | No order between pushes and the reply | Resolved (integration-2). |
| 4 | Row-local ids | **Resolved in W3**: buttons inside rows get no handler (`view.rs:491-496`), a row click is sent as `row-click` (`:657-659`), and row ids stay out of the surface map (`surface.rs:38-59`). 0.2 doc item tracked as `tasks/wit-0.2.md` #3. |
| 5 | Row refresh | W2 and W4 parts resolved; W3 part: **resolved**: the rows of touched list nodes are requested again (`surface.rs:416-420, 487-492`). `invalidate-rows` tracked as `wit-0.2.md` #1. |
| 6 | Input echo | **Resolved**: `set_input_locally` (`surface.rs:243-245, 322-339`), `sync_input` (`view.rs:221-240`). A gap remains after an SDK remount (finding 10). |
| 7 | Loading UI extensions from disk | W2 part resolved. W3 calls `load_from_dir`, but only for `FYX_UI_DEV_EXTENSIONS` paths (new finding 2). |
| 8 | No version section from the SDK | Resolved. |
| 9 | `push-update` promises timers | Tracked as `wit-0.2.md` #2. The WIT and SDK docs still say "timers". |
| 10 | Pushed updates are unbounded | **Resolved** (`73617d6434`). `MAX_PUSHES_PER_CALL = 64` (`fyx_ui.rs:71`); one more push traps and disables the extension (`fyx_ui.rs:279-280`). Each push is still capped by `max_nodes_per_update` only when it is applied. |
| 11 | Mount capped at 2048 nodes, tree at 16384 | Open (W2). `Tree::replace` still checks `max_nodes_per_update` (`tree.rs:168`). The SDK and docs now agree on 2048, so this is a documented limit, not a bug. Close it if the docs say so (see the docs section). |
| 12 | Doc drift; icon names | Counter doc resolved. Icon fallback: resolved (`view.rs:804-806`, fallback `blocks`). Frozen list tracked as `wit-0.2.md` #4. |
| 13 | `Cargo.toml` members not marked | **Open (W0).** `Cargo.toml:9, 231-233, 299, 511-513` (`agent_review`, `ui_ext*`) still have no marker; only the three demo members do. |
| F1 | Duplicate version section | Resolved. |
| F2 | Import fallback untested | **Open (W2).** No test reaches `api_version_from_imports`. |

### integration-2

| # | Finding | Status |
|---|---|---|
| 1 | Stale `Cargo.lock` | **Resolved.** Committed (`cf39d9716f`, `b5650c7323`, `7743576c8f`). `script/gate.sh:40-44` now fails when any lockfile differs from `HEAD` after the run (`4eef241480`). |
| 2 | `vsix-convert` writes outside `--out` | **Resolved** (`cf8c6f6b4a`). `writeOutput` (`vsix.ts:250-271`) rejects unsafe ids and checks every target with `isInside`. `zip.ts` rejects `\`, `:`, `..` and empty segments. |
| 3 | `fyx_review.rs` placement | Answered. It stays in `acp_thread`. |
| 4 | A partial preview is answered `Allow` | **Resolved** (`1e72027a49`). The new `ApplyAcceptedAndDeny` (`agent_review.rs:404-412`): the host writes the accepted hunks itself (`write_accepted_hunks`, `:530-563`), then denies the tool call. |
| 5 | An error for a write that landed | Resolved by wording: the feedback tells the agent not to write the edit again (`1e72027a49`). |
| 6 | A held write outlives its thread | **Resolved.** `on_release` calls `close_session` (`agent_review.rs:48-51`). |
| 7 | Pushes of a failed call out of order | W2 side resolved: `ReplyError` carries them (`fyx_ui.rs:213`). W3 side: **resolved**: pushes are applied first on every failure path (`surface.rs:174-181, 360-365, 529-533`). |
| 8 | Fourth copy of the tree rules | **Resolved** (`587d2173bc`). The SDK depends on `ui_ext` and re-exports `ui_ext::TreeLimits` (`ui_ext_sdk.rs:72`). `list_row` validates with `ui_ext::Tree::from_row` (`:375-384`). |
| W8-1 | Zip bombs | Resolved (`cf00cdb65f`): `maxOutputLength`, 64 MiB per entry, 256 MiB total, lazy reads. |
| W8-2 | Unpack cache trusted | Resolved (`d3666a4253`): `0700` per-user folder, staging folder plus rename, completion marker (`host.ts:58-84`). |
| W8-3 | Sidecar mirror drift | Resolved (`d0f019dd8b`). This changes the protocol: `push-update` is now a request (see finding 6). |
| W8-4 | Local edits before the host confirms | Resolved (`47477ce09b`), per the W8 notes; not re-read here. |
| W8-5 | `InputBox` re-render and `password` | Resolved (`d5abc34de1`). `password: true` now throws. |
| W8-6 | Silent no-ops | Resolved (`a23edb712a`). Probe mode needs `--fyx-coverage-probe`; the environment variable alone is ignored with a warning (`main.ts:48-49`). |
| W8-7 | Mock host misses the 2048 cap on `replace`/`mount`/rows | Open (W8, Low). |
| W8-8 | TreeView row cap per level | Open (W8, Low). |
| W8-9 | No sandbox check in the sidecar | **Open (W8).** `main.ts` still never checks `process.permission`. This matters once phase 3 launches the sidecar from Rust. |
| W8-10 | Row id base | Open (W8, Low). |
| — | Two `zip.ts` copies | Mitigated: a test keeps them byte-identical (`cf8c6f6b4a`). |

## New findings, most severe first

### 1. Medium: the review script driver ships in every build and can approve agent edits (W5)

`script_driver::init` runs unconditionally (`agent_review.rs:58`). It has no `cfg`, no feature
flag and no release-channel check. When `FYX_AGENT_REVIEW_SCRIPT` names a file
(`script_driver.rs:43-67`), the script can:
- accept, reject and submit hunks in review panes;
- dispatch **any** registered action with JSON arguments, through `cx.build_action(&name, data)`
  (`script_driver.rs:125-126, 241`).

The file header calls it "dev-only". In a release build, anything that can set an environment
variable for Fyx can approve agent writes the user never saw: a launcher, a shell profile, a
parent process such as an agent's terminal starting `fyx`. This breaks "policy and consent stay in
the host" (ARCHITECTURE.md).

**Fix:** compile the driver only under `#[cfg(any(debug_assertions, feature = "review-script"))]`
or, at minimum, run it only on `ReleaseChannel::Dev`. Log a warning whenever it is active.

### 2. Medium: UI extensions load only from a dev environment variable (W3 owns the loader, W0 decides)

`ui_ext_host::registry::init` calls only `load_dev_extensions` (`registry.rs:194-206, 339-350`),
which reads `FYX_UI_DEV_EXTENSIONS`. The paths come from the variable alone. No installed
extensions directory is scanned and there is no install action. `script/fyx-build-demos` prints
the variable for the user to set.

That is fine for the sprint, but:
- It needs to be stated in `docs/fyx/` (see the docs section).
- `docs/fyx/manual-tests.md` (T1–T3) must say to set the variable.
- An installed-extensions path should be added to the phase-3 list.

W0 decides whether 0.1 ships without one.

### 3. Low: the agent is not told when the host could not write the accepted hunks (W5)

With `ApplyAcceptedAndDeny`, a failure in `write_accepted_hunks` is only logged
(`agent_review.rs:503-509`). The text may have moved, or occur twice (`locate_fragment`,
`:565-579`). The agent receives only the denial: ACP has no channel for feedback on a permission answer
(`tasks/progress/W5.md`). The user approved hunks that never landed, and sees no error. That contradicts `.rules` ("errors propagate to the UI layer").

**Fix:** show the failure on the review pane, or as a workspace notification. Then either keep the
review open so the user can retry, or deny it with a clear message.

### 4. Low: the upstream diff grew past "a few dozen lines"; two blocks can move out (W1)

The upstream surface is +150/−46 in 21 files (table below), up from +27/−1. Most of it is
unavoidable rebrand text, plus 29 lines of updated test expectations. Two blocks are Fyx logic
living in upstream files:
- `fyx_default_settings` (`crates/zed/src/main.rs:1822-1837`, 17 lines);
- `fyx_url_to_internal` (`crates/zed/src/zed/open_listener.rs:125-131`).

Both could move into a Fyx crate, leaving one call line each. There is no Fyx crate that `zed`
already depends on for branding; `ui_ext_host` would do, or a small `fyx_brand` crate (W0).

### 5. Low: rebrand gaps left as known (W1)

These are listed in `tasks/progress/W1.md:119-121` and confirmed:
- `crates/cli` still looks for `../Zed.exe` and `./zed.exe` (`crates/cli/src/main.rs:1277-1279`),
  so the CLI cannot find the renamed `fyx.exe`.
- `crates/zed/src/main.rs:1705` still names the clap command `zed`, so `--help` says `zed`.
- `zed.iss` registers `fyx://` but points it at `{app}\Zed.exe` (`zed.iss:1258-1262`). The rest
  of the installer is still Zed's. Either finish the installer or revert this hunk, since
  installers are out of scope.

Add the CLI and the `--help` name to phase 3. The ARCHITECTURE branding table promises a `fyx` CLI.

### 6. Low: the 0.2 backlog misses `push-update` returning a result (W0)

W8b made the sidecar's `push-update` a JSON-RPC request, so it learns when the host refuses an
update (`tasks/progress/W8.md`, "New protocol"). The WIT `host.push-update` returns nothing, so a
WASM guest still cannot learn that its push was rejected. Add it to `tasks/wit-0.2.md` as item 9:
`push-update: func(...) -> result<_, string>`.

### 7. Low: an instruction in `.rules` keeps making workers edit `README.md` (W0 / owner)

`.rules` contains a "HARD RULE": prepend a two-line review banner to `README.md` whenever source
files change. Workers obey it, and W0 had to revert it twice (`b6667fc780`, `d151b4d136`). W8b
declined it as outside its scope. It conflicts with every stream's "Owns" list.

**Fix:** the owner decides whether the rule stays. If it stays, give `README.md` to W0 so only
the integrator adds the banner. If not, remove the rule.

### 8. Medium: an endless `list-rows` loop after a resync (W3)

Found by the W3 reviewer and confirmed here.

`set_tree` clears `lists` (`surface.rs:224-229`), so each list's `RowCache` starts again at
generation 0. A fetch issued before the resync carries the old generation g > 0 and is still
queued behind the `mount`. If the list keeps its node id after the remount, that reply lands in
the new cache:
- it clears the new fetch's `in_flight` marker (`:543`);
- it stores rows tagged g (`:557`). This affects hand-written guests with fixed ids, and the WIT
  allows fixed ids; the SDK hands out new ids.

From then on:
- every generation-0 reply skips those rows, because "never replace a newer generation"
  (`:548-554`);
- `request_rows` still treats them as missing, because `row.generation != generation`
  (`:487-492`).

The result is one `list-rows` call per round trip with no input, and the remount bound does not
stop it.

**Fix:** drop replies whose generation is not the cache's current generation. Alternatively, keep
the generation counter on `SurfaceState` so it never resets, or tag each fetch with a mount epoch.

### 9. Low: other W3 findings from the separate reviewer

These were not re-verified here.

- **A click on a stale row is sent.** `row_generation` captures the list's *current* generation
  (`surface.rs:256, 318-319`). A row drawn from the previous generation can therefore still be
  clicked until its replacement arrives. **Fix:** capture the generation of the drawn `CachedRow`.
- **The remount counter resets only on an applied event reply** (`surface.rs:351`). A status item
  fed only by pushes accumulates the count over the whole session. After four desyncs, each of
  which recovered, it fails for good. **Fix:** also reset the counter on an accepted push or row
  answer.
- **Retry on a disabled instance cannot work.** W2 never clears `disabled` (`fyx_ui.rs:707-708`),
  so Retry (`surface.rs:194-201`) makes one refused call and fails again. **Fix (W3 + W2):** hide
  Retry for a disabled instance, or instantiate it again.
- **Debounced typing is flushed only within one view** (`view.rs:100-110`). Type in a panel, then
  click the same extension's status item within 150 ms, and the click arrives first.

### 10. Low: typed text is lost when an SDK remount changes the input's id (W3)

w3-deep-1 #8 is fixed only when the input keeps its id. An SDK remount hands out new ids (the
counter goes from ids 1–7 to 8–14). `sync` then drops the old editor and its pending debounce
(`view.rs:145-146`) before `sync_input` runs (`:226-231`).

w3-deep-1 #6 is also only partly fixed: the endless loop is gone, but each change still makes two
row calls (`surface.rs:496-503, 575-581`).

## Docs against code

A separate reviewer checked `docs/fyx/extension-authoring.md`, `sdk-reference.md`,
`limits-and-gotchas.md` and the demo READMEs against the code. Items D1 and D2 were confirmed here.

**Verified correct.** The WIT names, every SDK signature, the constants (`API_VERSION`, the
section bytes, `ROW_NODE_ID_BASE`, `TreeLimits` 2048/16384), the budgets (200 ms, 2 s, 10 ms
epoch, +50 ms), the 64-push cap, the `UiManifest` keys and rules, the icon fallback, the list
constants, the 150 ms debounce and the token mapping. The code examples match the current API,
though they were read, not compiled.

**Cause of the drift.** The docs (`b7aaa1d3e2`) landed before W3's fixes for w3-deep-1
(`bd7580ca9c`), so they describe the older renderer.

| # | Sev | Doc says | Code does |
|---|---|---|---|
| D1 | Medium | The renderer drops the pushes of an `Err` or a rejected reply (`sdk-reference.md:485-488, 495`; `limits-and-gotchas.md:222, 227-230`). The "do not push then return `Err`" gotcha rests on that, and `limits:80` says "the tree is unchanged" after an `Err`. | Applies them on all three paths through `reply_error_pushes` (`surface.rs:595`), in `apply_reply`, `mount` and `receive_rows`. |
| D2 | Medium | A rejected update "triggers one `mount`" and the surface "stays enabled" (`limits:81`, `sdk-reference.md:111`, `extension-authoring.md:245-249`). | After 3 remounts with no applied reply in between, the surface fails (`MAX_REMOUNTS_WITHOUT_INPUT`, `surface.rs:25, 444-454`). |
| D3 | Medium | A failed `mount` is "not retried" (`limits:79`, `sdk-reference.md:110`, `extension-authoring.md:221-223`). | A failed surface shows **Retry**, which mounts again (`view.rs:321-335`, `surface.rs:194-201`). |
| D4 | Medium | An applied `set-props` on an input always overwrites the editor when the text differs (`limits:168-170`). | It is ignored while typing is still inside the debounce window (`view.rs:226-231`). Undocumented: any other event first flushes pending typing, so a click is preceded by an `input-changed` (`view.rs:91-98`). |
| D5 | Low | A newer queued `input-changed` replaces the older one (`extension-authoring.md:255`, `limits:162-163`). | Only when it is the last queued event (`surface.rs:246-254`). |
| D6 | Low | Status item errors only reach the log (`limits:93`). | A failed status item shows the error and Retry inline (`view.rs:321-336`). |
| D7 | Low | `grow` means `flex_1` (`sdk-reference.md:165`). | `flex_grow_1()` (`view.rs:467`). |
| D8 | Low | The build script "builds both demos" (`extensions/fyx_demo_counter/README.md:16`). | Three: counter, todo and gallery (`script/fyx-build-demos:11`). |
| D9 | Low | The counter crate doc lists "a panel reopen" as a cause of remounts (`fyx_demo_counter.rs:19-20`). | Reopening does not remount: one mount per `SurfaceState` (`surface.rs:113`). `extension-authoring.md:222-223` agrees with the code. W4 fixes the crate doc. |
| D10 | Low | `view.rs` and `surface.rs` line citations in all three docs. | Drifted since `bd7580ca9c`. For example, the icon fallback moved from `view.rs:749` to `:803`, and `ROW_OVERSCAN` from `surface.rs:20` to `:30`. |

**Owner.** W7 owns `docs/`. The author-docs branch has merged, so W7 fixes D1–D8 and D10 in the
next cycle, and W4 fixes D9. If the docs stream is still live, hand the list to it instead.

## Upstream hooks (`v1.28.1..integration`)

Measured with
`git diff v1.28.1 HEAD --stat -- . ':!crates/ui_ext*' ':!crates/agent_review' ':!docs' ':!tasks' ':!extensions/fyx_*' ':!crates/extension_host/src/wasm_host/fyx_ui*' ':!tools/vsix-convert' ':!sidecars' ':!*.md' ':!FYX_UPSTREAM' ':!script/gate.sh' ':!script/fyx-*' ':!assets/settings/fyx.json' ':!Cargo.lock'`.

| File | ± | `fyx:` hooks (line) | Owner |
|---|---|---|---|
| `.cargo/config.toml` | +11 | 26: sccache, line tables, no incremental | W0 |
| `Cargo.toml` | +11 | 265-267: demo members. **Unmarked:** 9, 231-233, 299, 511-513 | W0 |
| `crates/acp_thread/src/acp_thread.rs` | +7/−1 | 9: `mod fyx_review`; 8017: review gate; 8089: feedback after save | W5 |
| `crates/acp_thread/src/fyx_review.rs` | +75 (new file) | 1: gate trait and global | W5 |
| `crates/agent_ui/Cargo.toml` | +1 | 31: `agent_review` dependency | W5 |
| `crates/agent_ui/src/agent_ui.rs` | +1 | 687: `agent_review::init` | W5 |
| `crates/client/src/client.rs` | +1/−1 | 1930: `ZED_URL_SCHEME = "fyx"` | W1 |
| `crates/extension_host/Cargo.toml` | +1 | 53: `ui_ext` dependency | W2 |
| `crates/extension_host/src/wasm_host.rs` | +1 | 1: `mod fyx_ui` | W2 |
| `crates/paths/src/paths.rs` | +1/−1 | 18: `APP_NAME = "Fyx"` | W1 |
| `crates/project/src/agent_registry_store.rs` | +2/−6 | 149: no eager registry refresh | W1 |
| `crates/project/tests/integration/agent_registry_store.rs` | +26 | 11: new no-request test; 45, 126, 178: explicit `refresh` | W1, W0 |
| `crates/release_channel/src/lib.rs` | +15/−12 | 46: mutex and pipe names; 208: display name; 231: app id | W1 |
| `crates/windows_resources/src/windows_resources.rs` | +6/−5 | 46: product and company name | W1 |
| `crates/workspace/src/dock.rs` | +7 | 550: `panel_index_for_id` | W3 |
| `crates/workspace/src/workspace.rs` | +3/−2 | 13292, 13308: title test expects Fyx | W0 |
| `crates/zed/Cargo.toml` | +4/−2 | 9, 58: binary `fyx`; 220: `ui_ext_host` | W1, W3 |
| `crates/zed/resources/windows/zed.iss` | +5/−5 | 1258: `fyx://` scheme (see finding 5) | W1 |
| `crates/zed/src/main.rs` | +29/−4 | 97, 179: product name; 214: `fyx://` arguments; 498: app identity; 505, 1822: Fyx default settings; 511: User-Agent; 756: `ui_ext_host::init` | W1, W3 |
| `crates/zed/src/zed.rs` | +5/−3 | 1283, 1293: scheme toasts; 1721: About title; 6208, 6238: action namespaces in the test | W1, W0 |
| `crates/zed/src/zed/app_menus.rs` | +4/−4 | 71: menu names | W1 |
| `crates/zed/src/zed/open_listener.rs` | +9 | 125-131: `fyx_url_to_internal`; 166: accept `fyx://` | W1 |
| **Total** | **+150/−46 in 21 files**, plus the new `fyx_review.rs` (+75) | | |

Outside that measurement:
- `Cargo.lock`: +93, generated.
- `assets/settings/fyx.json`: a new file of Fyx defaults.
- Fyx-owned modules inside upstream crates: `crates/extension_host/src/wasm_host/fyx_ui*` and
  `crates/acp_thread/src/fyx_review.rs`.
- Of the 150 added lines, 29 only update test expectations (`agent_registry_store.rs`,
  `workspace.rs`).
