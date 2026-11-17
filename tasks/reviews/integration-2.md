# Judge review: integration-2

Scope: `git diff 8d76a5dbf9...integration` at `9cdf8a61b5`.

| Stream | Commits | What landed |
|---|---|---|
| W2 | `ad2471c3a2`, `3b654de7ac`, `4a65a92c95`, `6981c6004d`, `d51d50f940` | Fixes for integration-1 findings, plus `load_from_dir` and the `[fyx_ui]` manifest |
| W4 | `dd09bb924d` | `SurfaceTree` checks the host's tree rules; the todo demo rebuilds its list only when rows change |
| W5 | `5973cb794f`, `01498ea51c` | Phase A: `agent_review` model and a write gate in `acp_thread` |
| W8 | `f6dbb0ed1f`, `5bfa0b899e`, `bea2f9a991` | VS Code adapter: `tools/vsix-convert`, `sidecars/vscode-host`, `docs/vscode-coverage.md` |

Nothing was compiled. W8's TypeScript was read by a separate reviewer; its findings are in the W8
section below.

## Verdict

Fixes:
- W2 fixed every integration-1 finding it owned except 10, 11 and F2.
- W4 fixed its share of 2 and 5.

Two problems need fixing now:
- **Finding 1 (W0):** the committed `Cargo.lock` is stale.
- **Finding 2 (W8):** the VS Code converter can write outside its output folder.

W8's JSON tree schema matches the Rust serde form exactly. See the W8 section.

W5's upstream hook is acceptable as it is. See finding 3, which answers the integrator's question.

## integration-1 findings: status

| # | Finding | Status |
|---|---|---|
| 1 | Budget not enforced while a guest blocks in WASI | **Resolved** (`3b654de7ac`). `with_budget` races each call, the instantiation and `surfaces()` against a wall-clock `Deadline` woken by the epoch thread (`fyx_ui.rs:315-361`). The probe test sleeps 1 s and checks the call is cut off (`tests.rs:403`). |
| 2 | A rejected update desyncs the SDK mirror | **Half resolved.** W4 now checks the container rule, the per-update cap and the total cap before committing (`dd09bb924d`). Still open for W3: re-mount when `apply_update` fails. The counter documents the remount ids (8–14). |
| 3 | No order between pushes and the reply | **Resolved** (`4a65a92c95`). `Reply<T> { pushed, value }`: apply `pushed` first. A probe test pushes an insert and replies with a child of it (`tests.rs:321`). |
| 4 | Row-local ids | Decision stands; pinned by `tests.rs`. W3 still has to route clicks inside a row as `row-click`. |
| 5 | Row refresh | **W2 and W4 parts resolved.** `apply_update` returns the touched ids as a `BTreeSet` (`6981c6004d`). The todo demo rebuilds only when `visible_rows` changed (`dd09bb924d`). W3's part (re-request the rows of touched list nodes) and W0's `invalidate-rows` for 0.2 are still open. |
| 6 | Input echo | Open (W3). |
| 7 | Nobody loads UI extensions from disk | **W2 part resolved** (`d51d50f940`). `UiExtension::load_from_dir` with `UiManifest` checks: the version against the component, a relative `component` path, and capabilities per surface kind. Still open: W3 has to call it. |
| 8 | No version section from the SDK | Resolved earlier (`a945b18fd9`). |
| 9 | `push-update` promises timers | Open (W0, 0.2). The WIT and SDK docs still say "timers". W2's own doc is correct now (`fyx_ui.rs:200`). |
| 10 | Pushed updates are unbounded | Open (W2). They moved into `UiState::pushes`, a `Vec` with no cap per call (`fyx_ui.rs:222`). |
| 11 | Mount capped at 2048 nodes, tree at 16384 | Open (W2). The SDK now enforces the same 2048 cap on `mount` (`ui_ext_sdk.rs`, `SurfaceTree::mount`), so the two sides at least agree. |
| 12 | Doc drift | Counter part resolved. Icon fallback still open for W3. |
| 13 | Fork-discipline nits | Open (W0). The `Cargo.toml` lines for `ui_ext*` and `agent_review` still have no marker. |
| F1 | Duplicate version section | **Resolved** (`ad2471c3a2`). The error names the duplicate, and the shared note now says not to add the static. |
| F2 | Import fallback untested | Open (W2). No test reaches `api_version_from_imports`. `tasks/progress/W2.md:100` still says the demo "has no `fyx:ui-api-version` section yet", and `:102` still mentions the probe's "explicit version section". |

## New findings, most severe first

### 1. High: the committed `Cargo.lock` is stale for `agent_review`, so `--locked` builds fail (W0)

