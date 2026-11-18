# Triage 3: five specialist reports, sorted into owner action lists

**Inputs.** All five reports were written against `origin/integration` at `571c6826a5`. Triage was
done by W7, read-only.

| Tag | Report | Branch @ commit |
|---|---|---|
| **PERF** | `tasks/reviews/perf-1.md` | `claude/fyx-review-perf` @ `da039bbbd5` |
| **BUG** | `tasks/reviews/bugs-1.md` | `claude/fyx-review-bugs` @ `5f677ffe5d` |
| **ASYNC** | `tasks/reviews/async-1.md` | `claude/fyx-review-async` @ `49b4942864` |
| **SEC** | `tasks/reviews/security-2.md` | `claude/fyx-review-security` @ `8aa2996324` |
| **UX** | `tasks/reviews/ux-1.md` | `claude/fyx-review-ux` @ `f659094757` |

`tests-1` (`claude/fyx-review-tests`) had not been pushed when this was written. It is not folded
in.

**Fix status.** Status was checked against the branches that are ahead of integration:
- W5: the local branch `claude/fyx-w5-agent-review-45eb11` @ `ab477da229`. Its commit
  `a98221e345` is also on the W1 and W4 remote branches.
- W3: `fyx-w3-renderer-e20d2b` @ `db6619b9f8`.
- W4: `fyx-w4-sdk-demos-5d1497` @ `b7df5e0b5d`.
- W9: `fyx-w9-fixes` @ `9f5028f8ef`.
- Registry: `fyx-registry-core` @ `a5007f7d6b`.

**Fixed** means the code change was verified on that branch. **Claimed** means only the commit
message says so. None of these fixes is on `integration` yet. `fyx_guard` and `fyx_registry` exist
only on unmerged branches.

**Markers.**
- **[DUP …]** merges the same issue from several reports. Each issue is listed once.
- **[USER]** marks a decision for the user. Each one has a recommended answer, so the coordinator
  can decide the technical part.

## Top 7

