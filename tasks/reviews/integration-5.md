# Judge review: integration-5 (plan 4.9, judge pass over P2–P4)

**Scope.** Everything merged on `origin/integration` since integration-4 (`1ec364bfc0`) up to
`a3e640ed85`: 166 merges, 456 files, +114232/−3601. It covers:
- `fyx:ui` 0.2: host, renderer, SDK, §L decorations, §M notifications;
- P1 1.5–1.7: permission broker, instance limits, Inspector;
- P3: `fyx_registry` client, template, `registry_url`, Fyx Extensions page, block list;
- P4 4.5–4.8 and 4.10–4.15: T2 consent, `network:fetch`, `process:spawn`, `workspace:edit`,
  `agent:observe`, images, canvas.

**Unmerged tips, also read:**
- W2c `claude/fyx-w2c-host-0-2-cbde2c` @ `85a7d3c2ed`. The brief said `b3822c6fce`; the branch has
  moved on, and `85a7d3c2ed` contains it.
- W3 `claude/fyx-w3-renderer-e20d2b` @ `f5a3b863fc`.
- W4 `claude/fyx-w4-sdk-demos-5d1497` @ `8eea40d0de`.

**Method.** Read only. Nothing was built or run, except `wasm-tools component wit` on the 0.2, 0.3
and design WIT files, all of which parse. Four reviewers read the code in parallel:
- the T2 paths (security);
- the API and drift;
- the registry with P1;
- the renderer and lifecycle.

W7 read the upstream surface. W7 also re-checked these claims against the code:
- the 0.3 WIT header and `LINKED_INTERFACES`;
- `MAX_FROZEN_VERSION`;
- the Fyx tab's install path;
- `is_protected_path`;
- `agent_observe.rs`, which has no guard filter.

## Freeze recommendation: **not ready**

Decision 30 starts a 4-week no-break clock on 0.3. That clock cannot start yet, for three reasons:
- the 0.3 WIT calls itself an undecided draft;
- 12 of its 14 open questions are unanswered;
- about half of what it declares has no host behind it.

**Two blocking lists.** The **clock** list (A) contains only what could change the ABI. The
**1.0 ship** list (B) can be worked on while the clock runs, because none of it changes the WIT.

### A. Blocks starting the clock (API)

| # | Blocker | Where | Owner |
|---|---|---|---|
| A1 | The 0.3 WIT is a "STAGED DRAFT, not frozen and not fully decided". Q1–Q12 are open, and Q3 (document context) and Q9 (version lifetime) are hard to change after 1.0. | `crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit:1-4`; `docs/fyx/wit-0.3/design.md:1210-1268` | owner + W0 |
| A2 | 0.2 is not frozen: `MAX_FROZEN_VERSION = 0.1.0`. Its comment says freezing 0.2 sets it to `MAX_VERSION`, which is now 0.3.0, so freezing 0.2 that way would also ship the staged 0.3 to Stable and Preview. | `ui_ext_wasm.rs:132-135` | W0, W2c: freeze 0.2 with an explicit `0.2.0` and fix the comment |
| A3 | Declared but not linked: `decorations`, `documents`, `language`, `virtual-documents`, `picker` and `permissions` **trap** when called. They should return `unavailable`. Seven exports are never called by any host: `handle-command`, `provide-decorations`, `provide-hover`, `provide-code-lenses`, `provide-virtual-document`, `picker-source-event`, plus picker surfaces refused at `:489`. A 0.3 component also loses 0.2's `place-decoration`. Plan 4.2–4.4 has not started. | `fyx_ui_v0_3.rs:489,501-510`; `migration.md` §2 says "every 0.3 interface is linked" | W0 decides: **move these interfaces out of 0.3 into 0.4**, or build 4.2–4.4 before the clock. Recommended: move them out, and freeze what works. |
| A4 | The manifest cannot express 0.3: `UiManifest` (`deny_unknown_fields`) rejects `[[fyx_ui.commands]]` and picker sources. Capability checks ignore the world version, so a 0.1 or 0.2 manifest may declare `ui:picker`, `ui:canvas` or T1 capabilities. The version-first parse that `migration.md` §2 requires is missing. | `ui_ext_wasm.rs:290,491-503` | W2c |
| A5 | Shapes that will need a break: <br>• `spawn-error` and `edit-error` have no `other`/`unavailable` case, unlike `fetch-error`; <br>• `review-info.truncated` means both "hunk cut at 256 KiB" (WIT) and "older reviews dropped" (`ui_ext/src/observe.rs:126-137`), and a resync `snapshot` replacing everything is undocumented; <br>• buttons have no accessible `label` (ux-1 F16); <br>• the picker has no busy or empty state; <br>• code lens has no resolve path. | 0.3 WIT `:1221` and error types | W2c, W0 |
| A6 | Undocumented host caps in the WIT. The host enforces them, so they are part of the contract: <br>• tree depth 64; <br>• 32768 pushed items per call; <br>• 512 decoration effects per call; <br>• 4 `notify` calls per export call; <br>• 24 h maximum tick delay (`invalid-argument`); <br>• `.idea` protected; <br>• asset paths ≤1024 bytes, `.png`/`.svg` only. <br>Also, `show-surface` omits `canvas-click` and `notification-action` as gestures. | `tree.rs:24-26`; `effects.rs:26,31,249-254`; `api_version.rs:23`; `notifications.rs:18`; `edit.rs:24`; `image.rs:27,164` | W2c (doc comments only) |

