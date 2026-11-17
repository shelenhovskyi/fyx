# Rebase rehearsal: v1.29.1-pre (and upstream `main`)

**Date:** 2026-11-17.

**Fyx side.**
- `origin/integration` at `f3718cae58`.
- Base is `FYX_UPSTREAM` = `v1.28.1` (`e8bc16acee`).
- 107 commits: 75 regular commits and 32 merges.
- 185 files changed in total. Of those, 22 are upstream files Fyx edited: +243/−46, or +150/−46 without `Cargo.lock`.

**Nothing was pushed.** The rehearsal branches exist only in the session's local clone:

| Local branch | Head |
|---|---|
| `rehearsal/v1.28.1` | `8783066818` |
| `rehearsal/v1.29.1-pre` | `264a52986b` |
| `rehearsal/upstream-main` | `ad5ea10c14` |

## Target: there is no newer stable tag

`git ls-remote --tags upstream` lists the stable tags (`v*.*.*` without `-pre`) up to **`v1.28.1`**. That is `FYX_UPSTREAM` itself, so a rebase onto "the newest stable tag" changes nothing today.

What upstream has instead:

- **`upstream/v1.28.x`** has 4 cherry-picks past the tag. They are not tagged yet, and would become v1.28.2: helix clipboard, vim substitute, an LM Studio/llama.cpp probe fix, and docs. None of them touches a file Fyx edits, and the merge is clean.
- **`v1.29.1-pre`** (`dcfc7ee77c`, 2026-11-11) is on `v1.29.x`, so it is what the next stable will be built from.
  - It has 78 commits since `v1.28.1`: 223 files, +15027/−4814.
  - Zed cuts a stable from the preview branch about weekly, so this is the next rebase Fyx will actually do.
  - The `v1.29.x` tip (`6dfd108652`, 5 more cherry-picks) also merges cleanly.
- **`upstream/main`** (`4122f44f6f`, 2026-11-17) is about two releases ahead: 170 commits, 471 files, +45094/−12131. I rehearsed this too, to see what is coming.

The report is named after the tag that was rehearsed.

## Method, and why

**1. Linear replay.** Cherry-pick every regular commit in `FYX_UPSTREAM..origin/integration`, oldest first. This is what `git rebase --onto <new> FYX_UPSTREAM integration` would do, because rebase drops merges.
- I first replayed onto `v1.28.1` itself, as a control.
- It produced **zero conflicts**.
- The resulting tree is **byte-identical to `origin/integration`**.

So none of the 32 worker-branch merges contains a conflict resolution or other content of its own. They carry no information, and linearizing them loses nothing. Conflicts between Fyx streams do not distort the replay, so it measures only upstream cost.

**2. Squash 3-way merge.** `git merge-tree --merge-base=v1.28.1 <target> origin/integration` shows each upstream/Fyx collision exactly once, however many commits touched the hunk.

Both methods give the same tree for both targets: `rehearsal/v1.29.1-pre` = merge-tree `113a6940f7`, and `rehearsal/upstream-main` = merge-tree `9e6cb34171`. Plain `git rebase` would therefore show the real cost. The merges only make its log noisy, not its conflicts.

**3. Semantic review.** For every upstream file or API that Fyx code touches, I read the upstream diff between `v1.28.1` and each target. That covers `ui_ext_host`, `agent_review`, `extension_host::wasm_host::fyx_ui`, `acp_thread::fyx_review`, the settings overlay, the `Panel`/`Item` traits, gpui/ui, ACP and the lockfile. Signatures were compared across versions, not just checked for existence.

**4. Compile check.** `cargo check -p ui_ext -p ui_ext_sdk --locked` on both rehearsal trees. `zed` and the host crates were not built, as the brief asked.

## Verdict

**The next rebase costs almost nothing.**

| Target | Textual conflicts | Lockfile | `cargo check -p ui_ext -p ui_ext_sdk --locked` |
|---|---|---|---|
| `v1.29.1-pre` | 0 (75 of 75 picks clean) | consistent | pass (4m32s cold, 0 warnings) |
| `upstream/main` | 0 (75 of 75 picks clean) | **stale: 1 entry** | fails until the lock is regenerated, then passes |