1. **W9 + W5: decide on the real file, not the typed path.** One shared resolver fixes:
   - a committed symlink, which defeats both the shield and the review rules (SEC S1);
   - Windows aliases such as `.claude.`, `::$DATA` and 8.3 names (SEC T4, BUG 14);
   - case-sensitive globs (BUG 8, 16);
   - a false "was moved" error on a `..` or `\\?\` path. That error fires **even with review off**,
     so Fyx with review off is not identical to Zed (BUG 2).
2. **Merge W5's `a98221e345` and `903febbfde` first.** They fix the High regression from
   integration-4 #1: agent-written `.fyx/review.toml` applies at the next thread start
   [DUP SEC S2, ASYNC 1, BUG 1, BUG 9]. The fix is verified on the W5 branch, but `integration`
   still has the hole.
3. **W5: three review-pane defaults can throw away or accept a whole turn.** All three are
   [USER] decisions:
   - one stray chord, `ctrl-shift-y`/`n`, decides a whole turn with no undo (UX F1);
   - a new review pane steals focus while the user is typing (UX F2);
   - closing the review tab silently rejects every waiting edit (UX F3).
4. **W3: hot reload lets the old instance corrupt the new one.** Stale replies patch the new tree,
   and a stale error permanently fails a healthy extension [DUP ASYNC 2, BUG 3, BUG 4]. A stopped
   surface also revives when its mount finishes (BUG 10). One backend epoch fixes all of them.
5. **W5 + W3: performance.**
   - With review **off**, every edit permission request is still diffed (v1) or read from disk
     (v2) (PERF 2).
   - With review on, the diff runs on the main thread up to 3 times per write
     [DUP PERF 3, ASYNC 14].
   - A surface re-renders its whole tree on any change, a caret blink included: 1.28 s per frame
     at 16384 nodes (PERF 1, 14).
6. **W4: `fyx-ext package` can ship files from outside the extension.** It follows an `assets`
   symlink (SEC S3) and packs a backslashed `component` as `../` (BUG 5). It also drops symlinked
   assets (BUG 26) and packs a component twice (BUG 13). This becomes High as soon as registry CI
   builds authors' repositories.
7. **W3: load failures are invisible.** The install toast says "Installed" before loading
   (UX F5, BUG 29). A manifest that fails to parse counts as an uninstall (BUG 17). A crash shows a
   raw wasm trap with no next step (UX F6).

## W5: agent review (`crates/agent_review`, its hooks in `acp_thread` and `agent`)

| Sev | Item | Where | Fix sketch | Status |
|---|---|---|---|---|
| High | Agent-written `.fyx/review.toml` applies at the next thread start; trust fails open; a read error drops every rule [DUP SEC S2, ASYNC 1, BUG 1, BUG 9] | `agent_review.rs:145,1367-1372,1398-1404,545-550` | `auto_accept` from user settings only; a loosening waits for confirmation; read errors keep the previous rules | **fixed** W5 `a98221e345` + `903febbfde`. Not verified: the per-root load generation (ASYNC 1, part) |
| High | A write to `src/../a.txt` or a `\\?\` path fails with a false "was moved" error, even with review off | `fyx_review.rs:90-98`; `acp_thread.rs:8019` | Compare the buffer's path before and after review; pass the canonical path into `AgentWrite`. Part of Top 7 #1 | open |
| High | Typed paths, not resolved paths, decide `deny` and `always_ask` (symlinks) [DUP SEC S1 (rules half)] | `review_rules.rs:85-97` | Use W9's shared resolver (see W9) | partial `a98221e345`: lexical normalization and case-insensitivity only |
| High [USER] | Turn-wide accept or reject sits on chords Zed already uses, with no confirmation or undo (UX F1) | `review_pane.rs:68-121`; `agent_review.rs:497-517` | **Recommended:** real keymap bindings in `default-*.json` under an `AgentReview` context. Turn keys must not collide on any of the three OSes. Confirm when more than one file waits. Add a "Stop accepting" `ClearTurnDecision`. | open |
| High [USER] | A new review pane steals keyboard focus (UX F2) | `review_pane.rs:187-200,453-459` | **Recommended:** never take focus. Show one toast per turn with **Review**, and an `OpenReviews` binding (`ctrl-k y` / `cmd-k y`). | open |
| High [USER] | Closing the review tab rejects every waiting edit (UX F3) | `review_pane.rs:272-277`; `agent_review.rs:467-486` | **Recommended:** closing keeps the edits held, and the pane shows a dirty dot (`is_dirty`). Rejecting needs the explicit "Reject and Close" button of a close prompt. | open |
| High | The permission card and the review pane decide the same edit (UX F4) | `agent_ui` thread view | Hide the card's buttons and point to the pane | **fixed** W5 `29999cd5f3` (`permission_card_notice`; the Open Review button was not checked) |
| High | With review off, permission requests are still diffed (v1) or read from disk (v2) (PERF 2) | `agent_review.rs:968-1062,1244-1266,1298-1335` | Check `enabled` and `rule_for(path)` first, and return before cloning texts | open |
| High | With review on, the diff runs on the main thread up to 3 times per write [DUP PERF 3, ASYNC 14] | `agent_review.rs:867,895-898,1315`; `proposed_edit.rs:207-252` | `ProposedEdit::new` and `adopt_decisions` in `background_spawn` | open |
| Medium | A permission review is closed while its host write is in flight: the hunks land, the request is answered twice, the turn summary omits them (ASYNC 3) | `agent_review.rs:657-673,839-851` | Treat `applying` as in flight and close in `finish_permission`. This is the same bug class as the integration-4 W5 regression note. | open |
| Medium | The "changed during review" check is not atomic with `buffer.edit`, so typing during the diff await is merged into the reviewed write (ASYNC 4) | `fyx_review.rs:80-107`; `acp_thread.rs:8020-8089` | Run the checks and the edit in one foreground update; skip the save on foreign edits | open |
| Medium | gitignore-style patterns (`/Cargo.lock`, `secrets/`, `./x/**`) parse but never match (BUG 6) | `review_rules.rs:84-107` | Normalize them in `Pattern::new`, or reject them; document | open |
| Medium | The review log formats every unreviewed path on every change, which is O(n²) (PERF 4) | `agent_review.rs:406-436` | Log the count plus new paths at `debug`; keep the reviewed set incrementally | open |
| Medium | `deny` and `auto_accept` decisions are silent, although the docs say they are listed (UX F7) | `agent_review.rs:772,808-826,868-880`; `review-rules.md:29-30` | One toast per turn and rule, with **Show Review Rules**; fix the docs | open |
| Medium | Rule notifications blame the agent, show raw TOML errors (also SEC S10, prompt-injection text) and cannot be re-applied (UX F8) | `agent_review.rs:1380-1426` | Neutral wording, `file:line:col` only, an `agent_review: apply review rules` action | partial `a98221e345` |
| Medium | Review keys are invisible to the keymap tooling (UX F9); failed actions do nothing visible (F10); parts of the pane are mouse-only (F11) | `review_pane.rs:87-121,557-768,884-896` | Keymap bindings (with F1), `Tooltip::for_action_title`, reasons in the header, `NextFile`/`PreviousFile`, `tab_index` | open |
| Medium | The status item counts across windows and does nothing when idle (UX F12); `FYX_AGENT_REVIEW` silently overrides the toggle (UX F13, see W9's environment-variable item); no Settings UI switch (UX F14) | `review_pane.rs:1018-1035`; `agent_review.rs:95-113`; `settings_ui/page_data.rs:3055` | Per-project count plus a menu when idle; a tooltip and toast when overridden; a "Review Agent Edits" item | open |
| Medium | Host-side writes (`write_accepted_hunks`) do not re-check `fyx_guard` (w9-step1 §3) | `agent_review.rs:1468` | Call `fyx_guard::check_config_write` right before writing, once W9 merges | open |
| Low/Med | A held write whose request was dropped keeps its review, and submitting it records "landed" (ASYNC 6) | `agent_review.rs:643-656,917-931` | Await `sender.cancellation()`; record only when `send` succeeds | open |
| Low | A v2 call with two `diff` contents is allowed after previewing the first (SEC S8) | `agent_review.rs:1185-1194` | More than one `DiffPatch` means `Unreviewable`, or one review per diff | open |
| Low | "Accept/reject all in turn" stops at a review that is being applied (BUG 18); a preview plus its write is recorded twice (BUG 19) | `agent_review.rs:497-517,700,817,912` | Filter `!applying`; key entries by preview | open |
| Low | Quitting or closing the window with held reviews says nothing (ASYNC 11) | `review_pane.rs:273-277` | `cx.on_app_quit` closes the reviews explicitly and tells the agent | open |
| Low | The pane names files by file name only, so bidi characters reorder (SEC T7); copy fixes (UX F23) | `review_pane.rs:298,700,716,780,979` | Worktree-relative path, escaped controls, protected-path highlight | open |
| Low | The write gate copies the whole buffer on every agent write, even with review off (PERF 9); a review holds 2–3× the file (PERF 13) | `fyx_review.rs:62-68`; `proposed_edit.rs:59-65` | `wants_review(path)` before `snapshot.text()`; hunk byte ranges | open |

## W9: `fyx_guard` (on `fyx-w9-fixes`, not merged)

| Sev | Item | Where | Fix sketch | Status |
|---|---|---|---|---|
| High | One shared path resolver: symlinks defeat the shield (SEC S1), Windows aliases are missed [DUP SEC T4, BUG 14], case-sensitive security patterns (BUG 16) | `policy.rs:44-98`; `fyx_guard.rs:140-148,280-297` | Canonicalize the deepest existing ancestor (copy `tool_permissions.rs:331` `canonicalize_with_ancestors`). Match both the typed and the canonical path. Deny when the canonical path leaves the worktree. Refuse components ending in `.` or a space, or containing `:`. Case-insensitive globs on case-insensitive volumes. **W5 uses the same helper** (see the W5 resolver row). | open |
| Medium | A project inside `.claude/worktrees/…` or `~/.codex/…` is read-only in shield mode (BUG 15) | `policy.rs:35-44`; `fyx_guard.rs:284` | Apply the protected-path list to the worktree-relative path only. This repository's own worktrees hit it today. | open |
| Medium [USER] | The read-only list misses other agents' configs and Fyx's task sources (SEC T2): `.gemini/`, `.vscode/tasks.json`/`launch.json`, `.agents/`, `.cursor/`, `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.rules` | `policy.rs:20-22` | **Recommended:** add the directories. Make rules files read-only under shield and `always_ask` in review mode by default. | open |
| Medium | A lowering the user declined applies silently at the next start (SEC T3) | `fyx_guard.rs:470-473,548-550,604` | Persist the last confirmed mode. At startup, keep the stricter value and ask again. Show the mode in the status bar while it is lower than confirmed. | open (introduced by the F4 fix in `3851a95912`) |
| Medium [USER] | Features switched on by environment variables: `FYX_AGENT_REVIEW=0` overrides the user setting, and `FYX_UI_DEV_EXTENSIONS` skips consent. A repository's `.envrc` can set either when Fyx is started from a direnv shell [DUP triage-2 W9, UX F13] | `agent_review.rs:62-63,84-93`; `ui_ext_host/consent.rs:10` | **Recommended:** honour both only in dev builds (`ReleaseChannel::Dev`), and show a banner while either is in effect. Owners: W5 and W3 implement it, W9 reviews. | open |
| Low | The environment scrub misses common names: `*PASSWORD*`, `*_PWD`, `*APIKEY*`, `BW_SESSION`, `OP_SESSION_*`, `DOCKER_AUTH_CONFIG`, bare `TOKEN`/`SECRET`, credential URLs [DUP w9-step1 F5] (SEC S9) | `policy.rs:26-41` | Add them; keep `allow_env` | open |
| Low | A denial names every pattern instead of the one that matched (BUG 28); repository TOML errors are quoted to the model (SEC S10, shield half) | `policy.rs:113-119,158-160` | One matcher per pattern; `file:line:col` only | open |
| Low | Built-in `read_file`/`grep` ignore `.fyx/security.toml` (w9-step1 F7) | `crates/agent/src/tools` | One read hook beside the existing `is_path_private` calls | open |
| Low | Submodule hooks `.git/modules/<m>/hooks/` (BUG 27) | `policy.rs:63-68` | — | **fixed** `3851a95912` (all of `.git/` is protected) |
| Low | Guest and agent text in logs: forged lines are possible (SEC T6; the W3 and W5 parts are listed under those owners) | `fyx_guard.rs:104,109` | Use the one shared `sanitize_for_log` helper | open |

## W3: `ui_ext_host`, `ui_ext_wasm`

| Sev | Item | Where | Fix sketch | Status |
|---|---|---|---|---|
| High | A surface re-renders and re-lays out the whole tree on any notify (PERF 1); status items render uncached (PERF 14) | `view.rs:444-552,628-678`; `panel.rs:347-363` | Cached `SubtreeView`s notified by touched ids; `.cached()` for status items. W4 adds a warning above about 1000 nodes to `fyx-ext check` and the docs. | open |
| Medium | Hot reload: stale calls from the old instance patch or fail the new one; a stopped surface revives after its mount [DUP ASYNC 2, BUG 3, BUG 4; BUG 10] | `surface.rs:174-186,213-244,299-324,404-433` | A `backend_epoch` bumped in `replace_backend`; drop results from older epochs; `fail` sets `_mount = None` | open |
| Medium | Load failures are invisible: the install toast comes before the result; a manifest that fails to parse counts as an uninstall [DUP UX F5, BUG 29, BUG 17] | `registry.rs:319-341,489-535,648-665,779-783`; `install.rs:139-156` | One toast per id with **Open Log**; a "Failed to load" chip; the toast only after the scan; unreadable folders are not uninstalls | partial `6ecd47980f`: the consent prompt only |
| Medium | A crash shows a raw wasm trap, Retry is outside the Tab order, the footer has no dismiss key (UX F6) | `surface.rs:248-255`; `view.rs:446-534` | Blame-free text, raw error as detail plus **Copy Error Details**, host controls in `focus_order` | open |
| Medium | Consent TOCTOU: consent is checked on one read and the extension is loaded from another (SEC T1) | `consent.rs:61-67`; `registry.rs:398-420`; `ui_ext_wasm.rs:772-797` | Read the manifest and bytes once, hash them, then `UiExtension::load_checked(id, manifest, bytes)` | open |
| Medium | The time budget is wall-clock on a shared 2-worker Tokio runtime, so contention disables healthy extensions (ASYNC 5) | `ui_ext_wasm.rs:509-517,601-609,846-857` | Count executed epochs; keep a wall-clock deadline only for blocking host calls (shared with the W2c tick budget) | open |
| Medium | Host memory: 8 memories × 256 MiB; reply trees are not counted; no byte cap on the retained tree; the WASI `ResourceTable` is unbounded [DUP integration-4 #7] | `ui_ext_wasm.rs:141,147,319,340` | 1 memory, a byte budget covering the reply, a tree byte cap, a table limit | partial (16 MiB per-call push cap) |
| Medium | `sync` is quadratic (PERF 5); every update clones the tree (PERF 6); props are compared in full (PERF 12); `count_all_inputs` clones the tree (PERF 15) | `view.rs:188-302,976-1006`; `tree.rs:196-205`; `surface.rs:714-720` | Hash sets; `Arc` props; in-place apply with an undo log; walk from the roots | open |
| Medium | Every component is recompiled at each start, one after another (PERF 8) | `ui_ext_wasm.rs:55-87,554`; `registry.rs:305` | Precompiled cache plus `join_all` | partial `d74c8bb2ac` (cache added; loading is still sequential, unverified) |
| Medium | Extension panels cannot be opened from the palette (UX F15); the install dialog and chips speak manifest ids (UX F17) | `ui_ext_host.rs:52-81`; `install.rs:28-42,122-136` | An argument-free `fyx_ui: open extension panel` picker; plain-word chips ("Panels", "Status Bar Items") | open |
| Low | Capability strings are free text and reach the prompts verbatim (SEC S7); consent records are unauthenticated (SEC S4) [USER for S4] | `ui_ext_wasm.rs:219-259`; `consent.rs:76-79,148-159` | Validate `[a-z0-9:-]{1,32}` and show unknowns as "1 unknown capability". **Recommended for S4:** document the limit now, sign records with the permission broker (plan 1.5). | open |
| Low | Guest stderr is raw and unbounded (SEC T5); guest text is logged unescaped (SEC T6); `data:` images in markdown are decoded in-process (SEC T8) | `ui_ext_wasm.rs:462-481,797`; `markdown.rs:1361-1381` | A line-buffered, capped, escaped stderr through `log`; `sanitize_for_log`; no images in extension markdown | open |
| Low | Lifecycle leaks: failed or uninstalled surfaces keep their `Store` (ASYNC 7); a closed workspace's items live on (ASYNC 8); dev reloads are not serialized (ASYNC 9); the event queue is unbounded (ASYNC 10); typing in the debounce is lost on close (ASYNC 12) | `surface.rs:237-244,293-296`; `registry.rs:408-445,612,657-663`; `view.rs:119-127,387-390` | `DeadBackend`; keep the `Subscription` in `WorkspaceSurfaces`; one reload task per id; cap at 64; flush on release | open |
| Low | The epoch thread wakes 100×/s for the whole process (PERF 7); decorations do editor-wide work with no decorations (PERF 10) | `ui_ext_wasm.rs:452-465`; `decorations.rs:167,235-420` | Park on a `Condvar` at zero calls; early return | open |
| Low | A reinstall keeps the old title, icon and kind (BUG 31); PageDown overflows (BUG 30); pushes during instantiation are cleared (BUG 32) | `registry.rs:229-241`; `keyboard.rs:225`; `ui_ext_wasm.rs:622` | Store the new info; `saturating_add`; `begin_call` before `instantiate` | open (suspected) |
| Low | Tab wraps and Escape is the only exit (UX F21); status items cannot be hidden (UX F22); "Lists inside list rows" is author text shown to users (UX F23) | `keyboard.rs:46-135`; `panel.rs:340-377`; `view.rs:824` | Document it and add host controls to the focus order; `HideStatusItem` plus `fyx_ui.hidden_status_items`; log the detail | open |
| Info | Requirements for the coming install store's unpacker (zip-slip) (SEC T10) | `d74c8bb2ac` | Check the store against SEC T10's list before it merges | **review needed** |

## W4: SDK, `fyx_ext`, harness, API docs

| Sev | Item | Where | Fix sketch | Status |
|---|---|---|---|---|
| Medium (High for registry CI) | Package escapes: it follows an `assets` symlink (SEC S3), packs a backslashed `component` as `../` (BUG 5), silently drops symlinked assets (BUG 26), packs a component twice (BUG 13) | `fyx_ext/src/package.rs:26-50`; `manifest.rs:230-235` | Refuse symlinks (`symlink_metadata`, `follow_root_links(false)`). Require canonical paths under `canonicalize(dir)`. Validate `component` the same way on every OS. Dedupe archive names. Add a running byte cap. | open |
| Medium | The SDK cannot mount a tree grown past 2048 nodes (BUG 11) | `ui_ext_sdk.rs:610-614,834` | `mount` checks the tree cap (16384) only, as the host does; fix the gotchas doc | open |
| Medium | `fyx-ext check` passes a manifest the host refuses (UX F5, W4 half); a 0.2 component passes under `api_version = "0.1.0"` (BUG 25) | `fyx_ext/src/inspect.rs:45-48,159-208` | Instantiate in `check`; read the `@x.y.z` of the imports when there is no version section | partial `2b9b01c9e6` (the load depth) |
| Medium | `fyx-ext dev` stops watching when a running Fyx absorbs the child (UX F18) | `fyx_ext/src/build.rs:157-190` | Keep watching on a quick exit 0; later, a separate `--user-data-dir` instance | open (the `main.rs:378` string is W1) |
| Medium | The authoring guide describes an older host (UX F19) | `docs/fyx/extension-authoring.md` | — | **fixed** `15020af7ee` |
| Low | `apply_event` overwrites the mirror while a `set-props` is queued (BUG 20) | `ui_ext_sdk.rs:704-730` | Leave the mirror alone while a patch is pending | open |
| Low | `new` accepts names over 64 characters (BUG 23); `--sdk-*` values are not TOML-escaped (BUG 24); `\\?\UNC\` paths (BUG 33); the dev watcher matches `assets` anywhere (BUG 34) | `template.rs:68-136`; `build.rs:212-253` | `is_valid_id`; `toml::Value::String`; `dunce::simplified`; `strip_prefix(dir)` | open |
| Low | Smaller `fyx-ext` friction (UX F20) | `fyx_ext/src/{main,build,template,package}.rs` | The `--sdk-branch` default should be `fyx/main`; "Clicked 1 times"; an install hint after `package`; help text; a name suggestion | partial `83affa82e5`, `ba61c8c388`, `2b9b01c9e6`: `--lang ts` and the note are fixed |
| Low | The harness sets no store limiter and runs guest code in `check`/`package` (SEC T9, W4 half) | `fyx_ext_harness.rs:210-232` | Reuse `ui_ext_wasm`'s store limits; document a sandboxed CI | open |
| Low | Accessible names: `fyx-ext check` warns about icon-only buttons without a tooltip (UX F16, W4 half) | `fyx_ext/src/inspect.rs` | Add the warning once W2c adds the fields | open |

## W2c: the 0.2 host side

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| Medium | Accessible names: icon buttons and inputs have no label (UX F16) | `ui_ext/src/types.rs:76-88`; `view.rs:705-715` | Add a `label` field next to 0.2's `button-props.tooltip`, plus a host fallback from the icon (W3, now). The review pane part is W5. |
| Medium | Tick and call budgets: count executed epochs, not wall time (ASYNC 5) | 0.2 design §B; `ui_ext_wasm.rs` | Fold the ASYNC 5 fix into the tick CPU-share throttle, so both use the same counter |
| Low | `Arc` props and in-place apply (PERF 6, 12) | `ui_ext/src/tree.rs` | Do it while the 0.2 model types are being changed anyway; both SDK mirrors follow |

## W1: rebrand, network, packaging

| Sev | Item | Where | Fix sketch | Status |
|---|---|---|---|---|
| Low | "zed is already running" when a second instance hands off (UX F18, W1 half) | `crates/zed/src/main.rs:378` | "Fyx is already running" | open |

No other W1 items came out of these five reports. The open W1 items from triage-2 and
integration-4 (findings 5, 6 and 10) are unchanged.

## Registry (`fyx_registry` on `fyx-registry-core`; owner not yet assigned)

The signing format is the part that is hard to change later, so the recommended owner is W9.

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| Medium | A replayed block list stays fresh for ever (SEC S5) | `blocklist.rs:27-33,383-406` | A signed `expires_at`, re-signed daily; age taken from the signed `generated_at` |
| Medium | The index as a whole is unsigned, so a mirror can withhold a fix (SEC S6) | `index.rs:28-35` | A signed snapshot with `generated_at`, `expires_at`, a monotonic `sequence` and the SHA-256 of `index.json`. **Do this before any client ships the format.** |
| Medium | One malformed release discards the whole index (BUG 12) | `index.rs:31-35,289-292` | Parse each release on its own; record failures in `rejected` |
| Low | A forged duplicate evicts the genuine release (SEC S11); Cf characters pass validation (SEC S12); build metadata escapes blocks [DUP SEC S13, BUG 21]; wildcard public-suffix domains (SEC S14); `fetch_due` with a future stamp never fires (BUG 22) | `index.rs:302-317,402,458-471`; `blocklist.rs:73-81,139,212,409-411` | Count duplicates only among verified entries; refuse Cf, U+2028 and U+2029; `cmp_precedence` plus hash block entries; a PSL check; `last_attempt > now \|\| …` |
| Low | Tooling: no key roles or revocation, no size caps, a predictable `.tmp` file, the signing key stays in the environment (SEC T9) | `signing.rs:122-135`; `main.rs:64-66,173-175,312-318` | Key roles plus a revoked list in the snapshot; size caps; `NamedTempFile`; `--package` required |

## W0: gate, Cargo, scripts

| Item | Fix sketch |
|---|---|
| Merge order | Merge W5 `a98221e345` + `903febbfde` first (Top 7 #2). Then W9 (`fyx-w9-fixes`) before W5's host-write re-check, so that `agent_review` can depend on `fyx_guard`. |
| Regression tests the reports already wrote | ASYNC appendix: U1–U2 → `ui_ext_host/tests/async_lifecycle.rs`; A1–A3 → `agent_review`. SEC appendix A: A1–A5 per crate. Ask each owner to land them with the fix, not separately. |
| Suggested `.rules` addition (UX) | Review and surface keys go in `assets/keymaps/default-*.json` under the pane's key context, checked against all three default keymaps, never `cx.intercept_keystrokes` alone. Proposed for `crates/agent_review/.rules` and `crates/ui_ext_host/.rules`. |
| `tests-1` | Fold it into triage-4 when it lands. |

## Decisions for the user, with recommended answers

| # | Decision | Recommended |
|---|---|---|
| 1 | Turn-wide review keys and confirmation (UX F1) | Keymap bindings under an `AgentReview` context, with no collision on any OS. Confirm when more than one file waits. Add "Stop accepting". |
| 2 | May a new review take focus? (UX F2) | Never. A toast with **Review**, plus `ctrl-k y`. |
| 3 | What does closing the review tab do? (UX F3) | Keep the edits held, with a dirty dot. Rejecting needs an explicit "Reject and Close". |
| 4 | Agent rules files (`AGENTS.md`, `CLAUDE.md`, `.rules`) under shield (SEC T2) | Read-only under shield, `always_ask` in review by default. |
| 5 | Features switched on by environment variables (`FYX_AGENT_REVIEW`, `FYX_UI_DEV_EXTENSIONS`) | Dev builds only, with a banner while either is in effect. |
| 6 | Consent records: sign them now or later? (SEC S4) | Document the limit now; sign them with the permission broker (plan 1.5). |

Already decided, by implementation:
- **Project `review.toml` may not auto-accept.** W5 `a98221e345` implements "only user settings
  loosen review".
- **The built-in agent is reviewed.** W5 `29999cd5f3` implements that (w5-deep H2).