When A1–A6 are done and 0.2 is frozen, start the clock. Every A item is a WIT or manifest change.
Each one that lands after the clock starts resets it.

### B. Blocks shipping 1.0 (no ABI change; fix during the clock)

See the findings below: R1, R2, S1–S5, H1, H2 and M4. R1 and R2 come first.

## Top 7

1. **0.3 is not freezable** (A1–A3): a staged draft, Q1–Q12 open, six interfaces that trap, seven
   exports never called.
2. **Registry: the Fyx tab installs blocked releases and skips the 24 h wait** (R1).
3. **Registry: a leaked snapshot key can undo its own revocation** (R2).
4. **`agent:observe` sends hunks of private files to extensions.** Together with `network:fetch`,
   this exfiltrates `.env` (S1).
5. **`workspace:edit` can write agent configs that run code** (`.claude/settings.json` hooks,
   `sub/.zed/settings.json`, `.mcp.json`) whenever the shield is off, which is the default (S2).
6. **SVG decode bombs from a T0 extension** (`ui:images`, no prompt): `.svgz` and `<use>`
   expansion are not capped, and a decode timeout does not stop the decode (S3).
7. **Renderer: modals never tick (H1), and the image cache evicts on-screen images, so it decodes
   forever (H2).**

## Findings

### Registry (P3, W3 owns the client side)

| Sev | # | Finding | Where | Fix |
|---|---|---|---|---|
| High | R1 | The Fyx tab installs through `registry.download`, not `download_for_install`, so the block list, the 24 h wait and pins are all bypassed. The listing uses `index.latest(id)`. Confirmed: nothing outside the tests calls `download_for_install`. | `fyx_extensions_page.rs:215-220,363,503,707-770`; `registry_service.rs:360` | Build the listing from `plan_install` and install through `download_for_install`. Show "Blocked" or "Available at …" on a disabled button. |
| High | R2 | Snapshots are verified against the full compiled keyset, and only block-list revocations carry over, so an attacker with a leaked key signs sequence+1 without the revocation. | `fyx_registry/src/snapshot.rs:182,215-229`; `client.rs:219` | Verify with `previous.effective_keyset()`; keep every revocation; only a revocation-role snapshot may lift one |
| Med-High | R3 | A block is not applied "at once" (spec 19). The first check runs 6 h after start, and page refreshes don't apply. A session shorter than 6 h never checks, and an in-flight refresh skips a cycle. | `fyx_extensions_page.rs:100-110,184,191,863` | Apply Rollback and Disable on every refresh, and persist the last check time. **[USER]** spec 19's "fetched at startup" conflicts with "no request at startup". **Recommended:** keep no startup request; check when the page opens and 6 h after the last check, which is persisted. |
| Medium | R4 | A revoked permission comes back without a dialog: `grant_and_rescan` grants every `consent_items()` after Roll Back, Approve, Update or a registry install. | `consent.rs:358`; `store_actions.rs:214`; `fyx_extensions_page.rs:519` | Remove the permission from `approved` on revoke; grant only what the dialog showed |
| Medium | R5 | Install-from-file reads the package twice: once for the dialog, once to install. | `store_actions.rs:342,366` | Read the bytes once |
| Medium | R6 | Registry CI treats any `gh release download` error as "first release" and resets the sequence to 1, so clients refuse the next snapshot. | `tools/fyx-extensions-template/.github/workflows/publish.yml:104-106,143-147`; `blocklist.yml:37-40` | Fail on errors; start at 1 only with an explicit `--first-run` |
| Med/Low | R7 | A registry package is not checked against its index entry. The id, version and capabilities come from the package's own `extension.toml`, while the dialog shows the index's. | `install_store.rs:274` | Same id and version required; `consent_items() ⊆` the release's permissions |
| Med/Low | R8 | The 24 h wait and `urgent` rest on `published_at`, which the online release key signs, so a leaked key can backdate a release. | `update.rs:39-47` | Count from first sight in an accepted snapshot; consider an owner-signed `urgent` |
| Low | R9 | Smaller issues: <br>• manual rollback brings back a blocked version and leaves `disabled` set; <br>• consent ignores the component hash, contradicting `extension-authoring.md:448`; <br>• ids from `state.json` are not validated before `uninstall` deletes a folder; <br>• the file-install dialog omits the hosts and programs it grants; <br>• consent saves can land out of order; <br>• the block-list sequence is forgotten on a cache drop; <br>• an expired list is accepted when there is no cache; <br>• hash-only blocks never match installs with no recorded hash; <br>• role separation is not checked in release builds; <br>• the docs say "daily" where spec 18/19 say 12 h; <br>• the unpacker misses Windows reserved names, trailing dots or spaces and case-only duplicates (SEC T10, partial). | `install_store.rs:348-401,509,566`; `consent.rs:320-323`; `client.rs:420-422`; `blocklist.rs:439-441`; `update.rs:145`; `signing.rs:188` | as listed |

