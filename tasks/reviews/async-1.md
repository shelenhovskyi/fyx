# Review async-1: concurrency, async and GPUI lifecycle

Scope: `crates/ui_ext_wasm`, `crates/ui_ext_host`, `crates/agent_review`, the `fyx:` hooks in
`crates/acp_thread` (`fyx_review.rs`, `write_text_file`) and `crates/zed/src/zed/fyx.rs`, plus
`crates/fyx_guard` from `claude/fyx-w9-guard` (6775a877a5, not merged). Base: `fyx/main` at
571c6826a5. No product code was changed. File:line references are to that commit.

Method: read every `spawn`, `detach`, `subscribe`/`observe`, `on_release`, `Task` field, channel and
`WeakEntity` in the Fyx crates, and followed each through Zed's rules (`crates/gpui`: a dropped
`Task` is cancelled; `observe`/`subscribe` callbacks are removed lazily, the next time they fire
after the observer is gone, `invoke_observer_in` in `gpui/src/app/context.rs:893`; `on_release`
runs at effect flush, never inside another entity's update). Five findings are confirmed with GPUI
tests (`TestAppContext`, `run_until_parked`) on a local scratch branch; the tests are pasted in the
appendix. The rest are by inspection, and are marked so.

Other reviewers cover performance, logic, security, tests and UX. Where a finding crosses into one
of those lanes, the report says so in one line.

## Summary

| # | Severity | Area | Finding | Evidence |
|---|---|---|---|---|
| 1 | High | agent_review | Any new thread on the project applies a pending rules change without confirmation, and an agent can start one (a subagent) | test A3 |
| 2 | Medium | ui_ext_host | Hot reload: calls still in flight on the replaced instance are applied to the new one. A stale timeout stops the reloaded extension, and stale replies patch the new tree | tests U1, U2 |
| 3 | Medium | agent_review | A permission review is closed while its host write is in flight. The write still lands, the request is answered twice, and the turn summary leaves it out | test A2 |
| 4 | Medium | acp_thread hook | Held write: the "changed while under review" check is not atomic with the edit. User typing or a reload during the diff await is merged into a reviewed write | inspection |
| 5 | Medium | ui_ext_wasm | The time budget is wall-clock and includes time the call is not scheduled on the shared 2-worker tokio runtime, so contention disables healthy extensions | inspection |
| 6 | Low/Medium | agent_review | A held write whose request was dropped keeps its review open. Submitting it records the edit as landed | test A1 |
| 7 | Low | ui_ext_host | Failed and uninstalled surfaces keep their wasm instance (Store, queue task) alive until restart | inspection |
| 8 | Low | ui_ext_host | A closed workspace's status items and views live on in the registry observer until the next registry notify | inspection |
| 9 | Low | ui_ext_host | Dev-path hot reloads are not serialized, so an older build can replace a newer one | inspection |
| 10 | Low | ui_ext_host | The per-surface event queue has no bound and no back-pressure | inspection |
| 11 | Low | agent_review | Closing a window or quitting with held reviews asks nothing. Decisions and comments are discarded (nothing lands) | inspection |
| 12 | Low | ui_ext_host | Typing still inside its 150 ms debounce is lost when the window closes | inspection |
| 13 | Info | gpui_tokio | The only production `gpui_tokio::init` is `main.rs:495`; no double init. The latent effect is described | inspection |
| 14 | Info | all | Synchronous work on the main thread: diffs, TOML, `is_dir`, `block_on` | inspection |

Checked and clean: listed at the end.

---

## 1. High: a new thread applies a pending rules change without confirmation (agent-triggerable)

**Where.** `crates/agent_review/src/agent_review.rs:137-146` (`observe_new::<AcpThread>` →
`reload_rules(.., RulesLoad::ThreadStarted, ..)`), `:1398-1402` (`ThreadStarted` →
`set_rules`), `:545-550` (`set_rules` removes `pending_rules[root]`).

**Interleaving.**
1. Session S1 runs. Its agent rewrites `.fyx/review.toml` with a shell command (not reviewed),
   for example `auto_accept = ["**"]`, or deletes it.
2. `on_project_event` → `reload_rules(FileChanged)` → `propose_rules`. The change is pending and
   the toast "Apply new review rules" is shown. The old rules are still in force, as intended.
