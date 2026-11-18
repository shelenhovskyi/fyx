# Review: test quality and coverage (tests-1)

Base: `origin/fyx/main` at `571c682`. The two unmerged crates come from their branches:
`claude/fyx-registry-core` at `a5007f7` (`fyx_registry`) and `claude/fyx-w9-fixes` at `0b3ac8b`
(`fyx_guard`, which contains `claude/fyx-w9-guard`).

**Method.**
- **Scratch tree.** I merged all three into a throwaway worktree. The only conflicts were in
  `Cargo.toml`, `Cargo.lock`, `assets/settings/fyx.json` and `settings_content.rs`, all
  list-merge conflicts.
- **Coverage run.** On Linux (4 cores) I ran:
  `cargo llvm-cov nextest --branch -p ui_ext -p ui_ext_sdk -p ui_ext_wasm -p ui_ext_host -p agent_review -p fyx_ext -p fyx_registry -p fyx_guard`.
  That is cargo-llvm-cov 0.9.1 with branch coverage on the pinned 1.98.1 toolchain, using
  `RUSTC_BOOTSTRAP=1`.
- **Mutations.** I applied 9 mutations to product code in the scratch tree, one at a time, and
  re-ran the affected crates' tests.
- **Reading.** I read `fyx_registry` and `fyx_guard` with a sub-review and spot-checked its claims.
- **Nothing pushed but this file.** No product code changed on any pushed branch; the scratch
  tree is not pushed.

All 264 tests pass once the fixture build is fixed (finding T1). Every wasm and timing test
passed under coverage instrumentation on this machine.

## Summary

- **Strong.** `ui_ext`'s tree model (a 1024-case reference-model proptest plus 512-case property
  tests; 98.7% of lines). `fyx_registry`'s signing and update planning (95% of lines; 14
  proptests). `ui_ext_wasm`'s sandbox checks also look strong, but they assert far past their
  limits (mutations m1 and m3 survived).
- **Weak.**
  - **The seams.** Nothing runs:
    - the ACP thread event wiring in `agent_review`;
    - the deleted-file and moved-file branches of the write gate;
    - `UiExtension::load`'s version gate;
    - the registry's workspace integration;
    - the install consent prompt;
    - two of `fyx_guard`'s six hooks;
    - `fyx-ext` end to end.
  - **Four of the six demos.** They are never built for `wasm32-wasip2` or loaded by any test.
- **Mutations.** 6 of the 9 survive (table below). Each survivor is a one-line change that
  removes or weakens a check users rely on.
- **Review rules (verified, for the security reviewer).** `ReviewRules::evaluate` does not
  normalize paths. With trusted rules `auto_accept = ["docs/**"]`, `deny = ["secrets/**"]`, the
  path `docs/../secrets/key.pem` is auto-accepted, and `SECRETS/key.pem` matches no rule. Test 5
  pins this.
- **Gate.** The gate runs Windows only. It never builds the demos for wasm, never runs doc
  examples, and never runs `fyx-ext` or the dev build end to end. Some tests silently skip
  themselves when a developer's environment sets `FYX_AGENT_REVIEW`.

## 1. Coverage

**Product code only.**
- Lines and branches counted before a file's `#[cfg(test)] mod tests`.
- `tests.rs`, `*_tests.rs`, `tests/` and `test_support.rs` excluded.
- Branch pairs de-duplicated across monomorphizations.
- llvm-cov's own per-file numbers, which count the inline test modules, are 3–10 points higher
  for `agent_review` and `fyx_registry`.

| Crate | Lines hit | Lines | Branches hit | Branches |
|---|---|---|---|---|
| `ui_ext` | 233/236 | 98.7% | 25/28 | 89.3% |
| `ui_ext_wasm` | 583/612 | 95.3% | 48/52 | 92.3% |
| `fyx_registry` (unmerged) | 1259/1320 | 95.4% | 185/214 | 86.4% |
| `fyx_guard` (unmerged) | 527/592 | 89.0% | 82/104 | 78.8% |
| `acp_thread::fyx_review` (hook) | 56/68 | 82.4% | 9/14 | 64.3% |
| `agent_review` | 1924/2390 | 80.5% | 213/356 | 59.8% |
| `ui_ext_sdk` | 621/774 | 80.2% | 31/34 | 91.2% |
| `ui_ext_host` | 2210/2891 | 76.4% | 250/366 | 68.3% |
| `fyx_ext` | 468/683 | 68.5% | 86/104 | 82.7% |
| **total** | 7881/9566 | **82.4%** | 929/1272 | **73.0%** |

Per file, the outliers are:
- `ui_ext_host`: `registry.rs` 48%, `install.rs` 37%, `panel.rs` 62%, `ui_ext_host.rs` 0% (init).
- `agent_review`: `script_driver.rs` 30%.
- `fyx_ext`: `main.rs` 0%, `build.rs` 55%.
- `ui_ext_sdk`: `host_model.rs` 56%.

Reproducing the numbers takes two workarounds:
- **Fixture build.** Patch the scratch `test_support.rs` as in finding T1.
- **Report generation.** `cargo llvm-cov report` (text summary or JSON) segfaults inside
  `llvm-cov` on the binaries that link gpui or wasmtime. Run `llvm-cov report` / `llvm-cov show`
  directly and pass the Fyx source files explicitly.

### Most important uncovered code

The lines that matter most are error paths and security checks; logging and `Display` are left
out.

- **`agent_review/src/agent_review.rs:972-998`: thread event wiring.** `on_thread_event` for
  `ToolAuthorizationRequested` never runs.
  - The tests call `open_permission_preview` and `review_write` directly. So the real path from an
    ACP permission request to a review is untested, and so is the "one tool call edits N files"
    branch, where the edit is not reviewed.
  - **Security consequence:** `deny` rules are only evaluated inside a preview. A multi-file or
    otherwise unreviewable edit falls through to Zed's own prompt, and no test shows whether a
    `deny` rule still applies there.