**Resolved from triage-3:**
- S5 (block-list replay), S6 (unsigned index), BUG 12, S11, S12, S13/BUG 21, BUG 22, SEC T1 on the
  load path, S7 (capability strings).
- Plan 1.6 (limits) and 1.7 (Inspector) are done.
- **Partial:** S14 (about 90 hand-picked public suffixes), T9 (the CLI's `read()` is still uncapped),
  and integration-4 #7. Memory is now capped at 256 MiB across memories and the table at 4096, but
  there is still no byte cap on the retained tree.

**No network requests at startup.** `registry_url` is empty by default, which means off. The
registry is opened only from the Fyx tab or the 6 h timer, and release builds compile in no keys.

### T2 paths (P4, W2c and W3)

| Sev | # | Finding | Where | Fix |
|---|---|---|---|---|
| Medium | S1 | `agent:observe` sends old and new hunk text for any file inside a worktree. It applies no `private_files`, `file_scan_exclusions` or `fyx_guard` filter, so a written `.env`, or a write a Deny rule refused, reaches an extension that also has `network:fetch`. This is inconsistent with `workspace:edit`, which refuses the same paths. | W3 tip `f5a3b863fc`, `ui_ext_host/src/agent_observe.rs:201` (no guard reference in the file) | Drop the hunks (and the path) when `fyx_guard::check_agent_read` or `private_files` denies the file |
| Medium | S2 | `workspace:edit` protects only the **first** path component (confirmed: `is_protected_path`). `fyx_guard`'s config rules apply only in Shield, which is off by default. Editable by default: `sub/.zed/settings.json`, nested `.vscode/tasks.json`, `.claude/settings.json` hooks, `.codex/`, `.cursor/`, `.mcp.json`, `AGENTS.md`, `CLAUDE.md`. The host does not save these edits, but autosave or "save all" will. | `ui_ext/src/edit.rs:140-147`; `fyx_guard.rs:148-155` | Apply `protected_config_rel` at any depth, **regardless of shield mode**, for extension edits. The same rule list serves both. |
| Med-High | S3 | SVG decode bombs from a T0 extension: <br>• asset SVGs skip `check_image_bytes`; <br>• `.svgz` inflates uncapped; <br>• `<use>` expands before node counting, and `MAX_SVG_USE_DEPTH` is defined but unused; <br>• `DECODE_TIMEOUT` frees the concurrency slot but not the thread, so runaway decodes pile up past the 2-per-extension / 4-total caps (renderer M3 is the same issue). | `ui_ext_host/src/images.rs:318-336,425-430,504-505`; `ui_ext/src/image.rs:44` | Run `check_image_bytes` on assets; pre-scan the XML with a node limit and cap `<use>` count and depth; keep the slot busy until the decode returns |
| Medium | S4 | With a proxy configured, `network:fetch`'s DNS-rebinding check is moot: the proxy resolves the name again. Proxy credentials in the URL are sent, against decision 23. | W2c tip, `ui_ext_wasm/src/network.rs:325-336` (`b3822c6fce`) | When proxied, CONNECT to the checked IP and keep the name for SNI and Host, or refuse proxied domain fetches. Refuse proxy URLs with credentials. |
| Med-Low | S5 | Consent survives uninstall and is not tied to the component: there is no `remove`, `Grant::Component.sha256` is never compared, and a DevFolder record satisfies a Component load. A new "foo" inherits the old "foo"'s `network:fetch`. (This is R9's consent-hash item, from the load side.) | `ui_ext_host/src/consent.rs:322-344`; `install_store.rs:380` | Clear records and T2 answers on uninstall; reset them when the source or hash changes |
| Low | S6 | The `process:spawn` environment filter misses variables that run other programs: `GIT_EXTERNAL_DIFF`, `GIT_PAGER`, `PAGER`, `EDITOR`, `GIT_DIR`, `GIT_CONFIG`, `JAVA_TOOL_OPTIONS`, `DOTNET_STARTUP_HOOKS`, `PERL5LIB`, `NODE_PATH`, `RUSTC_WRAPPER`, `NPM_CONFIG_*`. A declared `PATH` reaches the child. An empty `args` pattern matches anything (`git -c alias.x=!cmd`). | `ui_ext/src/process.rs:32-46` | Refuse `PATH`, `PATHEXT` and `ComSpec`; filter the child's PATH; prefer an allowlist; an empty pattern matches no args |
| Low | S7 | Redirects re-send `Authorization`/`Cookie` across origins. | `network.rs:236` | Strip them on a cross-origin hop |
| Low | S8 | A symlinked file in an unscanned directory can be edited outside the worktree: `stays_inside` checks only the parent, and the buffer loads before the 16 MiB check. | W3 tip, `workspace_edit.rs:445-451` | `symlink_metadata` plus canonicalize when the file exists |