`git show integration:Cargo.lock` lists `agent_review` with only `gpui`. However,
`crates/agent_review/Cargo.toml` (`5973cb794f`) now declares `acp_thread`,
`agent-client-protocol`, `anyhow`, `futures`, `language` and `log`, plus dev dependencies
`serde_json` and `uuid`. W5 left the lock change uncommitted for W0 (`tasks/progress/W5.md:78`),
and the merge did not add it.

`build_guest` in `crates/extension_host/src/wasm_host/fyx_ui/tests.rs:42-64` runs
`cargo build -p fyx_demo_counter --locked` in the workspace, which needs an up-to-date root lock.
On a fresh checkout of `integration`, that fails with "the lock file needs to be updated but
--locked was passed". That breaks `test_demo_counter_round_trip` and `test_load_from_dir`.
`script/gate.sh` passing on a tree whose lock cargo rewrote locally hides this.

The integrator is tracing a test crash. Rule this out first: run `git status Cargo.lock` in the
gate's checkout.

**Fix (W0):** commit the regenerated lock. Make the gate fail when `Cargo.lock` is dirty after the
run, for example by checking that `git diff --exit-code Cargo.lock` is clean.

### 2. High: `vsix-convert` writes outside `--out` when the manifest is crafted (W8)

`tools/vsix-convert/src/cli.ts:34` builds the output folder as `resolve(values.out ?? "out", result.id)`.
`result.id` is `${publisher}.${name}`, taken unchecked from the untrusted `package.json`
(`vsix.ts:140`).

A publisher such as `../../../Users/<user>/AppData/Roaming/Zed/x` makes the converter write
`keymap.json`, `extension.toml` and `themes/*.json` into that folder. The converter never runs
extension code, so converting a downloaded `.vsix` should be safe; this is a real escalation, from
"convert a file" to "overwrite editor config".