- **`agent_review.rs:1177-1228`: unreviewable v2 requests.** Every "unreviewable" outcome of
  `v2_edit_request` is untested:
  - more than one change;
  - a `Delete` or `Move` operation;
  - no patch text;
  - no allow-once or reject-once option.
- **`acp_thread/src/fyx_review.rs:84-97`: file deleted or moved during review.** These two checks
  keep a stale write from landing, and neither branch runs (mutation m5). Lines 73-76 run: the
  edit is rejected and its feedback goes back to the agent.
- **`ui_ext_wasm/src/ui_ext_wasm.rs:556-570`: `UiExtension::load`'s gates.**
  - "Not a fyx:ui component" (no version section and no imports) never runs.
  - "Targets an unsupported fyx:ui version" never runs either. Only `UiManifest::read` gates the
    version in tests, so `load` with raw bytes is unchecked (mutation m8).
- **`ui_ext_wasm.rs:816-823` and `787-791`: guest replies.** `reply` is where a well-formed call
  with an invalid result becomes a `ReplyError` carrying its pushes. That arm never runs against
  a real component: no test has a real guest return an invalid tree, patch or row. Only
  `ui_ext_host`'s `FakeBackend` produces such replies.
- **`ui_ext_wasm.rs:157`: a version section of the wrong size.** The `% 6 == 0` false arm never
  runs: a section longer than 6 bytes that is not a multiple of 6.
- **`ui_ext_host/src/install.rs:68-69`: dev install over a real install.** The refusal to replace
  a regular installed extension never runs (mutation m9).
  - `prompt_and_install` (lines 95-168) is entirely untested. That is the consent dialog that
    lists capabilities, plus its Cancel path.
- **`ui_ext_host/src/registry.rs`: workspace integration.** Untested:
  - `init`, `attach_to_workspace`, `add_missing_surfaces` and `toggle_panel_focus`, i.e. how
    panels and status items reach a real workspace;
  - `watch_installed_extensions`;
  - the `FYX_UI_DEV_EXTENSIONS` loader;
  - batching of pushes outside a call (`register_extension`, lines 273-284);
  - "no longer has the surface" on a reinstall (lines 220-226);
  - scan overlap, where `rescan_requested` is set (line 295).
- **`ui_ext_host/src/surface.rs:187-203`: failed row fetch.** A row fetch that fails with a
  `ReplyError` that has pushes never runs. Branch 193 is never evaluated.
- **`ui_ext_sdk/src/host_model.rs`: unused SDK conversions.** The `From` conversions for colour
  tokens, list, table, icon, diff-view props and `ListRow` never run. So the SDK's "check with the
  host's rules" path is never exercised for those prop kinds.
- **`fyx_guard`: hook entry points.**
  - `check_agent_edit` is hook H3, the built-in agent's edit and write tools.
  - `scrub_env` covers both terminal hooks; only the inner `scrub` is tested.
  - The worktree `private_files` branch (line 145) never runs.
- **`fyx_registry`: rejecting paths with no test.** Coverage is high, but these have none:
  - `Index::add` checking the existing index (mutation below);
  - backslash and empty-component paths;
  - `https:///x`;
  - control characters in descriptions or reasons;
  - wrong-length base64;
  - domain separation between release and block-list signatures.
- **`fyx_ext`: the CLI and build path.** `main`/`run` (the CLI), `build`, `dev`, `find_fyx` and
  `cargo` never run. `check` runs only against committed dummy components, never a real one.

## 2. Test quality

### Mutations

These were applied in the scratch tree. Each mutated build ran the tests named in the third
column.