10 files are changed on both sides for `v1.29.1-pre`:
- `Cargo.toml`, `Cargo.lock`
- `acp_thread.rs`, `agent_ui.rs`, `wasm_host.rs`, `workspace.rs`
- `zed/Cargo.toml`, `zed/main.rs`, `zed/zed.rs`, `open_listener.rs`

Every Fyx hunk in them is at least 3 lines away from upstream's hunks (see "Near misses"). The static review found no renamed or removed upstream API that Fyx code calls.

**Expected work for the real `v1.29.x` rebase:**
- `git rebase`.
- Bump `FYX_UPSTREAM`.
- Run `script/gate.sh`. Its host-crate `cargo check`/clippy/nextest is the one thing this rehearsal could not run (see "Not verified").

**Before the rebase after that (onto what `main` becomes):** expect one `Cargo.lock` regeneration.

## Conflicts

### Textual

None, for either target.

### Semantic: `Cargo.lock` on `main` (silent: the merge is clean, but cargo rejects the lockfile)

- **Fyx side.** `ui_ext`'s entry (W2/W0, crates/ui_ext) lists `"thiserror 2.0.17"` under `dependencies`.
- **Upstream side.** `78a8526196` "Update merman to 0.9.3 (#66874)" moves the workspace from `thiserror 2.0.17` to `2.0.21` and deletes the `2.0.17` package block.
- **Result.**
  - Git merges the two hunks cleanly, because they are far apart.
  - `cargo … --locked` then fails with "cannot update the lock file".
  - `upstream/main` alone resolves fine with `--locked`.
- **Resolution.** After the rebase, run any cargo command without `--locked`, for example `cargo metadata --format-version 1 >/dev/null`. The only change is that one line, `thiserror 2.0.17` → `2.0.21`. Commit it as a W0 lockfile commit. The rehearsal check then passes. `script/gate.sh` already fails when cargo rewrites a lockfile, so this cannot slip through.
- **Can the hook move to avoid it?** No. Any Fyx crate with a dependency also used upstream gets a versioned lock entry. The fix is procedural: always regenerate and commit `Cargo.lock` as the last step of a rebase, and never hand-merge it.

## Near misses and hooks worth moving

Distance is measured in base lines from each Fyx hunk to the nearest upstream hunk. Git merges cleanly when the gap is more than about 3 lines of context. "Churn" is the number of upstream commits that touched the file in the last 6 months on `main`.

