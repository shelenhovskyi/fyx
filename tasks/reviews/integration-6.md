# Judge review: integration-6 (plan 4.9, the final pass before the 0.3 freeze)

**Scope.**
- `origin/fyx/main` `9f7aa1c401`: the smaller 0.3, with A3 and A5 merged.
- The pending batch, read at its gate merge `ebafa3f573`, which contains all of it:
  - W2c `2252519174`: Q1 accept, T2 and `Release.zed_extension`; Q2 "runs any code";
  - W3 `80f0518006`;
  - W10 `b237bf1a2c`;
  - W4 `d9c47c0e14`.
- Baseline: integration-5 (`tasks/reviews/integration-5.md`, `2c827f977e`), which read integration
  up to `a3e640ed85`.
- Owner decisions: `docs/fyx/wit-0.3/open-questions.md`, recorded 2026-11-19.

**Method.** Read only. Nothing was built or run except `wasm-tools component wit` on the staged
0.3 WIT, which parses. W7 read the WIT, the 0.3 linker, the manifest gating and the spec. Two
reviewers re-checked every integration-5 finding and the owner decisions in the code. W7
re-read each new medium finding and each deviation from a decision at the cited lines before
listing it here. Q9 means 0.1 and 0.2 are dropped at the 1.0 cut, so this review asks for no
long-term compatibility work.

## Verdict: **GO for the clock, at W0's freeze commit, with two conditions**

The staged 0.3 ABI is complete and consistent for the smaller scope. Nothing from 0.4 is left in
it, and every interface it declares is linked. Every integration-5 A item is closed except A1,
which is the freeze commit itself. Everything still open is host behaviour, CI or docs. None of it
changes the WIT, so none of it resets the clock.

**Clock-start conditions:**
1. **W0, the freeze commit.** It contains:
   - the new WIT header (no "STAGED DRAFT");
   - `MAX_FROZEN_VERSION = 0.3.0`, stated explicitly;
   - the `permission-state.unavailable` doc fix (F1);
   - the stale design copy of the WIT removed or marked historical (F2).