3. Before the user answers, **any** new `AcpThread` on the project is created: a second session,
   a reload of S1, or a subagent. The agent can cause the subagent case itself by sending a
   `subagent_update` session update: `agent_servers/src/acp.rs:799-846`
   (`apply_subagent_update`) does `cx.new(|cx| AcpThread::new(..))` for it.
4. `observe_new` → `reload_rules(ThreadStarted)` reads the file as it is now and calls
   `set_rules`, which applies it **and removes the pending confirmation**.

**Effect.** The rules the agent wrote take effect with no user action. Every later write in
`src/**` is auto-accepted, or the `deny` rules disappear. The toast stays on screen; clicking it
later does nothing (`apply_pending_rules` finds nothing pending), so the user still believes the old
rules apply. Confirmed by test A3. (Security lane: this is the policy bypass. It is reported here
because the cause is a lifecycle race: thread start against a pending change.)

A second race of the same kind: each `reload_rules` call spawns its own detached load (`:1378`)
with no generation, so two loads of one root (two threads starting, or `FileChanged` racing
`ThreadStarted`) finish in any order, and the last one wins.

**Fix sketch.** Load rules per worktree, not per thread. Apply directly only on the first load of a
root (`self.rules` has no entry for it and nothing is pending). After that, every load goes through
`propose_rules`, and `ThreadStarted` never clears `pending_rules`. Give each root a load generation,
as `fyx_guard::load_policy` does (`next_generation`), and drop a result whose generation is stale.

## 2. Medium: hot reload applies stale calls from the replaced instance to the new one

**Where.** `crates/ui_ext_host/src/surface.rs:213-223` (`replace_backend`: swaps `backend`,
clears `events`, remounts; nothing marks calls already in flight), the event worker
`:299-323`, `apply_reply` `:404-433`, `fetch_rows`/`receive_rows` `:613-650`, and
`fail_extension` `:248-255` → `RegistryLink::extension_failed` (`registry.rs:556-562`).

**Interleaving.**
1. A click is in flight on instance A. `backend.handle_event` cloned A into the future
   (`backend.rs:44-46`), so A stays alive.
2. The component file changes and the watch reloads it (`registry.rs:381-403`).
   `register_backend` → `replace_backend(B)`. B mounts, and the surface is `Ready(tree from B)`.
3. A's call finishes. Two cases:
   - It fails without a `ReplyError`, for example A's time budget (A was busy, which is why the
     developer rebuilt it). `apply_reply` → `fail_extension` → every surface of the extension
     becomes `Failed { retryable: false }`, including those now on the healthy B. No Retry button
     appears; only another reload brings them back.
   - It succeeds. Its `Update` was computed against A's mirror and is applied to B's tree. Ids
     from the SDK's counter usually collide, so this patches the wrong nodes without an error, or
     triggers a resync. Pushes in a stale `list-rows` reply are applied the same way, before the
     generation check (`receive_rows:632-638` runs `apply_pushes` first).
4. Also, `event_worker_running` stays true across the swap, so the first click on B waits until A's
   call returns (up to budget + slack, 250 ms). This is harmless on its own, but it is the same
   missing epoch.

**Effect.** After a reload, the extension can stop for good, or show state that B never produced.
Confirmed by tests U1 (both sibling surfaces stop after the stale timeout; B mounted and never
failed) and U2 (B's label holds the text A's late reply wrote). The same path runs on every
reinstall through `scan_installed`, not only on dev hot reload.

**Fix sketch.** Give `SurfaceState` a `backend_generation: u64`, bumped in `replace_backend`. The
event worker, `fetch_rows` and `mount` each capture it with their call. On return, if it differs,
drop the result: apply no pushes and call no `fail_extension`. The worker then continues with the
next event on the current backend. Option: in `replace_backend`, drop the old worker's handle and
start a new worker, so B is not blocked behind A.

## 3. Medium: a permission review is closed while its host write is in flight

**Where.** `submit` sets `applying = true` and spawns the responder (`agent_review.rs:657-673`).
Three paths close reviews and skip only `is_submitted()` (responder gone); none of them checks
`applying`:
- `permission_answered` `:839-851` (`ToolAuthorizationReceived`, `:1041-1043`),
- `pane_closed` `:467-486` through `open_reviews_in_turn` `:341-345`,
- `close_session` `:712-723` (`Stopped`/`Error`/`ConnectionLost`, `:1052-1057`).