**Fix:**
- Name the folder from a sanitized slug, rejecting `/`, `\`, `:` and `..`.
- Assert that the resolved `outDir` and every written path stay under `resolve(out)`.
- The `..` check in `zip.ts:24` splits on `/` only, so it misses `\` and drive-letter names. Fix it
  in both copies.

I verified this finding myself, in addition to the W8 reviewer.

### 3. Low: `crates/acp_thread/src/fyx_review.rs` is in the right place (answer to W0)

The question was whether these 75 lines should live in `agent_review`, with only a hook in
`acp_thread`. **No, keep them where they are.**

Moving them would create a dependency cycle. `agent_review` must depend on `acp_thread` for the
permission-preview path: it uses `AcpThread`, `AcpThreadEvent`, `PermissionRequestId`,
`ToolCallContent` and `SelectedPermissionOutcome` (`agent_review.rs:12-15`). So the trait that
`acp_thread` calls cannot live in `agent_review`.

The file is already the minimal hook: a trait, two structs, a `Global` and a 25-line glue
function. When no gate is installed, the behaviour is byte-for-byte upstream's
(`fyx_review.rs:57-59`). A new file in an upstream crate never conflicts on rebase. The rebase
surface is the 8 marked lines in `acp_thread.rs` (`:9`, `:8017-8019`, `:8089-8090`), which matches
the W2 `fyx_ui` module pattern.

There is one alternative: a new leaf Fyx crate (`agent_review_gate`: `gpui`, `language`,
`agent-client-protocol`) that `acp_thread` depends on. It would move the 75 lines out of the
upstream crate, but it costs a crate plus an upstream `Cargo.toml` line, which is a more
conflict-prone edit than a new file. It is not worth it.

For the diff metric, count `fyx_review.rs` the way `wasm_host/fyx_ui*` is counted: as Fyx-owned
code inside an upstream crate (see the table below).

### 4. Medium: a partly accepted permission preview is answered `Allow` (W5)

`permission_verdict` (`agent_review.rs:359-364`) turns anything except `RejectedAll` into
`AllowOnce`. The rejected hunks are enforced only if the agent then writes through
`fs/write_text_file`, where `review_write` adopts the preview's decisions (`agent_review.rs:322-338`).

An agent that writes to disk itself is told "allowed", and the hunks the user rejected land. W5
lists this bypass as not covered yet (`tasks/progress/W5.md:82`), but it is worse than an
unreviewed write: the UI showed the user that a hunk was rejected. ARCHITECTURE.md says "policy
and consent stay in the host".

**Fix (W5):** for a `Partial` preview, answer `RejectOnce` and send feedback naming the accepted
hunks, so the agent re-proposes only those. Alternatively, keep `Allow` only for sessions whose
agent is known to write through the client's fs, and show "rejections are enforced only for writes
made through Fyx" in the review UI.

### 5. Low: an agent gets an error for a write that landed (W5)

When there are comments, or a partial accept, `acp_thread.rs:8090` returns
`Err(feedback)` after the buffer was edited and saved (`fyx_review.rs:28-30`). With `AcceptedAll`
and comments, the agent sees a failed tool call for an edit that fully applied. Many agents retry a
failed write, which would open a second review of the same text.

The decision belongs to W5. Either word the feedback so that retrying is plainly wrong, which it
already does for `Partial` but not for `AcceptedAll` with comments, or deliver comments on a channel
other than the error.

### 6. Low: a held write outlives its thread (W5)

Noted by W5 (`tasks/progress/W5.md:84`). When the `write_text_file` task is dropped, its oneshot
receiver goes away. The `Review` stays in `reviews` until someone calls `close`, and `submit` then
only logs at debug level (`agent_review.rs:191`).

**Fix (phase B):** close the reviews of a session when its `AcpThread` is released, for example
with `cx.observe_release` in `init`.

### 7. Low: pushes from a failed call reach the channel out of order (W2)

`guest_result` and `reply` (`fyx_ui.rs:634-665`) forward the pushes of a call that returned a
guest error to the unbounded channel. A later call's `Reply.pushed` can be applied before the
renderer drains that channel, which inverts the order that finding 3 just fixed.

Rare: it needs a guest that pushes and then returns `Err`. **Fix:** return the pushes inside the
error, for example `Err(GuestError { message, pushed })`. W3 then applies them before showing the
error.

### 8. Low: SDK rule checks are a fourth copy of the host's rules (W4)

`dd09bb924d` adds `TreeLimits`, `kind_name` and `is_container` to the SDK. Each is a copy of the
same item in `ui_ext` (`tree.rs:9-23`, `types.rs:135-153`), and the 2048 and 16384 caps are
hard-coded in both places. If W2 changes a cap, the SDK silently disagrees again.

**Fix (W4, with W0 for the dependency):** take the constants from `ui_ext`. It is Apache-2.0,
has no wasmtime or GPUI dependency, and builds for `wasm32`. At minimum, add a test that asserts
`ui_ext_sdk::TreeLimits::default()` equals `ui_ext::TreeLimits::default()`.

`list_row` still does not check that a row has exactly one root (`ui_ext_sdk.rs`, `list_row`).
The host rejects the whole `list-rows` batch when a row breaks that rule.

## W8: VS Code adapter

Read by a separate reviewer. Finding 2, W8-1 and W8-2 were confirmed against the code here; the rest are the reviewer's reading.

**Verified correct:**
- `sidecars/vscode-host/src/schema.ts` matches the serde types in `crates/ui_ext/src/types.rs`
  exactly:
  - Field names: `row-count`, `default-size`, `old-text`.
  - Enum spellings, and arrays for `set-props`, `input-changed` and `row-click`.
  - Bare `"unchanged"`, `{"markdown": "..."}`, and options always present as `null`.
  - RPC params `surface-id` and `list-node`.
- `ui/theme.ts` maps only to the 9 colour tokens.
- Producers parent nodes only under `flex`, never reuse ids, and keep `before` within the same
  parent's children.
- The sidecar opens no network connections at startup.
- Spot-checked coverage claims hold: `commands` 4/4, `l10n`/`extensions` 3/3, and error `-32001`
  with `data.api` for an unimplemented API (`host.ts:255`).

Findings (plus finding 2 above):

- **W8-1, Medium: zip bombs.** Both `zip.ts` copies call `inflateRawSync(raw)` at line 32 with no
  `maxOutputLength`, and inflate every entry up front. The 25 MB cap in `fetch-vsix.ts` covers the
  compressed size only, and the converter accepts any local file. **Fix:** check the uncompressed
  size in the central directory against a per-entry cap and a total cap, pass `maxOutputLength`,
  and inflate lazily.
- **W8-2, Medium: the unpack cache is trusted blindly.** `unpackVsix` (`host.ts:44-48`) reuses
  `<cacheDir>/<sha16>/` whenever it contains `package.json`. The default `cacheDir` is under the
  temp folder (`host.ts:106`).
  - On Linux and macOS the temp folder is shared between users, so another local user can pre-seed
    the folder for a public `.vsix` and get code executed.
  - An unpack that crashed after writing `package.json` is later reused as if complete.

  **Fix:** use a per-user `0700` folder, unpack into a temporary folder and rename it into place,
  and write a completion marker.
- **W8-3, Medium: the sidecar's mirror drifts after a rejected update.** This is the same issue as
  integration-1 finding 2, on the JSON side. `ui/surfaces.ts:187-189` records a tree as mounted
  before the host has accepted it. `push-update` is a notification, so when the host rejects an
  update, every later diff is computed against a tree the host does not have. `diff()`
  (`schema.ts:315-347`) also ignores the 2048 cap. **Fix:** make `push-update` a request and send
  a full `replace` after an error; fall back to `replace` above the cap.
- **W8-4, Medium: edits are applied locally before the host confirms.** `shim/editors.ts:279` and
  `shim/workspace.ts:134` apply an edit locally and never roll it back. When the host answers
  `false`, the extension keeps seeing text the document does not have. Together with
  `documents.ts:266`, which ignores host versions lower than the local one, a real user edit can be
  dropped.
- **W8-5, Medium: `InputBox` problems.**
  - Setting `validationMessage` does not re-render (`ui/quickpick.ts:234`, `:316`).
  - `password` is ignored (`:232`), so typed secrets are shown in plain text in an `input` node.
    Until 0.2 adds a masked input, refuse `password: true` with an explicit error. Masking is a
    0.2 candidate for the `input-props` record (W0).
- **W8-6, Low: silent no-ops**, despite the claim that unimplemented APIs throw:
  - `createFileSystemWatcher` returns a watcher whose events never fire (`workspace.ts:204`).
  - `setLanguageConfiguration` is stored but not applied (`api.ts:285`).
  - `registerTreeDataProvider().dispose()` does nothing (`api.ts:211`).
  - TreeView: `reveal`, `title`, `description` and `badge` do nothing or never re-render
    (`treeview.ts:189-229`).
  - QuickPick: `buttons`; `busy` and `title` after `show()` (`quickpick.ts:46-53`).
  - `withProgress` cancellation never fires (`api.ts:221`).

  Either implement them or throw, and list them in `docs/vscode-coverage.md`. `FYX_VSCODE_PROBE`
  (`api.ts:399`) makes every missing API an inert stub. Allow it only when the coverage runner sets
  it.
- **W8-7, Low: the mock host misses one cap.** It does not apply the 2048 per-update cap to
  `replace`, `mount` or list rows (`schema.ts:279`). The coverage report's claim that the mock
  enforces 2048 per update (`docs/vscode-coverage.md:106-107`) is therefore only partly true.
- **W8-8, Low: TreeView row limits.** The 10 000-row cap in `treeview.ts:119` exits only the
  current level, so parent levels keep adding rows. A deep enough tree overflows the `u16` padding
  (`:143`), and the whole `list-rows` reply then fails to deserialize on the Rust side.
- **W8-9, Low: the sidecar does not enforce its sandbox.** Node's `--permission` flags are applied
  only by `MockHost.spawn`. `main.ts` never checks `process.permission`, so a launcher that forgets
  the flags runs extensions without a sandbox. **Fix:** refuse to start when `process.permission`
  is missing (W8). Whoever launches the sidecar from the Rust host must pass the flags (phase 3).
- **W8-10, Low: row ids follow a different convention.** Rows start at 1 000 000
  (`treeview.ts:17`, `quickpick.ts:21`) rather than the SDK's `0x8000_0000`. The host allows it,
  but use the same base, so integration-1 finding 4's rule ("row ids are row-local") reads the same
  everywhere.
- **Duplication.** The two `zip.ts` files (`tools/vsix-convert/src/`, `sidecars/vscode-host/src/`)
  are identical apart from a comment, so the fixes for finding 2 and W8-1 have to land twice. Move
  the file to one shared module, or copy it with a check that fails when the copies differ.
- **Ownership.** W8 wrote `docs/vscode-coverage.md` inside W7's `docs/`. Its task file names that
  path, so this is sanctioned; no action needed.

## Upstream diff (`v1.28.1..integration`)

Measured with
`git diff v1.28.1 integration --stat -- . ':!crates/ui_ext*' ':!crates/agent_review' ':!docs' ':!tasks' ':!extensions/fyx_*' ':!crates/extension_host/src/wasm_host/fyx_ui*' ':!tools/vsix-convert' ':!sidecars' ':!*.md' ':!FYX_UPSTREAM' ':!script/gate.sh' ':!script/fyx-check-licenses-index'`.

| Kind | Files | Lines |
|---|---|---|
| **Edits to upstream files** | `Cargo.toml` (+10), `.cargo/config.toml` (+8), `crates/acp_thread/src/acp_thread.rs` (+7/−1), `crates/extension_host/Cargo.toml` (+1), `crates/extension_host/src/wasm_host.rs` (+1) | **+27/−1** (integration-1: +20) |
| Fyx files inside upstream crates | `crates/acp_thread/src/fyx_review.rs` (75), `crates/extension_host/src/wasm_host/fyx_ui*` | new files, no rebase conflicts |
| Generated | `Cargo.lock` | +46, stale (see finding 1) |

`git grep -n 'fyx:'` lists every hand-edited spot except the `Cargo.toml` members (integration-1,
finding 13).