**Verified correct.** This list is a summary of a long check list; the parts are good.
- **Fetch:**
  - HTTPS only, with declared loopback ports as the exception; userinfo is refused;
  - IDNA and trailing dots are handled; wildcards exclude the apex and public suffixes;
  - every resolved address must be global (including NAT64, CGNAT and 169.254.169.254), and the
    addresses are pinned with `resolve_to_addrs`;
  - redirects are re-checked on each hop (5 at most) and never downgraded;
  - no environment proxy, no cookie store, and hop-by-hop and CR/LF headers are refused;
  - the caps hold (30 s, 16 MiB on the decoded body, 8 in flight).
- **Process:**
  - spawned by absolute path; worktree and relative PATH entries are skipped;
  - `.bat` and `.cmd` are refused;
  - the environment is cleared, then filtered;
  - the child starts suspended in a job object with `KILL_ON_JOB_CLOSE`; at most 4 processes;
  - trust is checked twice.
- **Consent:**
  - T2 exists only in the 0.3 world;
  - the check and the load use the same bytes;
  - revocation cancels fetches, kills processes, aborts edits and stops observe.
- **Edit:** untrusted worktrees are denied; one transaction per buffer; nothing is saved.
- **Observe:** strictly read-only; it cannot accept or reject.
- **Canvas:** caps and finite coordinates within ±32768.
- **Packages:** plain relative entry names, with size and entry caps.

### Renderer and lifecycle (W3, W2c)

