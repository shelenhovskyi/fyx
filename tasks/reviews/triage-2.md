# Triage 2: five reports → owner action lists

**Inputs.** All five reports cover `integration` at `f3718ca`. They were triaged by W7, read-only.

| Tag | Report | Branch @ commit |
|---|---|---|
| **RB** | `tasks/reviews/rebase-rehearsal-v1.29.1-pre.md` | `claude/fyx-rebase-report` @ `7d84f3f429` |
| **NET** | `tasks/reviews/network-audit-1.md` | `claude/fyx-audit-network` @ `423623709d` |
| **W5D** | `tasks/reviews/w5-deep-1.md` | `claude/fyx-review-w5` @ `4613cd7709` |
| **DIFF** | `tasks/reviews/upstream-diff-audit-1.md` | `claude/fyx-audit-diff` @ `82c54cf587` |
| **LIC** | `tasks/reviews/license-audit-1.md` | `claude/fyx-audit-licenses` @ `a352b6ee46` |

Older reviews are cited as **i3** (`integration-3.md`) and **i2** (`integration-2.md`).

**Markers.**
- **[DUP x]** means the same issue appears in report x. Each issue is listed once, under its owner.
- **[USER]** means the user has to decide.
- **[UP]** means a Zed pull request, which publishes under the user's account, so it also needs the user.

**Severity** is the reporting audit's. Two exceptions:
- NET and DIFF have no severity scale, so W7 assigned severities to their items.
- "(W7)" marks a severity that W7 raised above what the report implies.

## Top 5 overall

1. **W5: review mode silently misses most write paths** (W5D H1, H2, H3). ACP v2 agents, Zed's
   native agent and terminal writes all bypass the review, and nothing tells the user. The review
   UI promises a guarantee that holds only for v1 `fs/write_text_file`.