`close` `:726-733` removes the review whatever its state.

**Interleaving.** The user submits a partly accepted preview (`ApplyAcceptedAndDeny`).
`make_responder` (`:1124-1147`) awaits `open_buffer` and then `save_buffer` for the accepted
hunks. While that is in flight, one of these happens:
- the user also answers the request in the agent panel (both UIs are live),
- the user closes the review pane, or its window,
- the turn stops.

The review is removed. The host write continues: it lands the accepted hunks and then calls
`authorize_permission_request` / `select_permission_option` on a request that is already answered
or cancelled. `finish_permission` (`:688-690`) finds no review and returns before `record`.

**Effect.** Hunks land on disk after the user allowed or denied the whole edit elsewhere, or after
the agent stopped. For example, the user clicks "Allow" in the panel: the agent writes the full
file, and the host writes the partial version on top or in between. The permission is answered
twice. The turn summary omits a file the host did write. The pane shows the review as gone while
the write is still running. Confirmed by test A2 (`permission_answered` during the write; the
write completes; the history is empty).

**Fix sketch.** Treat `applying` as in flight. `permission_answered`, `pane_closed` and
`close_session` leave an applying review alone; `finish_permission` closes it afterwards. Better
still, make the responder check, right before it writes, that the request is still pending:
`thread.permission_request(request_id).is_some()`, as the v2 preview path already does at
`:1017-1021`. Then answering in the panel wins cleanly.

## 4. Medium: held write, the post-review check is not atomic with the edit

**Where.** `crates/acp_thread/src/fyx_review.rs:80-107` checks deletion, move and
`buffer.version().changed_since(&version_before)`, then returns. The edit happens later in
`acp_thread.rs:8020-8058`: first `cx.background_executor().spawn(text_diff ..).await` (`:8020-8031`),
then `buffer.edit(edits)` (`:8055`), then format-on-save `.await` and `save_buffer` (`:8070-8089`).

**Interleaving.** The user submits the review. The check passes. While the diff runs on the
background executor, the foreground keeps running: the user types in the buffer, or the file is
reloaded from disk (`git checkout`), or it is deleted. The edits are anchored in the agent's
`snapshot`, so `buffer.edit` merges them into the changed buffer without a conflict, and
`save_buffer` then writes the user's in-flight typing too. A deleted file is recreated. The window
is as long as `text_diff` takes; for a large file that is hundreds of milliseconds. The
format-on-save await that follows widens it for the save (that part is upstream behaviour).