| Sev | # | Finding | Where | Fix |
|---|---|---|---|---|
| High | H1 | Modals never tick: only panels call `set_visible`, so a modal's `PendingTick::due` stays `None`. A picker's refresh tick never fires, and no test covers it. | `surface.rs:842-843`; `panel.rs:156,262`; `modal.rs` | `set_visible(true)` on show and `false` on dismiss; add a test |
| High | H2 | The image LRU evicts images drawn this frame, so they miss on the next frame and are decoded again. With five 2048² images that loops forever. §12 says the fifth one shows its alt text. | `images.rs:85-100,264-266,382` | Never evict an entry used in the current frame; show the alt text when the budget is full |
| Medium | M1 | The remount bound can be defeated: an empty row answer, or a push to a sibling (even from a tick), resets `remounts_without_input`, so a bad tick update remounts forever every 100 ms. | `surface.rs:784-788,1271-1273`; `registry.rs:1785` | Reset only on a user event's applied reply |
| Medium | M2 | A failed decoration is removed only in the renderer. The host still counts it toward 512, and its notifications are orphaned, so after 3 the extension gets `limit-exceeded` until it reloads. | `decorations.rs:356-367`; `registry.rs:971`; `effects.rs:253-259` | A renderer→host "closed" channel, like `modal_closed`, also used for M4 |
| Medium | M4 | A modal that fails to open stays "open" with no view: `show()` runs before `toggle_modal`, which refuses. Every later `show-surface` is then a no-op. | `registry.rs:1036-1040,2562-2565` | Show the state only after the modal layer accepted the modal |
| Low | L1–L4 | <br>• `replace_backend` keeps the old instance's tick; <br>• `mount` overtakes a queued `dismissed`; <br>• the tick CPU share counts queue time; <br>• no PageUp/PageDown in modals (design C). | `surface.rs:272-311,526-538,699,897-899`; `modal.rs:125-144` | as listed |
| Low | — | The Inspector rebuilds up to 16k lines on every keystroke while open, and it is on in every debug build. Acceptable for a dev tool. | `inspector.rs:194,337` (W3 tip) | — |

**Architecture rules hold.** There are no WASM calls from render or paint (row fetches and image
decodes are scheduled, the canvas paints retained ops), theme tokens only (the canvas data palette
is a fixed host table, allowed by §13), one call per input event, and ticks never overtake events.

**Triage-3 W3 items, now resolved:**
- the backend epoch (ASYNC 2, BUG 3/4/10);
- load-failure visibility (UX F5, BUG 29/17);
- the crash message (UX F6);
- the call budget counted in epochs (ASYNC 5);
- quadratic `sync` and tree clones (PERF 5, 6);
- epoch wakeups and sequential compile (PERF 7, 8);
- ASYNC 7, 8 and 10;
- guest stderr and `data:` images (T5, T8).

**Partial:** PERF 1 (panels are cached and the caret no longer blinks, but any change still
re-renders the whole surface tree) and T6 (load errors are still logged unescaped,
`registry.rs:1345`).

**Not measured:** the 16384-op canvas paint bench (≤4 ms, plan 4.14). Clickable canvases have no
keyboard access.

**Build conflict.** W2c `85a7d3c2ed` makes `AgentReviews::subscribe` async, while W3 `f5a3b863fc`
implements the sync signature. Whichever merges second will not build, so W0 must merge them
together.

### Spec and design drift (W2c, W0)

- **Decision 28 lists more for 0.3 than ships.** Missing: decoration extras, picker, code lens,
  hover, virtual documents and commands. Only images and canvas ship (A3).
- **Decision 29 conflicts with Q9.** Decision 29 removes 0.1 "in 0.4", which never ships if 1.0 is
  0.3. Q9 is open and `MIN_VERSION` is still 0.1.0. **[USER]** **Recommended:** keep 0.1 and 0.2 loading
  through 1.x, and state removal as "two minor versions after 1.0".
- **`spec.md:196` drift.** It says `row-highlighted`; the WIT has `picker-source-event::highlighted`.
- **The design copy of the 0.3 WIT is stale.** `docs/fyx/wit-0.3/fyx-ui-0.3.wit` lacks
  `input-props.label` and holds the decided Q13 image rules that the shipped WIT lacks. Make one of
  them the source and generate the other.
- **Stale status lines** in `wit-0.2/design.md:40`, `wit-0.2/migration.md:15,44` and
  `wit-0.3/design.md:3`: "§M not landed", "no code changed".
- **Code comments behind the code.**
  - `fyx_ui_v0_3.rs:1-10` still says canvas is refused.
  - `ALL_UI_CAPABILITIES` lacks `ui:canvas` (`ui_ext_wasm.rs:229`).
  - The byte-for-byte check of the copied-forward WIT parts that `migration.md` §1 promises in
    `gate.sh` does not exist. A manual diff shows they match.
- **The TS SDK targets 0.1 only** (`sdks/ts/fyx-ui-sdk/src/fyx-ui-sdk.ts:23`). That is consistent
  with D.1 ("after the 1.0 freeze"), but if Q9 ends 0.1 support, the TS SDK has nothing to target.
- **These match the WIT:** the 2048/16384 caps, 64 pushes, the 100 ms tick floor (1 s in the
  background), the 5 s gesture window, the surface id rule, 512 decorations, 5 notifications per
  minute with 3 open, and the image and canvas caps. 0.1 and 0.2 keep their semantics through
  dispatch, and `docs/fyx/api/*.md` are generated and tested against the WIT.