| # | Mutation (one line) | Tests run | Result |
|---|---|---|---|
| m1 | Delete `deny_network(&mut wasi);` (`ui_ext_wasm.rs:591`) | `test_guest_cannot_open_a_socket` | **survives**: wasmtime 48's `WasiCtxBuilder` already denies TCP, UDP, name lookup and every address, so the test pins wasmtime's defaults, not Fyx's code |
| m2 | Swap `gap` and `padding` in `From<wit::Props>` (`ui_ext_wasm.rs:897`) | all 61 of `ui_ext_wasm` + `ui_ext_host` | **survives**: no test checks that a guest's props arrive field for field; the gallery test only checks for non-empty boxes |
| m3 | Push cap `>= MAX_PUSHES_PER_CALL` → `>= MAX_PUSHES_PER_CALL * 4` | `ui_ext_wasm` (13) | **survives**: the probe floods forever and the assertion formats the constant into its message |
| m4 | `review_rules.rs:163`: auto-accept ignores trust | `agent_review` (42) | caught by 2 tests |
| m5 | `fyx_review.rs:85`: skip the "deleted during review" check | `agent_review` (42) | **survives** (acp_thread's only Fyx test checks `feedback_error`) |
| m6 | `tree.rs:255`: node cap `>=` → `>` | `ui_ext` (38) | caught by 3 tests, including 2 proptests |
| m7 | `fyx_review.rs:100`: skip the "changed during review" check | `agent_review` (42) | caught by `a_file_changed_during_review_is_not_written` |
| m8 | `ui_ext_wasm.rs:566`: `load` accepts any API version | `ui_ext_wasm` + `ui_ext_host` (61) | **survives** |
| m9 | `install.rs:68`: dev install replaces a real install | `ui_ext_host` (48) | **survives** |

The sub-review of the unmerged crates reasoned through three more mutations; it did not run
them:
- **Survives:** `fyx_registry` `Index::add` without `verify_strict` of the existing index
  (`index.rs:359`).
- **Survives:** `fyx_guard` deleting the `terminal.rs:967` scrub hook, or the `edit_session.rs:725`
  edit hook.
- **Caught:** `fyx_registry` block-list replay `>` → `!=`, and update planning `break` →
  `continue`.

### Tests that assert little or skip themselves

- **`agent_review` `script_driver_is_only_in_dev_builds`** (`agent_review.rs:2707`) is a
  tautology.
  - It asserts `SCRIPT_DRIVER_COMPILED == cfg!(debug_assertions) || cfg!(feature = "review-script")`.
    That is the constant's own definition (`agent_review.rs:169`).
  - Test builds always have `debug_assertions`, so the `#[cfg(not(...))]` arm never compiles.
  - What should be tested is that a release build without the feature does not read
    `FYX_AGENT_REVIEW_SCRIPT`. That needs a release-profile check in the gate (G9).
- **`review_mode_follows_the_setting`, `only_user_settings_turn_review_off`,
  `changing_the_setting_keeps_held_writes`** (`agent_review.rs:2580-2690`) `return` early and
  pass when `FYX_AGENT_REVIEW` is set in the environment. A developer who uses the documented
  override silently loses the only tests of the setting.
- **`test_guest_cannot_open_a_socket`** proves only that wasmtime's defaults deny (m1). It also
  tries only TCP to `127.0.0.1:9`; UDP and name lookup are never attempted.
- **The sandbox cap tests** assert far past their limits: 64 MiB against 32 MiB, 8000 items
  against 4096, an endless push loop against 64. None of them pins the boundary (m3).
- **`test_gallery_renders_every_section`** checks that every leaf has a non-empty box. That
  catches missing nodes but not wrong props (m2).
- **`fyx_registry`:**
  - The `flipped_json_byte_is_refused` proptests assert only inside `if let Ok(..)`; most flips
    break parsing, so the property is rarely checked.
  - `validation_refuses_bad_fields` asserts `is_err()` without checking which error fired.
- **`fyx_guard`:** the Off-mode tests and `audit_only_logs_path_tools` assert only that operations
  succeed, so treating audit as off passes everything.
- **`fyx_ext` `creates_a_crate_whose_manifest_passes`** checks the generated files as text. The
  template's Rust source, including its own `#[test] click_patches_the_label`, is never compiled.
- **Negative assertions after a fixed sleep.**
  - The "nothing changed: a rescan keeps the instance" step in
    `test_installed_extensions_load_reload_and_stop` asserts after `sleep(200ms)`.
  - The "unrelated file: no reload" step in `test_dev_extension_hot_reloads` asserts after
    20 × (20 ms real + 50 ms test clock).
  - A regression that reloads after 300 ms passes both.

### Flaky patterns

- **Real-clock bounds** (`ui_ext_wasm/src/tests.rs:444-493`).
  - `test_runaway_call_...` requires `200ms <= elapsed < 2s`.
  - `test_call_blocked_in_host_...` requires `200ms <= elapsed < 600ms`.
  - Both use a real epoch thread and a 50 ms slack, and passed here under coverage.
  - The 600 ms upper bound is the first thing to fail on a loaded 2-core `windows-latest` runner
    while 10 other test processes compile fixtures.
- **Polling with `std::thread::sleep`.**
  - `ui_ext_host/src/wasm_tests.rs` `run_until` polls 500 × 10 ms with a 5 s cap.
  - The hot reload loop runs 250 × 20 ms.
  - Calls run on tokio threads outside GPUI's scheduler, so these tests are not deterministic and
    cannot use `advance_clock` alone.
- **mtime fingerprints** (`registry.rs:518-527`). Reload detection keys on `(mtime, len)`. The
  reinstall test rewrites the *same bytes* after `sleep(50ms)`.
  - On a filesystem with coarse mtimes the change is invisible and the test times out at 5 s:
    FAT/exFAT (2 s), HFS+ (1 s), and some overlay or network mounts.
  - The product has the same blind spot for a quick rebuild that produces a same-size component.
- **Shared global state.**
  - **Process-global wasm runtime.** `ui_ext_wasm` keeps the engine, linker and epoch thread in a
    `OnceLock` (`RUNTIME`), plus `DEADLINE_WAKERS` and the moka `CACHE_STORE`, all for the whole
    process.
  - **Per-test setup.** Every wasm test must remember `cx.executor().allow_parking()` and
    `cx.update(gpui_tokio::init)`. A test that forgets `gpui_tokio::init` panics in
    `Tokio::spawn`'s `read_global`. Each call builds its own 2-thread tokio runtime, which is
    shut down with `shutdown_background` when the test's `App` drops.
  - **nextest vs `cargo test`.** nextest gives each test its own process, so tests share nothing.
    Plain `cargo test -p ui_ext_wasm` runs them as threads of one process. Then they share the
    engine and epoch thread, and the `SPIN` test's busy guest competes with the timing assertions
    of the `SLEEP` test. That is a likely source of "fails only with cargo test" reports; I did
    not run `cargo test` to confirm.
  - **Environment variables.** `FYX_AGENT_REVIEW`, `FYX_UI_DEV_EXTENSIONS`,
    `FYX_UI_DEV_DECORATIONS` and `FYX_AGENT_REVIEW_SCRIPT` are read from the real process
    environment, and `fyx_guard` reads `paths::config_dir()` derived from `HOME`. No test sets or
    clears them.

### Fixtures that drift from the real host

- **T1 (verified): the fixture builder inherits the parent build's environment.**
  `ui_ext_wasm::test_support::build_uncached` runs a nested `cargo build --target wasm32-wasip2`
  and inherits everything.
  - Under `cargo llvm-cov` the nested build picks up its `RUSTC_WRAPPER` and coverage flags.
  - It then fails to build `serde_core`'s build script ("the option `Z` is only accepted on the
    nightly compiler").
  - So 18 of the 61 wasm tests fail under coverage for a reason unrelated to the code.
  - The same applies to `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` sanitizer runs.
  - Fix (test support only): remove `RUSTC_WRAPPER`/`RUSTC_WORKSPACE_WRAPPER` (or set them to
    sccache explicitly), `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_LLVM_COV*` and
    `LLVM_PROFILE_FILE` from the child command.
- **`FakeBackend`** (`ui_ext_host/src/tests.rs`).
  - It hand-implements `UiBackend` and answers synchronously.
  - It does not validate replies or serialize calls the way `UiExtension` does, and does not
    disable itself after a trap (it simulates that with `FATAL`).
  - The renderer's 30 most detailed tests run only against it. No contract test runs the same
    script against both backends.
- **`fyx_ext`'s committed dummy components** (`crates/fyx_ext/fixtures/dummy-v0.{1,2}.wasm`) are
  generated by hand with `wasm-tools`. Nothing checks them against
  `crates/ui_ext/wit/fyx-ui.wit`, so a WIT change leaves `fyx-ext check` tested against the old
  world.
- **`fyx_ext::manifest` re-implements the host's manifest rules**, and its doc comment still
  points at the pre-move `extension_host::wasm_host::fyx_ui::UiManifest`.
  - Duplicated rules: `is_relative_inside`, `decode_api_version`, the id rules.
  - The id rules are already different: the CLI rejects `a_b` and `9lives`, which the host
    accepts.
  - The CLI enforces a 32 MiB component cap the host does not.
  - The CLI cannot check "surface needs `ui:panel`", which the host enforces at load. So
    `fyx-ext check` passes extensions that the host rejects.
  - No test feeds one corpus to both.
- **Demo manifests.** `extensions/fyx_demo_{todo,markdown,gitstatus,form}/extension.toml` and
  components never meet `UiManifest::read`, `load_from_dir` or `check_surfaces` in any test. Only
  the counter is loaded through its source; the gallery is registered as a backend.

## 3. Gate gaps

What `script/gate.sh` and `.github/workflows/fyx_gate.yml` do not run:

1. **Linux and macOS.** CI is `windows-latest` only, and no Fyx test is `cfg`-gated. So every
   OS-specific behaviour is tested on exactly one OS in CI, and on Linux only on developer boxes:
   - component paths with `..\` are rejected on Windows only (`Path::components`);
   - `review_rules` replaces `\` with `/`;
   - symlinked dev installs;
   - the file watcher backend;
   - mtime resolution;
   - `fyx_guard`'s case-insensitive and Windows-alias matching.
2. **Doc tests and doc examples.**
   - Every Fyx crate sets `doctest = false`, and nextest does not run doctests anyway.
   - The SDK's crate-level example is ` ```ignore `.
   - The ` ```rust ` blocks in `docs/fyx/sdk-reference.md` (11) and
     `docs/fyx/extension-authoring.md` (8) are never compiled.
3. **Demos for `wasm32-wasip2`.** `cargo check/clippy -p fyx_demo_*` builds them for the host
   only.
   - Only `fyx_demo_counter` and `fyx_demo_gallery` are built for wasm, and only as a side effect
     of `ui_ext_host` tests.
   - `todo`, `markdown`, `gitstatus` and `form` can break their wasm build without the gate
     noticing. `script/fyx-build-demos` is not run.
4. **The fixture workspace** (`crates/ui_ext_wasm/fixtures`) is not a workspace member and is
   never `clippy`'d. It is built only inside tests.
5. **`fyx-ext` end to end.** `new → build → check → package` never runs, and neither does loading
   the packaged `.fyxext`. The template is never compiled.
6. **Proptest depth.**
   - Case counts are fixed in source: 1024 and 512 in `ui_ext`, and the default 256 in
     `fyx_registry`.
   - No deeper scheduled run (`PROPTEST_CASES`).
   - No committed `*.proptest-regressions` files, so a shrunk failure found on a laptop is not
     replayed by CI.
7. **The dev build end to end.**
   - `FULL=1` (the `zed` binary) is not run by CI.
   - Agent review e2e exists only as the `FYX_AGENT_REVIEW_SCRIPT` driver plus manual steps in
     `docs/fyx/manual-tests.md`; nothing launches it.
   - No test loads a UI extension into a real `Workspace`; see `registry.rs` above.
8. **Reverse dependents.** Only crates whose own files changed are tested. Since the base is the
   upstream tag, that is every Fyx crate and every hooked upstream crate. Crates that depend on a
   hooked crate without being hooked themselves are not tested:
   - `agent_servers` (depends on `acp_thread`);
   - consumers of `settings_content`.
9. **Release-profile behaviour.** The "script driver only in dev builds" property can only be
   observed in a release build, and the gate builds none.
10. **Environment hygiene.** The gate does not clear `FYX_AGENT_REVIEW`, `FYX_UI_DEV_EXTENSIONS`,
    `FYX_UI_DEV_DECORATIONS` or `FYX_AGENT_REVIEW_SCRIPT` before running tests.
11. **No coverage job**, so the numbers above are not tracked.
12. **Output truncation.** `| tail -20/-30` can cut the first compiler error when there are
    several. `pipefail` keeps the exit status correct, but the log may not show the cause.

## 4. The 15 most valuable tests to add

Ranked by risk removed per hour.
- The sketches use the crates' real APIs. Where a fixture needs a new behaviour, the sketch says
  so.
- New probe events go in `crates/ui_ext_wasm/fixtures/probe/src/probe.rs`, next to `PUSH_FLOOD`
  etc., with their ids in `tests.rs`'s `mod probe`.

### 1. The ACP permission event opens a review, and `deny` rules survive unreviewable edits

**Proves:**
- The real `AcpThreadEvent::ToolAuthorizationRequested` path reaches `open_permission_preview`.
- An edit that touches two files is reported as unreviewable and left to Zed's prompt.
- A `deny` rule still refuses such an edit.

The last point may currently fail. If it does, this is a fail-open path and a product fix, not
just a test.

**Where:** `crates/agent_review/src/agent_review.rs` tests, next to
`deny_rule_rejects_with_a_reason_even_with_review_off`.

```rust
let (thread, connection) = stub_thread(&project, cx).await;        // StubAgentConnection
set_test_rules(&review, "/p", r#"deny = ["*.env"]"#, cx);
let request = connection.request_permission_with_diffs(            // stub helper to add
    &thread, [("/p/a.txt", OLD, NEW), ("/p/.env", "", "S=1")], cx);
cx.run_until_parked();
assert_eq!(review.read_with(cx, |r, _| r.open_review_count()), 0);
assert!(workspace_toasts(cx).iter().any(|t| t.contains("edits 2 files")));
assert_eq!(request.await.verdict(), Deny, "a deny rule is not bypassed by batching");
```

### 2. Invalid guest replies become `ReplyError`s that keep their pushes, and the extension stays enabled

**Proves:** the host validates untrusted guest output from a *real* component, not just from
`FakeBackend`. It covers `ui_ext_wasm.rs:816-823, 787-791`.

**Where:** `crates/ui_ext_wasm/src/tests.rs`. Add probe events `BAD_PATCH` (push one valid update,
then return `SetProps(999, ..)`) and `BAD_ROW` (`list-rows` returns an index outside the request).

```rust
let (extension, _) = load("fyx_ui_test_probe", probe_wasm(), cx).await;
let error = extension.handle_event(probe::SURFACE, UiEvent::Click(probe::BAD_PATCH)).await.unwrap_err();
let reply = error.downcast_ref::<ReplyError>().expect("a rejection, not a trap");
assert_eq!(reply.pushed.len(), 1, "pushes made before the bad reply are kept");
let error = extension.list_rows(probe::SURFACE, probe::LIST, 0, 4).await.unwrap_err();
assert!(format!("{error:#}").contains("outside 0..4"), "{error:#}");
extension.handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH)).await.expect("not disabled");
```

### 3. A file deleted or moved during review is not written

**Proves:** the two uncovered stale-write checks in `crates/acp_thread/src/fyx_review.rs:84-97`
(mutation m5).

**Where:** `crates/agent_review/src/agent_review.rs` tests, as a sibling of
`a_file_changed_during_review_is_not_written`, which already has the whole setup.

```rust
let write = thread.update(cx, |t, cx| t.write_text_file(path.clone(), NEW.into(), cx));
cx.run_until_parked();
fs.remove_file(&path, Default::default()).await.unwrap();   // or fs.rename(&path, &moved, ..)
cx.run_until_parked();
accept_held(cx);
let error = write.await.expect_err("must not land");
assert!(format!("{error:#}").contains("was deleted while your edit was under review"));
assert!(fs.load(&path).await.is_err(), "nothing recreated the file");
```

### 4. `UiExtension::load` refuses unsupported and unversioned components

**Proves:** the load-time version gate, independently of the manifest (mutation m8), plus the
"not a fyx:ui component" error and the odd-length section branch (`ui_ext_wasm.rs:157, 556-570`).

**Where:** `crates/ui_ext_wasm/src/tests.rs`. Patch the probe in place, the way
`test_version_falls_back_to_imports` already does.

```rust
let mut wasm = probe_wasm();
let at = find(&wasm, API_VERSION_SECTION.as_bytes()) + API_VERSION_SECTION.len();
wasm[at..at + 6].copy_from_slice(&[0, 0, 0, 2, 0, 0]);       // claims fyx:ui 0.2.0
let error = cx.update(|cx| UiExtension::load("probe".into(), wasm, &cx.to_async())).await.err().unwrap();
assert!(format!("{error:#}").contains("targets fyx:ui 0.2.0"), "{error:#}");
let seven = module_with_version_section(&[0; 7]);
assert!(format!("{:#}", parse_api_version("x", &seven).unwrap_err()).contains("invalid"));
```

### 5. Review rules: non-normalized and case-variant paths cannot escape `deny`

**Proves:** `ReviewRules::evaluate` is a pure function on paths that come from the agent. These
spellings of a denied path must still match `deny`:
- `docs/../secrets/key.pem`, which today matches `docs/**` → auto-accept;
- `./secrets/x`;
- `SECRETS/x` and `x.ENV` on case-insensitive filesystems.

The property test checks that inserting `./` or `a/..` never weakens the outcome, and that an
untrusted worktree never yields `AutoAccept`.

**Where:** the `crates/agent_review/src/review_rules.rs` tests, with `proptest` as a dev-dependency
(integrator).

```rust
proptest! {
    #[test]
    fn spelling_never_weakens_a_rule(dir in "[a-z]{1,6}", file in "[a-z]{1,6}", detour in "[a-z]{1,4}") {
        let rules = rules(true);
        let plain = kind(&rules, &format!("secrets/{dir}/{file}"));
        let detoured = kind(&rules, &format!("docs/{detour}/../../secrets/{dir}/{file}"));
        prop_assert_eq!(plain, Some(RuleKind::Deny));
        prop_assert_eq!(detoured, plain);   // today: Some(AutoAccept)
        prop_assert_ne!(kind(&rules(false), &format!("docs/{file}.md")), Some(RuleKind::AutoAccept));
    }
}
```

**Verified in the scratch tree** with this module's own `RULES` and a trusted worktree:
- `docs/../secrets/key.pem` evaluates to `Some(AutoAccept)`;
- `SECRETS/key.pem` and `config/prod.ENV` evaluate to `None`;
- `./secrets/key.pem` evaluates to `Some(Deny)`.

So the `detoured` assertion fails today. Whether every caller normalizes agent-supplied paths
before `rule_for` is for the security reviewer. This test makes the assumption explicit either
way.

### 6. `fyx_guard` hooks H3 and the terminal env scrub

**Proves:**
- The built-in agent's write and edit tools refuse agent config paths in Shield mode.
- `scrub_env` blanks secrets through the real entry point.
- Deleting `edit_session.rs:725` or `terminal.rs:967` fails a test.

**Where:** `crates/fyx_guard/tests/agent_tools.rs` and the `fyx_guard.rs` tests.

```rust
let tools = tools(FyxSecurityMode::Shield, cx).await;
let input = WriteFileToolInput { path: "root/.claude/settings.json".into(), content: "{}".into() };
assert_blocked(tools.run(WriteFileTool::new(/* project, thread, action_log, registry */), input, cx).await, ".claude/");
assert!(!tools.exists(path!("/root/.claude/settings.json")).await);
let mut env = HashMap::from_iter([("GITHUB_TOKEN".into(), "x".into())]);
cx.update(|cx| scrub_env(&mut env, cx));
assert_eq!(env["GITHUB_TOKEN"], "");
```

### 7. `fyx-ext` end to end: new → build → check → package → host load

**Proves:**
- The template compiles for `wasm32-wasip2`, its embedded test passes, and `check` is clean on a
  real component.
- The packaged `.fyxext` unpacks into something `UiExtension::load_from_dir` accepts.

This is the only test of `build.rs` and `main.rs` (0–55% covered).

**Where:** `crates/fyx_ext/tests/end_to_end.rs`.
- It is slow (one wasm build), so mark it `#[ignore]` and have the gate run it with
  `--run-ignored` (G4).