2. **W2c and W3, the canvas benches.** Run them before the clock starts:
   - W2c: the lift bench of plan 4.13, at most 1 ms per 10 000 ops;
   - W3: the paint bench of plan 4.14, at most 4 ms for 16384 ops.

   Neither bench exists, and plan 4.9 depends on 4.13 and 4.14. A paint miss can be fixed in the
   renderer (Q14's texture fallback). A lift miss might mean a lower op cap, and the 16384 cap is
   in the WIT, so lowering it after the clock starts is a break. **Alternative:** the coordinator
   records that the 16384 cap stays whatever the benches show.

**Before the 1.0 cut** (fix while the clock runs): the must-fix list at the end, per owner.

## (1) Integration-5 A and B items

### A (the clock list)

| # | Status | Evidence |
|---|---|---|
| A1 | **Open; it is the freeze commit.** The owner answered the product questions on 2026-11-19. Q4, Q7 and Q14 are technical and the coordinator's; none of them changes the WIT (below). The header still reads "STAGED DRAFT, not frozen and not fully decided". | `crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit:1-4` |
| A2 | Closed. 0.2 is frozen at an explicit `0.2.0`, and the comment forbids using `MAX_VERSION`. | `ui_ext_wasm.rs:132-136` |
| A3 | Closed. The WIT has no `documents`, `language`, `virtual-documents` or `picker`, no decoration extras, and no provider, command or picker exports. All 10 declared interfaces are in `LINKED_INTERFACES`, so none traps. 0.3 keeps 0.2's `place-decoration`. | `fyx_ui_v0_3.rs:559-570`; WIT `:707-719` |
| A4 | Closed. `world_capabilities` refuses a capability the world lacks, with an error. The `network`, `processes` and `process` tables are accepted from 0.3 on only. `fyx-ext` mirrors the list exactly. | `ui_ext_wasm.rs:323-360,432-468`; `fyx_ext/src/manifest.rs:35-62` |
| A5 | Closed. <br>• `spawn-error` and `edit-error` have `other`. <br>• `truncated` means "hunks only", and `review-snapshot.older-dropped` covers dropped reviews (code agrees: `observe.rs:120-136`). <br>• The resync snapshot is documented. <br>• Buttons have `accessible-label`. <br>• The picker and code lens left with 0.4. | WIT `:174-183,850-856,906-912,968-985` |
| A6 | Closed. All of these are in doc comments: <br>• depth 64; <br>• 32768 pushed items; <br>• the 24 h tick delay; <br>• 512 decoration calls; <br>• 4 notify calls; <br>• `.idea` and the other protected paths at any depth; <br>• asset paths of at most 1024 bytes, `.png`/`.svg` only; <br>• `canvas-click` and `notification-action` as gestures. | WIT `:231,448,654,662,681-686,713,896,1091` |

### B (the 1.0 ship list)

| # | Status | Evidence, or what is left |
|---|---|---|
| R1 | Closed | The listing comes from `plan_install`, installs go through `download_for_install`, and a disabled button says "Blocked" or "Available in …" (`fyx_extensions_page.rs:331-353,676-686,960-977`) |
| R2 | Closed | `previous.effective_keyset()`; revocations carry over (`snapshot.rs:195,229-259`), test `a_revoked_snapshot_key_cannot_undo_its_revocation` |
| R3 | Closed as recommended | Applied on every refresh, last check saved across restarts. The block list itself is still fetched every 6 h. |
| R4 | Closed | A revoke removes the permission from `approved` (`registry.rs:418-445`) |
| R5 | Closed | One read (`store_actions.rs:358-386`) |
| R6 | Closed | `scripts/published.py` fails on any `gh` error; a missing document is allowed only with `--first-run` |
| R7 | **Partial** | Id, version and lists ⊆ the release (`install_store.rs:569-608`). Process `args` and `env` are not in the release, so they are not checked. This waits until T2 opens in the registry (coordinator, W2c.md:446). |
| R8 | **Partial** | The age gate counts from first sight (`update.rs:93-103`). `urgent` is still signed by the online key and skips the wait (`update.rs:98`). |
| R9 | **Partial** | Closed: rollback of a blocked version, `disabled` cleared, uninstall ids checked, dialog lists, ordered saves, Windows names. **Open** (W2c, `fyx_registry`): <br>• the block-list sequence is forgotten on a cache drop; <br>• an expired list is accepted with no cache; <br>• hash-only blocks miss installs with no recorded hash (`blocklist.rs:317-320,439-441`); <br>• role separation is not checked in clients (`signing.rs:188-196`), with no recorded deferral; <br>• "daily" vs 12 h (`blocklist.rs:11`, `registry-repo.md:496`). <br>Consent is not tied to the hash, by decision, but `extension-authoring.md:481-483` still promises it. |
| S1 | Closed | `redact_review`/`redact_event` through `is_private_path` (`agent_observe.rs:84-117`) |
| S2 | Closed | Any depth, any case, every mode (`ui_ext/src/edit.rs:148-158`, `protected.rs`, `workspace_edit.rs:447`) |
| S3 | Closed | `check_image_bytes` on assets, `prescan_svg`, and a timed-out decode keeps its slot (`images.rs:386-396,505-509,581`). The slot hold has no test. |
| S4 | Closed by refusal | Proxied domain fetches are refused, and proxies with credentials are never used. Real proxying is D.2, after 1.0. |
| S5 | Closed | `ConsentStore::forget` on uninstall; sources do not mix (`consent.rs:328-335`) |
| S6 | **Partial** | The listed names, `PATH`, `PATHEXT` and `COMSPEC` are forbidden, and the child's PATH is filtered. It is still a denylist: see N1. |
| S7, S8 | Closed | `network.rs:288-291`; `workspace_edit.rs:480-520` |
| H1, H2, M1 | Closed | `modal.rs:68-75`; `images.rs:101-120,451`; `surface.rs:1028`; each with a test |
| M2 | **Partial: not wired** | The renderer calls `registry.decoration_closed`, which calls `backend.decoration_closed`. `impl UiBackend for UiExtension` does not override it, so the trait's no-op default runs (`backend.rs:180`). In production a failed decoration still counts toward 512. The tests use a fake backend. |
| M4 | Closed | `modal_not_shown` (`registry.rs:1170,2816`) |
| L1–L4 | Closed | L1, L3 and L4 have no test |
| Build conflict | Resolved | `AgentReviews::subscribe` is async everywhere |
| Drift | Mostly closed | Spec 28 and 29 are updated, `ALL_UI_CAPABILITIES` has `ui:canvas`, and `spec.md:199` (`row-highlighted`) now sits under the moved items. **Open:** F2, F3, F4 below. |

**Not closed and not deferred:** R9's open items and M2. Everything else is closed, closed by an
explicit decision, or deferred (S4 to D.2; R7 and the Q2 review rule until T2 opens in the
registry).

## (2) The staged 0.3 WIT and model

**Complete for the smaller scope.**
- Images: `image-props` and `images.add-image`/`remove-image`.
- Canvas: the six draw ops, the data palette, `canvas-click` and `canvas-fit`.
- T2: `permissions`, `network`, `process`, `workspace-edit` and `agent-observe`, with `io` and
  `handle-io`.
- 0.2's notifications, unchanged.

The host links all of it. The manifest, `fyx-ext` and the harness agree on the per-world
capability lists.

**Nothing from 0.4 is left.** A grep for gutter, inline, namespace, swatch, orphaned, lens,
provide, picker-source and handle-command finds nothing. `types` and `host` match 0.2 except for
additions: the image and canvas types, `position`, `text-range`, `file-ref`, the three host-error
codes, and the doc comments. `notifications` is identical to 0.2.

**Inconsistencies.**
- **F1 (ABI doc, fix in the freeze commit).** The WIT gives two answers for an untrusted worktree,
  and the code follows the second.
  - `permission-state.unavailable` says "the active worktree is not trusted" (WIT `:735-736`).
  - `host-error-code.untrusted-worktree` says it is "Not a permission state" (`:636-638`).
  - The broker returns `unavailable` only when the manifest lacks the capability
    (`consent.rs:162-178`), and calls return `untrusted-worktree`.
  - **Fix:** drop the worktree clause from `unavailable`.
- **F2 (docs).** The design copy has not been updated for the owner's decisions. It still holds
  the full draft (picker, `document-id`, `decoration-target`, `orphaned`) and says it "moves to
  crates/… once the owner has answered". `design.md:3` still says "Nothing is decided and no code
  has changed", and `migration.md:3` says "Nothing here has been implemented". 0.2 solved this by
  deleting its docs copy (`wit-0.2/migration.md:15`). Do the same: one source.
  (`docs/fyx/wit-0.3/fyx-ui-0.3.wit:1-3`)
- **F3 (comments).**
  - The `fyx_ui_v0_3.rs:1-12` module doc still speaks of unbuilt interfaces linked as traps and
    of refused `picker` surfaces.
  - `permissions.request`'s error text says "a command or an accepted picker item" and omits
    `canvas-click` and `notification-action` (`fyx_ui_v0_3.rs:251-254`).
  - Once frozen, `define_unbuilt_as_traps` has nothing left to stub for 0.3. Keep it only if a
    later staged world needs it.
- **F4 (header rule).** The header says `types` is copied "byte for byte except where a comment
  says 0.3". `position`, `text-range` and `file-ref` carry no such comment (WIT `:579-600`). The
  `gate.sh` check that `migration.md` §1 promised still does not exist; a manual diff matches.

**The technical questions (the coordinator's to decide).** None changes the WIT, so none blocks
the clock. Record them anyway:
- **Q4.** The loopback part is built. `private_hosts` is an additive setting.
- **Q7.** 256 MiB for every world is already in the code.
- **Q14.** Renderer only. It needs the paint bench (condition 2).

## (3) The owner decisions in the code

| Q | Behaves as decided? | Evidence, and the gap |
|---|---|---|
| Q1 | **Yes, with one hole.** | The package is detected at any depth and in any case, with trailing dots and spaces ignored, in Rust, Python and `pr.yml`. `zed_extension` requires T2 (`index.rs:265`). An undeclared one is refused at download (`index.rs:318`). The warning shows in CI, the file, dev and registry dialogs, and the consent card. <br>**Hole (N3):** the trusted decision sees only the source file list (`review.yml:141-143`), and `publish.yml` never re-checks the tier. <br>**Low:** a dev folder is checked at the root only (`install.rs:345`). An update of an approved T2 install shows no dialog, so the warning shows only on first install (`fyx_extensions_page.rs:648`). |
| Q2 | **Yes.** | `RUNS_ANY_CODE` and `runs_any_code` feed "can run any program on your computer" in all three prompts. Tests: `interpreters_and_shells_run_any_code`, `test_programs_that_run_any_code_are_named`. <br>**Gaps:** <br>• versioned names miss (`python3.12`, `node22`); <br>• `wsl`, `npm`/`pnpm`/`yarn`, `cargo`, `make`, `uv`/`uvx`, `ssh` and `rundll32` are not listed; <br>• `git config`, `bisect run` and `submodule foreach` are not seen; <br>• N1 (a declared `HOME`) makes plain `git` run anything with no warning. <br>The registry rule (args patterns or a reason) is deferred until T2 opens in the registry. |
| Q5 | **Yes, but noisier than decided.** | No gesture; one transaction per buffer; never saved; fixed protected paths. The Show/Undo card shows for **every** edit, including one open file (`workspace_edit.rs:652` skips only 0 files). The decision was multi-file or unopened files only. No test covers the card. |
| Q8 | **Yes.** | Full hunks and comments for worktree files only; private files are redacted in both the snapshot and the stream (`agent_observe.rs:142,149,253,278-285`). No end-to-end redaction test. |
| Q9 | **Yes; nothing is due yet.** | `MIN_VERSION = 0.1.0` until the cut. It is documented in spec 29, plan 4.9 and W2c.md:470. Prepare the cut before 1.0: <br>• the demos still default to 0.1 (W4.md:29); <br>• `sdks/ts` targets 0.1 only, so at the cut it has no world. It is deferred after 1.0 (plan.md:120), so keep it out of the 1.0 build. |
| Q10 | **No (N2).** | `ask_permission` calls `window.prompt(PromptLevel::Warning, …, ["Allow", "Don't Allow"])` (`registry.rs:2666-2672`). That is a modal dialog, it can be answered from the keyboard (Enter picks "Allow"), and it has no click guard. The decided form is non-modal, has a 1 s guard and takes no keyboard answer. The guarded card in `notifications.rs` exists, but its guard is 750 ms. No test. |

## New findings (no ABI change)

| Sev | # | Finding | Where | Owner |
|---|---|---|---|---|
| Medium | N1 | `HOME`, `USERPROFILE` and `XDG_CONFIG_HOME` can be declared and overridden by the guest. An extension with `workspace:edit` writes `tmp/.gitconfig` (not protected), sets `core.fsmonitor` or `core.pager`, then spawns `git status` with `HOME=<worktree>/tmp`, and git runs the command. `runs_any_code` ignores env, so the prompt does not say "any program". The same goes for `ZDOTDIR`, `GOFLAGS`, `CLASSPATH`, `LUA_INIT`, `GEM_*` and `GCONV_PATH`. **Fix:** forbid the home and config-dir names, or make `[fyx_ui.process].env` an allowlist. | `ui_ext/src/process.rs:26-95`; `ui_ext_wasm/src/process.rs:266-274` | W2c |
| Medium | N2 | Q10 is not implemented as decided (above). A modal Allow dialog with Enter as the default answer is the reflex-grant the decision was meant to prevent. | `ui_ext_host/src/registry.rs:2656-2688` | W3 |
| Medium | N3 | **A Zed `extension.wasm` that only the build makes can auto-merge.** <br>• The decision sees `git ls-tree` of the source. An `extension.wasm` that exists only as build output passes as T0 and may auto-merge, for example a gitignored `[fyx_ui].component = "extension.wasm"`. <br>• `publish.yml` then signs it T2 with `zed_extension` and never checks the approval. <br>• `pr.yml`'s warning does not fail the build. <br>**Fix:** the decision also reads `[fyx_ui].component`, and `publish.yml` refuses a T2 or `zed_extension` release that the decision did not rate T2 with the owner's approval. | `tools/fyx-extensions-template/.github/workflows/review.yml:141-143`, `publish.yml:60-80` | W10 (template) |
| Medium | N4 | **Dropping the cached block list re-enables every blocked extension.** <br>• `apply_plan` re-enables any entry with `disabled` set and no `Disable` action in the current plan. <br>• `plan_updates` emits no `Disable` when `blocklist()` is `None`. <br>• `recheck_blocklist` sets it to `None` when a rotation snapshot revokes the block-list key, which is exactly the leaked-key response. A cache that fails to restore does the same. <br>The next page open logs "enabled … again: no longer blocked" and loads it. **Fix:** re-enable only when a fresh, verified list is present and matches nothing. | `ui_ext_host/src/fyx_extensions_page.rs:552-561`; `fyx_registry/src/client.rs:424-435`; `update.rs:283-307` | W3 (re-enable), W2c (keep the sequence and refuse to treat "no list" as "nothing blocked") |
| Low | N5 | M2 is not wired in production (above): a one-line forward in `impl UiBackend for UiExtension`, plus a test through the real backend | `ui_ext_host/src/backend.rs:180,191-268` | W3 |

## Must-fix list per owner

**Before the clock starts:**
- **W0:**
  - the freeze commit (header, `MAX_FROZEN_VERSION = 0.3.0`, F1);
  - delete the design WIT copy or mark it historical (F2, or W10 does it);
  - record Q4, Q7 and Q14, and record the canvas-cap decision if the benches are skipped.
- **W2c:** the 4.13 lift bench.
- **W3:** the 4.14 paint bench.

**During the clock, before 1.0:**
- **W2c:**
  - N1;
  - N4's registry half;
  - R9's open `blocklist.rs` items, plus a recorded decision on client-side role separation;
  - R8 `urgent` (owner-signed, or "urgent" never skips the wait);
  - F3's error text;
  - Q2's versioned names and the missing runners.
- **W3:**
  - N2 (Q10: a non-modal card, a 1 s guard, no keyboard answer);
  - N4's re-enable half;
  - N5 (M2);
  - Q5's card only for multi-file or unopened-file edits;
  - Q1's warning on updates, and dev folders checked at any depth;
  - tests for L1, L3, L4, the S3 slot hold and the Q8 end-to-end redaction.
- **W10:**
  - N3;
  - F2 (if W0 hands it over);
  - stale status lines: `wit-0.3/design.md:3`, `migration.md:3`, `wit-0.2/design.md:40`,
    `wit-0.2/migration.md:15,44`;
  - `extension-authoring.md:481-483` (consent and the component hash);
  - "daily" in `registry-repo.md:496`.
- **W4:** before the 1.0 cut, the demos default to 0.3/1.0 (W4.md:29), and `sdks/ts` stays out of
  the 1.0 build until it is ported (Q9).

**Carried over from integration-5, still the user's:**
- `.claude/settings.json` is committed: move it out before going public.
- `assets/fyx/icon-concepts/` is still in `assets/`.