| Fyx hook | Gap (1.29 / main) | Churn | Recommendation |
|---|---|---|---|
| `zed/src/main.rs:748` `ui_ext_host::init(cx)` in the big init list | **3 / 3** | 39 | **Move.** `project_bookmarks::init` landed 3 lines above it in 1.29. This list is where upstream adds every new crate's `init`. See (1). |
| `agent_ui/src/agent_ui.rs:686` `agent_review::init(cx)`, plus the `agent_ui/Cargo.toml` dependency | 422 / 422 | 47 (file); the `init` block changed 5 times since March | **Move** to `crates/zed` with (1). `ARCHITECTURE.md` already says `agent_review::init` belongs in `crates/zed`. This removes two hooked files. |
| `zed/Cargo.toml:9` `default-run = "fyx"` | **4 / 4** | 58 | **Keep.** It is 4 lines below `version =`, which upstream bumps every release, so it will conflict as soon as upstream adds a key between them. It cannot move, and the fix is one line. |
| `zed/src/main.rs:213` `let mut args = Args::parse()` plus the `fyx://` rewrite | 102 / **5** | 39 | **Keep for now.** `main` adds `sandbox_linux::run_hidden_entrypoint_if_invoked()` just above it, and the gap is still clean. It must run before `Args` is used, so there is no better spot. |
| `zed/src/zed/open_listener.rs:157` `let url = fyx_url_to_internal(url)` | **8 / 8** | 22 (4 commits in 1.29) | **Keep.** `OpenRequest::parse` is the one place all URL sources meet (`main.rs:939`, `main.rs:1005`, CLI `open_listener.rs:609`). Moving it to `OpenListener::open` would miss two of them. It is one line at the top of the loop. |
| `zed/src/zed.rs:1721` "About Fyx" title | 568 / **4** | 70 | **Keep**, low value. `main` changes the `WindowBounds::centered` call 2 lines below. Branding goes through `ReleaseChannel::display_name()` elsewhere, so switching this literal to `format!("About {}", ReleaseChannel::global(cx).display_name())` would make it line-stable. It still needs a hook. |
| `zed/src/zed.rs:6207,6236` action-namespace test list | 57 / 57 | 70 | **Keep.** It is an alphabetical list that upstream extends, so a future conflict is likely but trivial to resolve. |
| `acp_thread.rs:8015,8084` review gate in `write_text_file` | 838 / 838 | **129** (file); the function changed 4 times since March | **Keep.** The file churns heavily but this function rarely changes. Its body is byte-identical in v1.28.1, v1.29.1-pre and `main`. It is two short inserts; the alternative (`agent_servers/src/acp/v1.rs`, 10k lines) is worse. |
| `workspace/src/workspace.rs:13292,13307` window-title test strings | 1672 / 361 | 95 | **Keep.** Test-only. On a conflict, take upstream's version and redo `Zed` → `Fyx`. |
| `Cargo.toml` members and workspace dependencies | ≥34 / ≥34 | 175 | **Keep** (integrator-only). These are the only hunks without a `fyx:` marker; `Cargo.toml` takes inline comments, so markers could be added. |

Every other hook is in a file upstream did not touch for either target: `client.rs`, `paths.rs`, `release_channel/src/lib.rs`, `windows_resources.rs`, `dock.rs`, `app_menus.rs`, `zed.iss`, `.cargo/config.toml`, `extension_host/Cargo.toml`, and the registry-store test. On `main`, `agent_registry_store.rs` changed 230 lines away from the hook.

**(1) Proposed move.**
1. Add one `fyx_init(cx)` next to `fyx_default_settings` at the bottom of `crates/zed/src/main.rs`, calling `ui_ext_host::init(cx)` and `agent_review::init(cx)`.
2. Call it once, after the last upstream `*::init` call in the init block. Upstream appends new crates in the middle of that list, so the call is much less likely to have upstream edits next to it.
3. `agent_review` becomes a `crates/zed` dependency (integrator-only `Cargo.toml`). Drop the hook from `agent_ui`.
4. Ordering constraints:
   - `agent_review::init` only needs to run before the first `AcpThread` exists. It uses `observe_new`.
   - `ui_ext_host::init` needs workspace and settings initialized. Those run earlier in the list anyway.

   Verify with the phase-4 T1 manual test after moving. The move saves one fragile line in a hot list and two hooked files.

## Upstream changes that could silently break Fyx: checked

Each item was checked against the actual signatures and call sites in Fyx code. The "next" column is `v1.29.1-pre`.