- It needs a `ui_ext_wasm` dev-dependency (integrator: `Cargo.toml`).

```rust
let dir = tempfile::tempdir()?;
let crate_dir = fyx_ext::template::create(&NewExtension { name: "e2e-panel".into(),
    parent: dir.path().into(), sdk: SdkSource::Checkout(default_fyx_checkout()) })?;
let manifest = fyx_ext::build::build(&crate_dir, &BuildOptions { release: false, ..Default::default() })?;
let (_, report) = fyx_ext::check(&crate_dir)?;
assert!(!report.has_errors(), "{report:?}");
let package = fyx_ext::package::package(&crate_dir, dir.path())?;
assert!(load_unzipped_with_host(&package, cx).await?.is_some()); // UiExtension::load_from_dir
```

### 8. `fyx-ext check` and the host agree on one manifest corpus

**Proves:** the CLI's re-implemented rules do not drift from `UiManifest::read`/`check_surfaces`.
Every manifest the host rejects gets an error from `check`, and the documented, intentional
differences (stricter registry ids) are listed explicitly.

**Where:** `crates/fyx_ext/tests/host_agreement.rs`, with a `ui_ext_wasm` dev-dependency.

```rust
const CASES: &[(&str, Expect)] = &[
    (r#"component = "../x.wasm""#, Both), (r#"component = "/abs.wasm""#, Both),
    (r#"api_version = "0.2.0""#, Both), (r#"extra = 1"#, Both),
    (r#"capabilities = []"#, Both /* host: surface needs ui:panel — CLI must flag it */),
    (r#"id = "a_b""#, CliOnly),
];
for (fragment, expect) in CASES {
    let (host, cli) = (host_read(fragment, cx).await.is_err(), cli_check(fragment).has_errors());
    assert_eq!((host, cli), expect.pair(), "{fragment}");
}
```