2. **W5: the review script driver ships in release builds** (W5D M5, [DUP i3 #1], open since
   cycle 3). An environment variable can approve agent edits and dispatch any action. The fix is a
   `cfg` gate of about 3 lines.
3. **W0 / user: `tools/vsix-convert` is GPL-derived code under an Apache-2.0 label** (LIC F1).
   This blocks release. [USER]: relicense it as GPL, or rewrite it clean-room.
4. **W1 / W0: Zed's name, bundle IDs, logo and Terms of Service still ship** (LIC F2, F3).
   This blocks any distributed binary.
5. **W1: two privacy leaks bypass Fyx's off switches.**
   - Copilot Chat calls GitHub at startup when `GH_COPILOT_TOKEN` is set, and ignores `disable_ai`
     (NET #1).
   - Anthropic `/v1/log/zed` usage telemetry ignores `telemetry.metrics: false` (NET #3).

   Each is a one-hook fix.

## W5: agent review

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| High | ACP v2 edits are never reviewed: there is no preview for `DiffPatch`, and v2 has no `fs/write_text_file` | `agent_review.rs:470,474-480`; `agent_servers/src/acp/v2.rs` | Build a `ProposedEdit` from the buffer text plus the patch, and reuse the v1 verdicts. Where no preview is possible, say so on the permission card. |
| High | The native Zed agent writes buffers directly, so review mode has no effect on it | `agent/src/tools/edit_session.rs:1025,1136,208`; `write_file_tool.rs`, `delete_path_tool.rs`, `move_path_tool.rs:268` | [USER] Either add one `fyx:` hook calling the `AgentWriteGate` (put it in `fyx_hooks`, see the W0 section), or declare the native agent out of scope and label its threads "not reviewed". |
| High | Terminal writes and agents that write to disk themselves are neither reviewed nor reported | `acp_thread.rs:8094` (`create_terminal`) | Record file events during a turn for paths with no host write in flight. On `Stopped`, list them ("changed outside review"). Document this in `docs/fyx/`. |
| Medium | Script driver in release builds, as Top 5 #2 | `agent_review.rs:10,58`; `script_driver.rs:43-67,125-127` | `#[cfg(any(debug_assertions, feature = "review-script"))]`; log a `warn!` while active; allowlist the `action` steps. [DUP i3 #1] |
| Medium | Stale base: a held write is applied to whatever the buffer holds later, which overwrites concurrent user edits, other threads' writes and disk reloads | `acp_thread.rs:8011-8031`; `fyx_review.rs:61-65` | After the gate, compare buffer versions. If an accepted hunk intersects a later edit, rebase the review or fail with feedback. Serialize reviews per path. |
| Medium | A partly accepted preview answers only `RejectOnce`: the landed hunks and every comment are lost | `agent_review.rs:404-412,487-524`; `proposed_edit.rs:210-251` | Put `edit.feedback()` in `_meta`, and add it as a host note to the session's next prompt. Add a test for `Deny` with comments. |
| Medium | Failure to write accepted hunks is only logged, and `save_buffer` also saves the user's unrelated unsaved edits | `agent_review.rs:503-509,560-562,565-579` | Keep the review open with "could not apply"; skip the save when the buffer was dirty. [DUP i3 #3] |
| Medium | A held write survives Stop, errors and a closed pane, so a late accept writes after the agent gave up | `agent_review.rs:438-460`; `review_pane.rs:598-620` | Close `Write` reviews on `Stopped`, `Error` and `ConnectionLost`, and when the last pane closes. Show an open-review count. |
| Medium | The pane's hunks (`BufferDiff`) and the model's hunks (`line_diff`) diverge on CRLF and slider cases | `proposed_edit.rs:254-285`; `review_pane.rs:350,644-648` | Derive both from one diff. Normalize line endings before `compute_hunks`. Add a test that maps hunks one to one. |
| Medium | Review mode is not persisted and has no indicator | `agent_review.rs:36-43`; `review_pane.rs:118-129` | Add a user setting and a status-bar indicator. Meet the W5D M6 requirements: projects may only tighten review, and a change of setting never releases held writes. Have W9 check them. |
| Low | Adopted decisions match by text only (L1); a preview's position is ignored | `proposed_edit.rs:183-207,28-38` | Locate the fragment uniquely, and adopt only inside it. Security-relevant, so W9 reviews it. |
| Low | Allowed previews never expire (L2); only the first diff of a tool call is previewed (L3) | `agent_review.rs:235-240,474-480` | Drop the preview on `Completed`, `Failed` or `Stopped`. Create one review per diff, and answer `Allow` only when all are accepted. |
| Low | Cleanup is keyed by session id, so a reload closes the new thread's reviews (L4) | `agent_review.rs:48-52,248-258` | Key reviews by the thread's `EntityId`. |
| Low | Double review of mid-line fragments (L5); format-on-save after a partial write (L6); deleted or renamed files (L7) | `proposed_edit.rs`; `acp_thread.rs:8066-8088` | Compare after locating the fragment; skip formatting on a partial resolution; check `buffer.file()` after the gate. |
| Low | Two consent UIs: the permission card's Always allow ends previews (L8) | `agent_review.rs:341-353` | Disable the card while a preview is open. Needs a small `agent_ui` hook. |
| Low | A new review pane steals focus (L9); a comment goes to the cursor's hunk, not the one just clicked (L10) | `review_pane.rs:181-182,661-667,286-293` | Focus only the window that owns the thread; set `current_hunk` in the button handler. |
| Nit | The responder is taken before the fallible `write_outcome` | `agent_review.rs:214-220` | Swap the order. |

## W1: rebrand, de-cloud, network

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| High | Bundles: Zed names, `dev.zed.Zed*` IDs, the Zed logo, Zed as publisher, welcome and onboarding titles | `crates/zed/Cargo.toml:288-318`; `resources/app-icon*`, `Document.icns`, `windows/*.ico`; `script/bundle-linux:225-234`; `flatpak/zed.metainfo.xml.in:9,37-41`; `script/bundle-windows.ps1:272-314`; `zed.iss:5-6`; `workspace/src/welcome.rs:454-456,483`; `onboarding.rs:350` | Take names and IDs from `release_channel`. Add `fyx:` hooks for welcome and onboarding. [USER] supplies the Fyx icons. Packaging scripts go to W0. LIC F2 [DUP i3 #5, DIFF #9 and B] |
| High (W7) | Copilot Chat calls GitHub at startup with an env token and ignores `disable_ai` | `copilot_chat/src/copilot_chat.rs:584-627`; `zed/src/main.rs:692` | One hook: skip it under `DisableAiSettings`, and fetch models only when the provider is first used. NET #1 |
| High (W7) | Anthropic `/v1/log/zed` ignores `telemetry.metrics` | `language_models/src/provider/anthropic/telemetry.rs:80-123` | One line: return early unless `TelemetrySettings::metrics`. NET #3 |
| Medium | BYO-key model lists are fetched at startup when a folder is open: Anthropic, Google, OpenRouter, Vercel, OpenCode (hourly), ChatGPT refresh | `language_models/src/provider/*.rs` (NET table); `language_model/src/registry.rs:545-576` | [USER] Decide whether "no request at startup" covers configured keys. If yes: fetch on first use, or authenticate only providers named in settings. NET #2 |
| Medium | Git blame hover sends the author's email to GitHub, and repo and SHA to other forges | `git_ui/src/commit_tooltip.rs:152-213`; `blame_ui.rs:326-336` | `git.blame.show_avatar: false` in `fyx.json`, plus a hook so `CommitAvatar::avatar` honours it. NET #4 |
| Medium | Default prettier runs `npm install` when a JSON or JSONC file opens, including `settings.json` | `project/src/prettier_store.rs:600-625`; `default.json:2539-2543` | `languages.JSONC.prettier.allowed: false` in `fyx.json`. NET #6 |
| Medium | LSP version checks and downloads on file open cannot be turned off globally | `lsp_store.rs:585`; `main.rs:546` (TODO) | Expose `allow_binary_download` as one Fyx setting, default `true`. Optionally skip the "latest version" check when a binary is cached. NET #5 |
| Medium | `fyx://` channel links can never parse: the const was renamed *and* the URL is rewritten at the edge | `client.rs:1930,1955`; `open_listener.rs:166,206`; `main.rs:214,1847` | Keep `ZED_URL_SCHEME = "zed"` internally and give OS registration its own `fyx` scheme (`register_zed_scheme.rs:13`). DIFF A |
| Medium | Rebrand tails: the CLI still looks for `Zed.exe`; the clap name is `zed`; macOS `osx_url_schemes` and Linux `.desktop` still register `zed://` | `crates/cli/src/main.rs:1277-1279`; `zed/src/main.rs:1705`; `zed/Cargo.toml:295-322`; `zed.desktop.in:16` | Fix in one sweep with the bundle row above. [DUP i3 #5, DIFF B] |
| Low | `zed.iss` registers `fyx://` but launches `Zed.exe` | `zed.iss:1258-1262` | Revert the hunk now; brand installers at bundle time. [DUP i3 #5, DIFF #9] |
| Low | `crates/zed` package metadata (`description`, `authors`) is Zed's | `zed/Cargo.toml:2,8` | `authors = ["Fyx contributors"]` along with the bundle row. LIC F8 |
| Low | Unmarked edits | `app_menus.rs:74,106,112`; `windows_resources.rs:104` | Add `fyx: ` markers. DIFF C |
| Low | `fyx_default_settings` panics on a bad overlay | `zed/src/main.rs:1828,1835` | Return `Result` and log, while moving the function into `crates/fyx` (W0 row "crates/fyx"). DIFF #2 |
| Low | The no-request registry test lives in `project` | `project/tests/integration/agent_registry_store.rs:11-32` | Move it into a Fyx crate's tests. DIFF #5 |
| Low | LM Studio and llama.cpp probe localhost at startup and ignore `disable_ai` | `lmstudio.rs:222-230`; `llama_cpp.rs:519-521` | [USER] Decide whether loopback counts as "network". If it does, gate both on `disable_ai`. NET, RB "Startup probes" |
| Low | Rebrand sweep after every rebase | 380 "Zed" literals (LIC F2); new strings in `main`'s `main.rs`/`zed.rs` (RB) | `git grep -n '"Zed '` after each rebase. Fix titles, dialogs and notifications first. |
| Info | Upstream PRs: one product constant (DIFF #3), a lazy ACP registry fetch (DIFF #10) | `release_channel/src/lib.rs`, `app_menus.rs`, `agent_registry_store.rs:149` | [UP] They save about 60 hand-edited lines if accepted. |
| Info | Environment overrides: `ZED_SERVER_URL`/`ZED_RPC_URL` still win; `GITHUB_TOKEN` is sent on release lookups; the REPL User-Agent is still `Zed/repl`; remote development fails because the remote-server download is dead | `client.rs:63-65,119`; `github_releases/src/releases.rs:144`; `remote_kernels.rs:163-169`; `auto_update.rs:707` | Document them in `docs/fyx/`. Remote dev needs a Fyx-hosted server binary or a source-build fallback: Phase 5. |

## W0: gatekeeper (Cargo, licenses, scripts, upstream hooks)

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| High | The macOS DMG makes users accept Zed Industries' Terms of Service | `script/bundle-mac:291-293`; `script/terms/*`; `legal/*` | Drop `dmg-license`, or use the GPL notice. State in the README what `legal/` is. LIC F3 |
| High | `tools/vsix-convert` is GPL-derived but labelled Apache-2.0 (W8 is archived, so W0 carries it out) | `tools/vsix-convert/src/theme.ts:1-3`, `syntax.ts:1`, `snippets.ts:272`; `package.json` | [USER] (a) Relicense as `GPL-3.0-or-later` (recommended: a standalone CLI loses nothing), adding a `LICENSE-GPL` symlink; or (b) a clean-room rewrite. LIC F1 |
| Medium | The README is Zed's; there is no GPL §5(a) "modified version" notice | `README.md` | A Fyx README: modified from Zed `v1.28.1`, not affiliated with Zed Industries, the license split. [USER] approves the text. LIC F4 |
| Medium | The `LICENSE-APACHE` copyright line names only Zed Industries | `LICENSE-APACHE:1` | Add `Copyright 2026 Fyx contributors (Fyx-owned crates)`. [USER] decides the holder's name. LIC F5 |
| Medium | Move the `fyx:ui` runtime out of `extension_host` | `extension_host/src/wasm_host/fyx_ui*` (2,382 lines) | Into `ui_ext_host` or a `ui_ext_runtime` crate; copy the 10-line section parser and the cache store. Saves 2,382 lines. [DUP PLAN Phase 5 #9 (W2c)] DIFF #1 |
| Medium | A `crates/fyx` app-glue crate | `main.rs:214-217,498-505,756,1822-1837`; `open_listener.rs:125-131`; `agent_ui/Cargo.toml:31`, `agent_ui.rs:687` | `fyx::init_early` (identity and overlay) and `fyx::init` (`ui_ext_host` and `agent_review`), called after the last upstream `init`. `agent_ui` loses both of its hooks. [DUP RB "Proposed move"; i3 #4] DIFF #2 |
| Medium | Move the write-gate trait to a bottom-of-graph `crates/fyx_hooks` | `acp_thread/src/fyx_review.rs` (105 lines) | Pass `session_id` instead of `&WeakEntity<AcpThread>`. W6's editor hook goes there too. **This reverses W7's i2 #3 ("keep in place")**; the reversal is fine if W6 needs a hook home anyway. W0 decides. DIFF #4 |
| Low | Root `Cargo.toml`: 11 unmarked lines inside sorted lists | `Cargo.toml:9,231-233,299,511-513` (plus `# fyx` with no colon on the demo lines) | Two trailing `# fyx:` blocks with glob members (`crates/fyx*`, `crates/ui_ext*`, `extensions/fyx_*`). Check that `script/clippy` and the license checks accept globs. [DUP i1 #13, RB] DIFF #7 |
| Low | `.cargo/config.toml` forces sccache and the dev profile on everyone; it is the only hunk that fails to back-apply | `.cargo/config.toml:26-35` | Move the settings to env vars in a sourced `script/fyx-env`, used by `gate.sh`. DIFF #6 |
| Low | Rebase procedure | `FYX_UPSTREAM` | v1.29.x: rebase, bump, run `gate.sh`. On `main`: regenerate `Cargo.lock` (`thiserror 2.0.17` → `2.0.21`). Always regenerate the lock last and never hand-merge it. RB |
| Low | Measurement hygiene: three reports use three pathspecs; `fyx:` also matches the `fyx:ui` namespace | `script/` | Add a `script/fyx-diff-stat` with the canonical pathspec. Use the marker `fyx: ` (with a space) and `git grep -n 'fyx: '`. Optionally move `ARCHITECTURE.md`/`PLAN.md` to `docs/fyx/` (W7 consents) and rename `gate.sh` to `fyx-gate`. DIFF #12 |
| Low | One `fyx` action namespace | `zed.rs:6208,6238` | Merge `agent_review` and `fyx_ui` before keymaps depend on them. DIFF #11 |
| Low | The `About Fyx` literal sits next to a line `main` changes | `zed.rs:1721` | `format!("About {}", ReleaseChannel::global(cx).display_name())`. RB |
| Info | Upstream PR: `Dock::panel_index_for_id` | `workspace/src/dock.rs:550-555` | [UP] DIFF #8 |
| Info | Publishing `fyx-sdk` will need the WIT vendored into the crate | `ui_ext_sdk.rs:61` (`../ui_ext/wit`) | At publication: vendor it and handle the license notice. LIC F10 (with W4) |

## W4: SDK and demos

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| Low | An Apache demo test `include_str!`s a GPL source file | `extensions/fyx_demo_gallery/src/fyx_demo_gallery.rs:1616-1619` | Move the icon cross-check into `ui_ext_host`'s tests. LIC F6 |
| Low | "Zed" in the Apache contract docs | `ui_ext_sdk.rs:81`; `fyx_demo_gallery.rs:60`; `fyx-ui.wit:74` (W0, integrator-only) | Say "the host" or "Fyx". LIC F7 |
| Low | The probe and status-item fixtures each carry an 850-line lockfile | `fixtures/*/Cargo.lock` | One shared fixture workspace once DIFF #1 lands. |

## W3: renderer

Nothing new for W3 in these five reports:
- `ui_ext_host` has no network code (NET).
- No upstream API it relies on changed for v1.29 or `main` (RB: `Panel`, `IntoElement`, scrollbars
  and the tab-stop API were all checked).

Carry-overs and new inputs:
- **Carry-over:** i3 #8 (the endless `list-rows` loop after a resync), #9 and #10. These are still
  the W3 queue.
- **New input:** DIFF #1 lands the runtime in `ui_ext_host`. W3 reviews that move. The
  `<target>/fyx-ui-fixtures` coupling (`wasm_tests.rs:24`) goes away with it.
- **Upstream note:** `main` removes `with_rotate_animation`, which the 0.2 design's `spinner` (§I)
  relies on (RB "`ui` `CommonAnimationExt`"). Pick another animation helper when implementing 0.2.

## W9: security

| Sev | Item | Where | Fix sketch |
|---|---|---|---|
| Medium | Extensions' `fetch`/`fetch_stream` have no capability check, and `granted_extension_capabilities` defaults to `host: "*"` | `extension_host/src/wasm_host/wit/since_v0_8_0.rs:633-667`; `default.json:2376-2380` | Gate it through `capability_granter` or a new `http_fetch` capability. Set Fyx's default grant narrower than `*` in `fyx.json`. Candidate [UP]. NET #8 |
| Medium | Threat review of environment-armed features | `FYX_AGENT_REVIEW_SCRIPT`, `FYX_AGENT_REVIEW`, `FYX_UI_DEV_EXTENSIONS`, `ZED_SERVER_URL` | Define which may exist in release builds, which must warn, and which only take user-level config. The script driver row in the W5 section is the first case. |
| Medium | Review-policy requirements before W5 ships the setting and `.fyx/review.toml` | W5D M6 (1–5) | Use them as acceptance criteria. A project can only tighten review; files the agent can write never loosen it. |
| Low | Verify that the Fyx UI WASI context denies sockets in wasmtime-wasi 48 (no `inherit_network`) | `fyx_ui.rs:340,452` | Add a probe test: a guest `TcpStream::connect` fails. NET (*unverified*) |
| Low | Preview decision adoption by content (W5D L1) | `proposed_edit.rs:183-207` | Confirm W5's fix closes the way a harmless-looking preview could launder a hunk. |

## Duplicates resolved

| Issue | Reports | Listed under |
|---|---|---|
| Script driver in release | i3 #1, W5D M5 | W5 |
| Host write of accepted hunks fails silently | i3 #3, W5D M2 | W5 |
| `zed.iss`, installer and bundle branding | i3 #5, DIFF #9 and B, LIC F2 | W1 |
| Unmarked `Cargo.toml` lines | i1 #13, RB, DIFF #7 and C | W0 |
| Move the `agent_review::init` and `ui_ext_host::init` hooks | RB "Proposed move", DIFF #2, i3 #4 | W0 (`crates/fyx`) |
| Move the `fyx_ui` runtime | PLAN Phase 5 #9, DIFF #1 | W0 |
| Loopback LLM probes at startup | RB "Startup probes", NET | W1 [USER] |
| Product-name strings | RB (rebase sweep), DIFF #3, LIC F2 (in-app UI) | W1 |

## Decisions for the user

1. **`vsix-convert` license** (LIC F1): GPL (recommended) or a clean-room rewrite.
2. **Native agent and review** (W5D H2): hook it into the review, or label it "not reviewed".
3. **Scope of "no network at startup"** (NET #2, RB):
   - configured BYO-key model fetches: defer them to first use, or keep them;
   - loopback probes: do they count?
4. **Branding inputs** (LIC F2, F4, F5): Fyx icons, the README and non-affiliation text, and the
   copyright holder's name.
5. **Upstream PRs** (DIFF #3, #8, #10; NET #8): whether to send Zed pull requests from the user's
   account.