**Effect.** The guarantee the hook documents ("if it changed meanwhile … writing them would land
changes nobody reviewed") has a gap exactly where the user is most active: right after clicking
Submit, while focus returns to the editor. Logic lane: w5-deep-1 M4 covers the stale base before
the review; this is the window after it.

**Fix sketch.** Compute the diff first, then do the version, deletion and move checks in the same
foreground `cx.update` that calls `buffer.edit`. That is, move the second half of
`gate_agent_write` into the update at `acp_thread.rs:8048`, or have the gate return a closure
that the hook runs there. After `buffer.edit`, also skip the save when
`buffer.version()` gained edits other than the agent's.

## 5. Medium: the time budget counts time the call is not running

**Where.** `crates/ui_ext_wasm/src/ui_ext_wasm.rs:601-609` (the epoch callback interrupts when
`call_started.elapsed() > call_budget`, a wall clock), `:509-517` (`with_budget`: a `Deadline`
on the same wall clock), `:846-857` (any error disables the instance). Every UI component's queue
is a task on the single `gpui_tokio` runtime: `gpui_tokio.rs:13-16`, `worker_threads(2)`, shared
with every other Zed user of `gpui_tokio`.

**Interleaving.** Both checks run only when the call's task is polled. The guest yields every epoch
(`UpdateDeadline::Yield(1)`). When the tokio workers are busy, a yielded call is not polled again
for a while; when it is, `elapsed()` is already past 200 ms, and the next epoch callback interrupts
it, or `Deadline` wins the `select`. Example: three UI extensions each run a 120 ms handler at the
same time on two workers, or another tokio task blocks a worker. Each call takes over 200 ms of
wall time with under 200 ms of its own execution. All three are disabled for good, with the
misleading message "exceeded its time budget … while blocked in a host call".

**Effect.** Healthy extensions are disabled under load, with no retry (`retryable: false`). This is
the race the task names: epoch interruption and time budgets against event delivery. (Perf lane:
UI calls also take tokio workers from other users.)

**Fix sketch.** Count execution, not wall time. In the epoch callback, add one `UI_EPOCH_INTERVAL`
to a per-call counter each time it runs, since it only runs while the guest executes, and interrupt
on the counter. Keep the wall-clock `Deadline` only for "blocked in a host call", start it from the
last time the guest yielded, and make it generous (seconds). Or give UI components their own small
tokio runtime, so other users cannot starve them.

## 6. Low/Medium: a cancelled held write keeps its review, and submitting it records a landed edit

**Where.** `agent_review.rs:917-931` (`review_write` keeps the `oneshot::Sender` in the review and
returns a task that awaits the receiver), `:643-656` (`submit`: `sender.send(..)` fails, is
logged at debug, and `record(..)` still runs).

**Interleaving.** The task that awaits the review is dropped without any event that
`close_session` handles. For example, the ACP request is cancelled or its handler future is
dropped, or `write_text_file`'s task is dropped by its caller. The receiver is gone, but the review
stays open and the pane keeps asking. The user decides and submits. `send` fails, and the turn
summary records `AcceptedAll` for a file nobody wrote.

**Effect.** The pane asks for decisions nobody waits for, and the turn summary is wrong. Confirmed
by test A1.

**Fix sketch.** For each held write, spawn a task on the review that awaits
`sender.cancellation()` (futures' `oneshot::Sender::cancellation`) and closes the review then. In
`submit`, record only when `send` succeeded.

## 7. Low: failed and uninstalled surfaces keep the wasm instance alive

**Where.** `surface.rs:237-244` (`fail` keeps `backend`), `registry.rs:434-445` (uninstall only
fails the surfaces; `LoadedSurface`s are never removed from `self.surfaces`), `ui_ext_wasm.rs:525-533`
(each `UiExtension` clone holds the queue task and, through it, the `Store`).

**Effect.** After an uninstall, a trap or a disabled budget, the instance stays alive until restart
or reinstall: its `Store` with up to `UI_MEMORY_LIMIT` (32 MiB) of linear memory, its WASI context
and its tokio task waiting on `rx`. A developer who reinstalls under a new id, or who keeps failing
during hot reload, piles these up.

**Fix sketch.** On `fail(.., retryable: false)`, and on uninstall, replace `backend` with a
`DeadBackend` that keeps only the extension id and fails every call. Remove uninstalled
`LoadedSurface`s from the registry, and their panels from the docks.

## 8. Low: a closed workspace's containers outlive it until the next registry notify

**Where.** `registry.rs:657-663` (`cx.observe_in(&registry, window, ..).detach()` captures
`surfaces: Rc<RefCell<WorkspaceSurfaces>>`), `:612` (`status_groups` holds the
`ExtensionStatusGroup` entities strongly; these hold `ExtensionStatusItem` → `SurfaceView` → input
editors, markdown entities and `DiffEditor`s).

**Effect.** GPUI drops a detached observer only when it next fires and finds its observer gone
(`gpui/src/app/context.rs:893-908`). The registry notifies only when an extension registers or
reloads. Until then, every closed window keeps its status bar group, status items and their
per-window editors. They also stay in `ExtensionViews`, so `flush_sibling_views` keeps upgrading
and updating them (`view.rs:131-145`). The leak is bounded and released at the next load.

**Fix sketch.** Store the `Subscription` in `WorkspaceSurfaces` (owned by the workspace through its
action closures). Or capture a `Weak` of `surfaces` in the observer.

## 9. Low: dev-path hot reloads are not serialized

**Where.** `registry.rs:408-423` (`Reload::DevPath` spawns a detached `load_dev_extension` each
time).

**Interleaving.** The developer saves twice while the first reload is still compiling. Reload #1
reads build N; reload #2 reads build N+1. The incremental compilation cache can make #2 finish
first, and then #1 replaces it with the older build.

**Effect.** After a rebuild, the extension runs stale code. Only dev paths (`FYX_UI_DEV_EXTENSIONS`)
are affected; installed dev links go through `scan_installed`, which is serialized and checks
fingerprints.

**Fix sketch.** Keep one reload task per id in `watches` (or next to it). A new change replaces
the task (dropping the old load) or sets a "load again" flag, the way `scan_installed` does.

## 10. Low: no bound or back-pressure on the per-surface event queue

**Where.** `surface.rs:293-296`. Only a trailing `input-changed` for the same node is coalesced
(`:278-286`). The wasm queue behind it is `mpsc::unbounded` (`ui_ext_wasm.rs:645`); it is bounded in
practice because each surface keeps at most one event call and one `list-rows` per list in flight.

**Effect.** A key held on a focused button (key repeat about 30/s), with a handler that takes
100 ms, grows `events` by about 20/s. Every queued click is still delivered, seconds after the
user stopped. Memory use is not a concern.

**Fix sketch.** Cap `events` (for example 64) and drop the oldest non-input events with a log
line, or coalesce repeated identical `click`s that are still queued.

## 11. Low: closing a window or quitting with held reviews asks nothing

**Where.** `review_pane.rs:273-277` (`on_release` → `pane_closed` closes the turn's open reviews),
`review_pane.rs:974-` (`impl Item for ReviewPane`: no `is_dirty`/`can_close`). No `on_app_quit` in
`agent_review`.

**Effect.** When the last pane of a turn closes, or the window or app closes, held writes fail with
"The review of this edit was closed; the file is unchanged." Permission previews stay unanswered,
and the user's decisions and comments are discarded with no prompt. This fails safe (nothing
lands); the cost is lost work. At quit, whether `on_release` runs at all depends on teardown order.
Agents are told nothing, or are killed with the process. (UX lane: whether to prompt.)

**Fix sketch.** Register `cx.on_app_quit` in `agent_review::init` to close every review
explicitly, so held writes get the feedback error before the agent process goes away. Optionally
prompt on pane close while the pane has decided but unsubmitted hunks.

## 12. Low: typing inside its debounce is lost when the window closes

**Where.** `view.rs:387-390`. The debounce `Task` lives in the view's `InputEditor`. A closed window
drops the view, which cancels the task, and nothing flushes it (`flush_pending_inputs` runs only
before another event, `:119-127`).

**Effect.** The last 150 ms of typing in an extension input never reaches the extension when the
window closes or the panel is removed.

**Fix sketch.** Flush pending inputs in a `cx.on_release` of `SurfaceView`. The state entity outlives
the view, so the dispatch still has somewhere to go.

## 13. Info: `gpui_tokio` initialization

Production calls `gpui_tokio::init` once (`crates/zed/src/main.rs:495`), before `ui_ext_host::init`
(`:751`). Every other call is a test, a bench, an eval or a separate binary (`remote_server`,
`eval_cli`, `edit_prediction_cli`), and none of those load UI extensions. So no production path
reaches W3's double-init problem.

The latent effect: replacing `GlobalTokio` drops the old runtime (`gpui_tokio.rs:42-48`,
`shutdown_background`). That aborts every UI component's queue task. The next call on each fails
with "has shut down", a non-`ReplyError`, so each extension stops with `retryable: false`.
Nothing guards against it. If a Fyx crate ever needs tokio during init, it should use
`Tokio::handle` and never call `init` again. A `debug_assert!(!cx.has_global::<GlobalTokio>())` in
`gpui_tokio::init` would be an upstream edit, so it is left as a note.

## 14. Info: synchronous work on the main thread

None of these blocks for long in normal use; they are listed for the perf lane.

- `ProposedEdit::new` → `compute_hunks` → `language::line_diff`, on the foreground inside
  `AgentReview` updates: `review_write` `agent_review.rs:867`, `permission_preview` `:1315`,
  `v2_patch_preview` `:1263-1265` (also `apply_diff_patch`); `submit` → `apply_accepted` `:644`.
  `gate_agent_write` copies the whole file with `snapshot.text()` (`fyx_review.rs:66`). An agent
  writing a multi-MB generated file stalls a frame or more.
- TOML parsing on the foreground: `UiManifest::read` (`ui_ext_wasm.rs:223`) from registry scans
  and the install action, and `ReviewRules::parse` (`agent_review.rs:1380`). The files are small.
- `path.is_dir()` on the foreground: `registry.rs:793, 802, 834` (dev paths only).
- `foreground_executor().block_on(kvp.write_kvp(..))` at startup: `zed/src/zed/fyx.rs:67-71`. Runs
  on the first launch only, and on purpose (the comment says why).
- `std::fs::read_to_string` in `script_driver::init` (`script_driver.rs:55`): dev builds only.

Wasm compilation is off the main thread (`ui_ext_wasm.rs:549`, background executor), and
instantiation runs on tokio (`:643`).

## Checked and clean

- **Install consent across window close** (`install.rs:122-150`). `window.prompt` resolves to
  `Err` when the window goes, so `answer.await != Ok(0)` returns without installing. Closing after
  consent but before the rescan leaves a linked, not yet loaded extension, which loads on the next
  scan.
- **Entity graph, ui_ext_host.** Registry → `SurfaceState`; panel / status item / decoration →
  `SurfaceView` → `SurfaceState`. `SurfaceState` holds no view, panel or workspace handle, and
  `RegistryLink` holds the registry weakly. No cycle. Panels are kept as `WeakEntity` in
  `WorkspaceSurfaces` (`panel.rs:243`).
- **Entity graph, agent_review.** `Responder::Permission` captures `WeakEntity<AcpThread>`
  (`:1120`); `ReviewHunkRenderer` holds the review and pane weakly (`review_pane.rs:1052-1055`);
  `AgentWrite.buffer` is not stored. No cycle keeps a pane or a thread alive.
- **Re-entrancy.** `fail_extension` skips the calling surface (`registry.rs:461`), and
  `route_push` never targets the caller's own surface (`surface.rs:442`). So the registry never
  updates the `SurfaceState` that is calling it, and there is no double-borrow panic.
  `ToolAuthorizationRequested` with a turn decision defers the answer to a task (`:819-824`),
  because answering re-enters the thread. `ReviewPane::new`'s `pane_opened` runs inside the
  workspace update, not the review's.
- **Decorations.** Blocks and views are released with the editor (`decorations.rs:264-269`).
  `Decorations::remove` notifies, and editors drop stale blocks.
- **Wasm call queue.** Pushes per call are capped (`MAX_PUSHES_PER_CALL`,
  `MAX_PUSHED_ITEMS_PER_CALL`). The instantiation push channel ends after init. A dropped caller
  does not cancel a queued call; its reply is discarded (`:860-861`). Dropping the last
  `UiExtension` clone aborts the tokio task through `Tokio::spawn`'s `defer`. `DEADLINE_WAKERS` is
  drained every tick.
- **Mount and resync.** Replacing `_mount` cancels the previous mount's host future. Within one
  instance, the single FIFO queue orders an in-flight event before the resync mount, so its reply
  lands on the pre-mount tree, consistent with the guest mirror.
- **`fyx_guard` (W9 branch).** Policy loads carry a generation (`load_policy`). Mode Off clears
  state, a stale load is ignored, a removed worktree drops its entry, `Loading` fails closed, and
  `track_project` defers until the project is built. No lifecycle problem found.
- **Held write against session end.** `Stopped`, `Error` and `ConnectionLost` close the session's
  reviews (`:1052-1057`), and the write fails with feedback. A thread released while a newer thread
  of the same session lives keeps the reviews (`thread_released`, which fixes w5-deep-1 L4).
- **Two windows sharing one instance.** One `SurfaceState` per surface and one queue per
  instance serialize events across windows, and `flush_sibling_views` keeps typing ahead of clicks
  across windows. One remaining divergence, for the logic/UX lane: typing in window A updates the
  shared tree (`set_input_locally`) but not window B's editor, because the set counter does not move.

## Appendix: tests

Run on a local scratch branch (not pushed) on top of 571c6826a5. Commands:

```bash
cargo test -p ui_ext_host --test async_lifecycle
```

```bash
cargo test -p agent_review --lib async_review_tests
```

Results: pending (build in progress); this section is updated in the next commit.