### 9. Sandbox caps hold at their exact boundaries

**Proves:**
- Exactly `MAX_PUSHES_PER_CALL` pushes and exactly `MAX_PUSHED_ITEMS_PER_CALL` items succeed, and
  one more traps.
- Growing memory to just under `UI_MEMORY_LIMIT` works.

That kills m3 and pins the limits users and extension authors can rely on.

**Where:** `crates/ui_ext_wasm/src/tests.rs`. Add a probe event `PUSH_N`, carried as
`InputSubmitted(PUSH_N, "<count>:<items each>")`.

```rust
let push = |n: usize, items: usize| extension.handle_event(probe::SURFACE,
    UiEvent::InputSubmitted(probe::PUSH_N, format!("{n}:{items}")));
let reply = push(MAX_PUSHES_PER_CALL, 1).await.expect("exactly the cap");
assert_eq!(reply.pushed.len(), MAX_PUSHES_PER_CALL);
assert!(format!("{:#}", push(MAX_PUSHES_PER_CALL + 1, 1).await.unwrap_err()).contains("more than 64"));
// fresh instance, since a trap disables it:
let (extension, _) = load("fyx_ui_test_probe", probe_wasm(), cx).await;
push(1, MAX_PUSHED_ITEMS_PER_CALL).await.expect("exactly the item cap");
```