## Upstream surface (`v1.28.1..integration`)

Canonical exclusions, as in integration-4, plus `crates/fyx_*` and the icon binaries:

| Measure | integration-4 | integration-5 |
|---|---|---|
| Files | 67 | 87 |
| Lines | +341/−215 | +1655/−255 |

Since integration-4: 99 files, +1486/−1738. Part of W1's string sweep was reverted, as
integration-4 decision 2 allowed.

**Most of the growth is new Fyx files inside upstream crates or assets.** These cannot conflict on
rebase:

| File | Lines |
|---|---|
| `crates/agent/src/tools/edit_session/fyx_review.rs` | 260 |
| `crates/agent/src/tools/fyx_file_operations.rs` | 221 |
| `crates/fs/src/fyx_junction.rs` | 159 |
| `crates/agent/src/fyx_models.rs` | 72 |
| keymap blocks in `assets/keymaps/default-{linux,macos,windows}.json`, marked | 46–49 each |
| `assets/fyx/**` icons | about 190 |

**Hand edits to upstream functions are small and marked:**
- `acp_thread.rs` +11/−3: the guard, `ensure_unchanged`, no format-on-save around rejected hunks,
  review notes in the next prompt;
- `agent.rs` +14/−2 and `thread.rs` +5 (models load on first use);
- `copilot_chat.rs` +26/−2 (no startup model fetch);
- `editor.rs` +8 (`disable_cursor_blink`);
- `commit_tooltip.rs` +9 (avatar setting);
- `thread_view.rs` +8 (the permission card defers to review);
- `move_path_tool`/`copy_path_tool` +10 each;
- `edit_session.rs` +7;
- `fs.rs` +11/−13;
- `language_models.rs` +5 and the LM Studio/llama.cpp probes;
- `settings_ui/page_data.rs` +39/−18;
- `zed/src/main.rs` +4/−25 (glue moved to `zed/fyx.rs`).

`status_bar.rs` needed no hook.

**Notes:**
- **Upstream PR.** `fs.rs` now creates junctions through `fyx_junction` (`DeviceIoControl`) instead
  of `cmd /C mklink /J` for **every** Zed caller. That is a real fix for upstream too (it is the
  integration-4 #2 injection class), so **[UP]** offer it to Zed. Until then it is the largest
  semantic change to an upstream crate.
- **Design scratch in `assets/`.** `assets/fyx/icon-concepts/` (`build_preview.py`, `a/b/c.svg`)
  are design work files. Move them to `docs/fyx/` or `script/`; `assets/` is for shipped
  resources.
- **[USER] `.claude/settings.json` is committed** (`0e817f1d0e`, user-approved). It grants Claude
  Code `acceptEdits` plus `Bash`, `Edit`, `Write` and `WebFetch`, with a push deny list, to anyone
  who clones Fyx and trusts the folder. That suits the sprint swarm. **Recommended:** move it to
  `.claude/settings.local.json` or the swarm's user settings before the repository goes public.
  `fyx_guard` treats `.claude/` as protected, so a public Fyx would ship a config its own shield
  calls dangerous.

## Decisions for the user, with recommended answers

| # | Decision | Recommended |
|---|---|---|
| 1 | Answer Q1–Q12 of the 0.3 design. Q3 (document context) and Q9 (version lifetime) are the hard-to-change ones (A1). | Answer them in one sitting, before any further 0.3 work. Take scope out rather than leave a question open. |
| 2 | What 0.3 is: build plan 4.2–4.4 first, or freeze 0.3 without picker, code lens, hover, virtual documents, commands and decoration extras, moving them to 0.4 (A3)? | Freeze the smaller 0.3 (images, canvas, T2 imports, notifications) and move the rest to 0.4. That starts the clock weeks earlier, and nothing declared is left unimplemented. |
| 3 | When the registry block list is checked, given "no request at startup" (R3) | When the Fyx tab opens, and 6 h after the last check, which is persisted across restarts. Never at startup. |
| 4 | Version lifetime (decision 29 vs Q9) | 0.1 and 0.2 load through all of 1.x. Removal is stated as "two minor versions after 1.0". |
| 5 | The committed `.claude/settings.json` before going public | Move it out of the repository (`settings.local.json` or user settings). |