| Area | Upstream change (next / main) | Fyx dependency | Status |
|---|---|---|---|
| **gpui `Element`/`IntoElement`** | `39467cf4b1`: `Element` no longer requires `IntoElement`; a blanket `impl<T: Element> IntoElement for T` replaces hand-written impls; `AnyElement::into_any_element` override removed | `ui_ext_host`/`agent_review` call `.into_any_element()` (~30 sites) and implement neither trait by hand | OK: `IntoElement::into_any_element` still exists in both. A future Fyx custom `Element` must not also `impl IntoElement` (overlapping impl). |
| **gpui tab stops** | `62682ebde9`: `TabStopMap::next/prev` take `&mut self`, order computed lazily | `ui_ext_host/src/keyboard.rs` keeps its own `focus_order`, uses `track_focus` and `FocusHandle::focus` only | OK. |
| **gpui `list()` padding** | `78fde14d12`: horizontal padding now applied | Fyx uses `uniform_list`, not `list()` | OK, nothing to undo. |
| **gpui `Div` interactivity** | `4ce4b7b567`: moved out of line | Builder API only | OK. |
| **gpui test context** | Only additions (`simulate_decorations_change`) | `ui_ext_host` tests use `add_window_view`, `simulate_*`, `run_until_parked` | OK. |
| **gpui `WindowBounds::centered`** | main: gains a `display_id` argument | Not called by Fyx | OK. |
| **`ui` scrollbar** | `ScrollableHandle::into_scrollbar_element` and `ScrollbarElement`'s `IntoElement` impl removed | `WithVerticalScrollbar` uses `vertical_scrollbar_for` on `ScrollHandle`/`UniformListScrollHandle` | OK. |
| **`ui` `CommonAnimationExt`** | main: now `Transformable + Sized`, and `with_rotate_animation` is gone | Not used | OK. |
| **`Panel` trait** (`workspace/src/dock.rs`) | Unchanged in both | `ui_ext_host::panel` implements it; Fyx added `Dock::panel_index_for_id` | OK. The method list hashes identically in v1.28.1 and main. |
| **`Item` trait** | main: `toggle_read_only` replaced by `read_only_toggle_buffer` | `ReviewPane` overrides neither | OK. The default `None` is the right behaviour for a review pane. |
| **editor `DiffHunkRenderer`** | Unchanged | `review_pane::ReviewHunkRenderer` | OK. Identical in v1.28.1 and main. |
| **`acp_thread` permission API** | Plan operations (`1478fb5e60`, `b5be8e79f7`); `Plan::is_empty` gains `cx` | `agent_review` uses `permission_request`, `legacy_options`, `allow_once_option_id`/`deny_once_option_id`, `legacy_tool_call_id`, `tool_call`, `ToolCallContent::LegacyDiff`, `authorize_permission_request`, `ToolAuthorization{Requested,Received}` | OK: the signatures are identical in v1.28.1 and main. `ToolCallContent` gains a `Session` variant (5.2.0), and Fyx matches with `_`. |
| **ACP SDK** | `=5.0.0` → `=5.1.0` (next) → `=5.3.0`/schema 1.18 (main) | `acp_v2::{SessionId, PermissionOptionKind::{AllowOnce, RejectOnce}}`, `Error::new`, `ErrorCode::INTERNAL_ERROR` | OK: all present in 5.3.0 / schema 1.18. The pin is exact, so it moves with the rebase. |
| **ACP feedback path** | `agent_servers/src/acp/v1.rs` rewritten in places | W5 returns review feedback as the `write_text_file` error, which `handle_write_text_file` maps with `agent_client_protocol::Error::from` | OK: that handler is unchanged in main. |
| **Native agent permissions** | main `12c1fc907f`: `edit_file`/`write_file` prompt when an `always_confirm` pattern matches an in-project path | `agent_review` previews only permission requests with a `LegacyDiff` | OK. In-project edits under a `confirm` default still do not prompt, so nothing Fyx relies on changed. |
| **`extension_host::wasm_host`** | `on_main_thread` now takes `AsyncFnOnce`; `extension.wit` v0.8.0 gains `bzip2`/`bzip2-tar` | `fyx_ui.rs` uses `super::cache_store()`, `super::parse_wasm_extension_version_custom_section`, `super::wit::wasm_api_version_range`, and its own `Engine`/`Linker` | OK: all three helpers are identical in main, and `on_main_thread` is not used. `wasmtime`/`wasmtime-wasi` stay at 48.0.7. The `fyx:ui` world does not import `zed:extension`. |
| **Settings schema** (`assets/settings/fyx.json` overlay) | New keys: `title_bar.show`, `git.*.date_style`, `agent.auto_generate_thread_titles`, and more | Every key Fyx overrides still exists: `server_url`, `telemetry.{diagnostics,metrics}`, `auto_update`, `title_bar.{show_sign_in,show_onboarding_banner}`, `collaboration_panel.button`, `edit_predictions.provider`, `auto_install_extensions`, `suggest_extensions` | OK. `parse_json_with_comments`, `merge_non_null_json_value_into`, `set_default_settings` and `asset_str` are unchanged. |
| **URL scheme** | `windows_only_instance.rs` gains `split_paths_and_urls`; `open_listener` gains `zed://file` drive-letter handling and multi-`ssh://` requests | Fyx rewrites `fyx://` → `zed://` in `main()` before `Args` is used, and again in `OpenRequest::parse` | OK. The new code only checks the internal `zed://` prefix, which Fyx produces. `ZED_URL_SCHEME` still has its two uses. |
| **Window title test** | No new upstream tests assert `"Zed — …"` titles | Fyx changed 2 assertions | OK. The only `"Zed — Settings"` literal, in `settings_ui.rs:900`, was already there in v1.28.1. |
| **Registry "no request at startup"** (W1) | main `cb48e27ceb`: platform-key probe becomes `async` (musl), and `init_global` is untouched | Fyx removed the eager `refresh` from `init_global` | OK. No new `init_global` callers: the same files call it as in v1.28.1 (`main.rs`, `zed.rs` test, `visual_test_runner.rs`, `headless_project.rs`, the registry tests). |
| **Startup probes** | `v1.28.x`/`v1.29.x` cherry-pick `3e42c7de13`: no LM Studio/llama.cpp probe when `disable_ai` is true | W1 promises no request at startup | **Note, not a break.** Those probes go to localhost and already ran in v1.28.1. If "no request at startup" is meant to include loopback, `fyx.json` would need the providers off. Owner's call. |
| **Product name in new upstream strings** | main `3d42ca7c09` (`cli: Add debug state command`), `f99c53f4f6` (restore session, +507 lines in `main.rs`), `zed.rs` +1966 lines of tests | Branding goes through `ReleaseChannel`/`APP_NAME` | Not audited line by line. New user-visible "Zed" literals are W1's routine rebrand sweep: run `git grep -n '"Zed '` on the rebased tree. |