### 10. Guest props arrive field for field

**Proves:** both hand-written conversions (`From<wit::Props>` in `ui_ext_wasm` and `host_model.rs`
in the SDK) map every field to the field of the same name (kills m2). It covers the six
`host_model` conversions that never run today.

**Where:**
- `crates/ui_ext_wasm/src/tests.rs`: build `wit::Props` values with a distinct value per field.
- `crates/ui_ext_sdk/src/ui_ext_sdk.rs` tests: the same check for the guest side.

```rust
let wit = wit::Props::Flex(wit::FlexProps { direction: wit::FlexDirection::Column, gap: 1,
    padding: 2, grow: true, align: wit::Align::End, background: Some(wit::ColorToken::Warning) });
assert_eq!(ui_ext::Props::from(wit), ui_ext::Props::Flex(ui_ext::FlexProps {
    direction: FlexDirection::Column, gap: 1, padding: 2, grow: true,
    align: Align::End, background: Some(ColorToken::Warning) }));
// …one assertion per variant: Text, Button, Input, List(row_count 3, row_height 4), Table, Icon, DiffView
```

### 11. Dev install never replaces a real install, and Cancel installs nothing

**Proves:**
- The guard at `install.rs:68` (m9).
- The consent prompt in `prompt_and_install`: it shows the capability list, and Cancel leaves the
  installed directory untouched.

**Where:** `crates/ui_ext_host/src/wasm_tests.rs`, next to `test_install_dev_ui_extension`.

```rust
std::fs::create_dir_all(installed.join("fyx-test-status-item"))?;          // a regular install
let error = install_dev_ui_extension(&source, &installed, fs.as_ref()).await.unwrap_err();
assert!(format!("{error:#}").contains("not as a dev extension"));
assert!(!std::fs::symlink_metadata(installed.join("fyx-test-status-item"))?.is_symlink());
workspace.update_in(cx, |w, window, cx| prompt_and_install(w, window, cx));
cx.simulate_new_path_selection(|_| Some(source.clone()));
assert!(cx.has_pending_prompt()); cx.simulate_prompt_answer("Cancel");
assert!(std::fs::read_dir(&installed)?.count() == 1, "nothing linked");
```

### 12. Registry and signing rejection paths in `fyx_registry`

**Proves:**
- `Index::add` refuses to extend an index whose existing entries do not verify.
- Release and block-list signatures are domain-separated.
- The untested validation rules reject: backslash and empty path components, `https:///x`,
  control characters, over-long owners, wrong-length base64.

**Where:** the `crates/fyx_registry/src/index.rs` and `blocklist.rs` tests.

```rust
let mut bad = signed(release("todo", "1.0.0")); bad.signed.urgent = true; // signature now stale
index.releases.push(bad);
assert!(matches!(index.add(signed(release("todo", "1.1.0")), &keyset(), Timestamp(1)),
    Err(Error::BadSignature { .. })));
let tag = |d| CanonicalWriter::new(d).finish();
assert!(blocklist(1, vec![]).canonical_bytes().starts_with(&tag("fyx-registry/blocklist/v1")));
for path in ["a\\..\\b", "a//b"] { assert!(with_path(path).validate().is_err(), "{path}"); }
```

### 13. Every demo builds for wasm, loads through its own manifest and mounts every surface

**Proves:**
- The four demos never built for wasm still compile.
- Their `extension.toml` passes `UiManifest::read` and `check_surfaces`.
- `mount` returns a valid tree for each declared surface.

This closes gate gap 3 without a new CI step.

**Where:** `crates/ui_ext_host/src/wasm_tests.rs`.

```rust
for demo in ["fyx_demo_counter", "fyx_demo_todo", "fyx_demo_gallery",
             "fyx_demo_markdown", "fyx_demo_gitstatus", "fyx_demo_form"] {
    let dir = demo_dir_with_component(demo, &tempdir);   // copy extension.toml + demo_wasm(demo)
    let (extension, _) = load_from_dir(&dir, real_fs.clone(), cx).await
        .unwrap_or_else(|e| panic!("{demo}: {e:#}")).expect("[fyx_ui] table");
    for surface in extension.surfaces() {
        extension.mount(&surface.id).await.unwrap_or_else(|e| panic!("{demo}/{}: {e:#}", surface.id));
    }
}
```

### 14. UI extensions reach a real workspace

**Proves:**
- `init` + `attach_to_workspace` add a loaded panel to its dock and a status item to its side of
  the status bar, ordered by priority.
- `ToggleExtensionPanel` focuses the panel.
- A reinstall that drops a surface fails it with "no longer has the surface".

This covers about 250 lines of `registry.rs` and `panel.rs` that no test reaches.

**Where:** `crates/ui_ext_host/src/tests.rs`, with `workspace::Workspace::test_new` and
`FakeBackend`.

```rust
cx.update(crate::init);
let registry = cx.update(|cx| UiExtensionRegistry::global(cx)).unwrap();
let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::test_new(project, window, cx));
registry.update(cx, |r, cx| r.register_backend(backend.clone(), &[panel("main"), status("s")], cx));
cx.run_until_parked();
assert!(workspace.read_with(cx, |w, cx| w.panel::<ExtensionPanel<0>>(cx).is_some()));
cx.dispatch_action(ToggleExtensionPanel { key: "fake/main".into() });
registry.update(cx, |r, cx| r.register_backend(backend, &[status("s")], cx));
assert!(failed_with(&registry, "fake/main", "no longer has the surface", cx));
```

### 15. Fuzz harnesses for the parsers and the tree

**Proves:**
- Nothing that reads attacker-controlled bytes panics.
- The tree keeps its invariants under arbitrary patch sequences.

The `ui_ext` proptest model already defines the invariants; cargo-fuzz runs them for hours
instead of for 1024 cases.

**Where:** a new `fuzz/` crate outside the workspace, built with nightly cargo-fuzz, run on a
schedule (G6).

Targets:
- `tree_patches`: `Tree::from_nodes` then `apply_update` over arbitrary `Update`s, checking the
  invariants from `tests/properties.rs` via a shared `ui_ext::test_support::check_invariants`.