## Not verified

- **`cargo check`/clippy/nextest of `ui_ext_host`, `agent_review`, `extension_host`, `acp_thread`, `agent_ui`.** The brief allowed only `ui_ext`/`ui_ext_sdk`, and those depend only on serde/thiserror/wit-bindgen, so they prove the lockfile and little else. The gpui `Element` change is the one most likely to surface at compile time. The static review found nothing that would break, but the gate is the real test: `script/gate.sh v1.28.1` on the rebased branch checks every changed crate.
- **The `zed` binary and manual phase-4 tests**, on any platform.
- **Windows packaging** (`zed.iss`, `windows_resources`). Upstream did not touch these files.

## Procedure for the real rebase

```bash
git fetch upstream v1.29.x --tags
git switch -c fyx/rebase-v1.29.x origin/integration
git rebase --onto <v1.29.x stable tag> "$(cat FYX_UPSTREAM)"
```

```bash
cargo metadata --format-version 1 >/dev/null
git diff --stat Cargo.lock
```

Commit a lockfile change if there is one. There is none for `v1.29.1-pre`; on `main` it is `thiserror 2.0.21`.

```bash
echo <tag> > FYX_UPSTREAM
script/gate.sh <tag>
```

Then run `git grep -n 'fyx:'` and diff the list against the previous one. It should be unchanged.

The worker merges carry no resolutions (verified above), so linearizing them is safe. The rebased history is 75 linear commits.

## Reproduce

```bash
git remote add upstream https://github.com/zed-industries/zed.git
git fetch upstream v1.29.x main --no-tags
git fetch upstream tag v1.28.1 tag v1.29.1-pre --no-tags
```

Squash merge (exit 0 means no conflicts):

```bash
git merge-tree --write-tree --name-only --merge-base=v1.28.1 v1.29.1-pre origin/integration
```

Linear replay:

```bash
git switch -c rehearsal/v1.29.1-pre v1.29.1-pre
for c in $(git rev-list --reverse --no-merges --topo-order v1.28.1..origin/integration); do git cherry-pick "$c" || break; done
```