- `ui_manifest`: `toml::from_str::<ExtensionToml>` followed by the `UiManifest::read` checks, plus
  `fyx_ext::manifest::parse`.
- `api_version`: `ui_ext_wasm::parse_api_version` and `fyx_ext::inspect::inspect` on arbitrary
  bytes.
- `registry_index`: `Index::from_json`, `VerifiedBlocklist::from_json`,
  `BlocklistCache::from_json`, `Keyset::from_json`, `VersionRange::parse`.

```rust
fuzz_target!(|input: (Vec<Node>, Vec<Update>)| {
    let Ok(mut tree) = Tree::from_nodes(input.0, TreeLimits::default()) else { return };
    for update in input.1 {
        let before = tree.clone();
        if tree.apply_update(update).is_err() { assert_eq!(tree, before, "failed updates roll back"); }
        ui_ext::test_support::check_invariants(&tree).unwrap();
    }
});
```

**Also worth doing, below the cut:**
- A `FakeBackend`/`UiExtension` contract test that runs one script against both backends.
- UDP and DNS cases in the network test.
- A proptest that the block-list cache sequence is monotonic over arbitrary fetches (and the
  freeze design gap: nothing compares the signed `generated_at` to now).
- A test that `dummy-v0.1.wasm`'s imports and exports equal the current WIT world.
- A `#[cfg(windows)]` test of `fyx_guard`'s Windows path aliases (`.claude.`, `::$DATA`, `\\?\`).

## 5. Gate changes

In priority order. Steps marked *(CI)* only touch `.github/workflows/fyx_gate.yml`; the others
touch `script/gate.sh`.

- **G1. Fix the fixture builder's environment** (test support only). This is finding T1. Without
  it, coverage and sanitizer runs report 18 false failures.
- **G2. Add a Linux job** *(CI)*. Run `script/gate.sh` on `ubuntu-latest`, next to Windows. Linux
  is cheaper and faster, and it covers the path, symlink, watcher and mtime behaviours that
  currently run on Windows only. Add `macos-latest` on a weekly schedule.
- **G3. Build every demo and the fixture workspace for wasm**:
  ```bash
  cargo clippy --locked --target wasm32-wasip2 -p fyx_demo_counter -p fyx_demo_todo -p fyx_demo_gallery -p fyx_demo_markdown -p fyx_demo_gitstatus -p fyx_demo_form -- --deny warnings
  ```
  ```bash
  cargo clippy --locked --manifest-path crates/ui_ext_wasm/fixtures/Cargo.toml --target wasm32-wasip2 -- --deny warnings
  ```
  Test 13 then also loads each demo.
- **G4. Run ignored end-to-end tests:**
  `cargo nextest run -p fyx_ext --run-ignored ignored-only` (test 7). Also run the SDK's doc
  examples: make the crate example ` ```no_run `, drop `doctest = false` for `ui_ext_sdk`, and run
  `cargo test --doc -p ui_ext_sdk`, because nextest does not run doctests. Compile the
  `docs/fyx/*.md` Rust blocks via a small test that extracts them into the e2e crate.
- **G5. Clear the Fyx override variables before testing:**
  ```bash
  unset FYX_AGENT_REVIEW FYX_AGENT_REVIEW_SCRIPT FYX_UI_DEV_EXTENSIONS FYX_UI_DEV_DECORATIONS
  ```
  Also change the three `review_mode_*` tests to fail, not `return`, when an override is present,
  or better, read the override through a test-settable seam.
- **G6. Scheduled deep runs** *(CI, weekly)*:
  - `PROPTEST_CASES=50000 cargo nextest run -p ui_ext -p fyx_registry`;
  - the cargo-fuzz targets from test 15, at 10 minutes each;
  - commit `*.proptest-regressions` files when they appear.
- **G7. Coverage job** *(CI, weekly, Linux)*. Run `cargo llvm-cov nextest` on the Fyx crates and
  publish `llvm-cov report` restricted to Fyx sources (the workaround from section 1). Fail on a
  per-crate drop of more than 2 points against the last main run.
- **G8. A nextest profile for the gate** (`.config/nextest.toml`):
  - put the real-clock tests (`ui_ext_wasm::tests::test_*budget*|test_runaway*|test_call_blocked*`
    and `ui_ext_host::wasm_tests::*`) in a `test-group` with `max-threads = 1`, so CPU-heavy
    neighbours cannot break their upper bounds;
  - set a `slow-timeout`, so a hung `run_until` reports which test hung;
  - add JUnit output for the CI summary;
  - keep `retries = 0`, so flakiness stays visible instead of being retried away.
- **G9. Release-profile smoke test.**
  `cargo test --release -p agent_review -- script_driver_is_only_in_dev_builds` turns the
  tautological test into a real check. Run `FULL=1` on the schedule the script's header already
  describes; CI does not run it today.
- **G10. Reverse dependents of hooked crates.** Add `-p agent_servers` while `acp_thread` carries a
  hook, or compute rdeps with `cargo metadata`. This is cheap compared with the rest of the gate.
- **G11. Print the first error, not the last 20 lines.** Use `--message-format short` for
  check/clippy, or drop the `tail` for failing runs: `pipefail` already keeps the status.

## Suggested .rules additions

- In `ui_ext_wasm` and `ui_ext_host` tests, a wasm test must call `cx.executor().allow_parking()`
  and `cx.update(gpui_tokio::init)` before `UiExtension::load`. It must wait with `run_until`, not
  only `run_until_parked`, because guest calls run on tokio threads outside GPUI's scheduler.
  (Seen in every wasm test file; forgetting `gpui_tokio::init` panics in `Tokio::spawn`.)
