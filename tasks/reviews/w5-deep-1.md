# Deep review: W5 agent change review (w5-deep-1)

Base: `origin/integration` at `f3718ca` (W5 last merged in `3db9068`, head `3a3b488`).

**Method.** I read the code and did not compile or run it. In scope:
- `crates/agent_review/` (the model, store, pane, script driver and stub agent);
- the `fyx:` hooks in `crates/acp_thread` (`acp_thread.rs:9, 8017, 8089`, `fyx_review.rs`) and `crates/agent_ui` (`agent_ui.rs:687`, `Cargo.toml:31`);
- the write and permission entry points in `crates/agent_servers/src/acp/{v1,v2}.rs`;
- Zed's native agent tools (`crates/agent/src/tools/`);
- `buffer_diff` and `language::text_diff`, for the pane's diff.

The settings toggle files named in the brief are not on `integration`:
- `crates/settings_content/src/fyx.rs` does not exist;
- `crates/settings/src/vscode_import.rs` has no `fyx` hook;
- `assets/settings/fyx.json` has no review key.

`.fyx/review.toml` and per-turn review are not there either. For these, finding M6 gives the requirements they must meet, not a code review.

## Summary

The gate on `fs/write_text_file` holds up, and the store and pane do what they claim for that path:
- it fails closed;
- a partial accept writes exactly the old text plus the accepted hunks;
- the feedback reaches the agent as the JSON-RPC message;
- toggling review off does not release held writes;
- no extension can resolve a review.

The main gap is coverage. Three of the four ways an agent changes files today are not reviewed, and nothing tells the user that they bypassed review:
- ACP v2 agents (H1);
- Zed's native agent (H2);
- shell or terminal edits (H3).

The second gap is how review state behaves over time:
- a held write is diffed against a snapshot taken before an unbounded wait, then applied to whatever the buffer holds afterwards (M4);
- a held write survives a stopped turn and a closed pane (M3);
- in a partly accepted permission preview, the agent is told only "denied" (M1).

Two findings from `integration-3` are still open: #1, now M5, and #3, now M2.

| # | Sev | Area | One line |
|---|---|---|---|
| H1 | High | coverage | ACP v2 edits are never reviewed: no preview for patch diffs, and v2 has no `fs/write_text_file` |
| H2 | High | coverage | Zed's native agent edits buffers directly; review mode has no effect on it |
| H3 | High | coverage | Terminal/shell writes and agents that write to disk themselves are neither reviewed nor reported |
| M1 | Medium | truthfulness | Partly accepted preview → plain `RejectOnce`; landed hunks and every preview comment never reach the agent |
| M2 | Medium | errors | (integration-3 #3, open) failed host write of accepted hunks is only logged; it also saves unrelated unsaved edits |
| M3 | Medium | cleanup | Held writes survive a stopped turn and a closed pane; the agent blocks with no indicator, a late accept writes after the agent gave up |
| M4 | Medium | correctness | Stale base: held write is anchored to the agent's last-read snapshot; concurrent threads/user edits/disk reloads in an accepted hunk are overwritten |
| M5 | Medium | consent | (integration-3 #1, open) the script driver still ships in release and can approve edits and dispatch any action |
| M6 | Medium | settings | Review mode is env/action only, not persisted, no indicator; requirements for the coming setting and `.fyx/review.toml` |
| M7 | Medium | UI ↔ model | Pane hunks (BufferDiff) and model hunks (`line_diff`) can differ; on CRLF/newline-only changes all displayed hunks bind to one model hunk |
| L1–L10 | Low | various | adoption by content ignores position; stale allowed previews; first diff only; session-id keyed cleanup; … |

---

## High

### H1. ACP v2 agents' edits are never reviewed

**Where.**
- `agent_review.rs:470` (`request.legacy_options()?`) and `:474-480` (`find_map` only on `ToolCallContent::LegacyDiff`): a v2 permission request (`AcpThread::request_permission`, `acp_thread.rs:6662`, `PermissionRequestData::Generic`) and v2 patch diffs (`ToolCallContent::DiffPatch`, `acp_thread.rs:3264`) get no preview.
- `crates/agent_servers/src/acp/v2.rs` registers no `fs/*` handler. Only v1 advertises and handles `write_text_file` (`v1.rs:447, 513-515, 10194-10226`).

**Failure scenario.** Review mode is on and a v2 agent proposes an edit:
1. The thread shows the patch with its usual Allow/Deny.
2. The user clicks Allow.
3. The agent writes the file itself.

The gate never runs, so the user gets no hunk review and no notice that review did not apply. `tasks/progress/W5.md:102` says v2 diffs are "reviewed at write time instead". That is true only for v1 agents that use `fs/write_text_file`. v2 has no such write.

**Fix.**
- Build a `ProposedEdit` for v2 edit permissions. Take the old text from the project buffer (the current text, not the agent's copy) and get the new text by applying the patch.
- Reuse the v1 verdicts: `Allow`, `Deny`, `ApplyAcceptedAndDeny`.
- If the patch does not apply to the current buffer, or any other edit permission cannot be previewed while review mode is on, say so on the permission card: "Fyx cannot review this edit hunk by hunk."
- Add a test with a v2 `RequestPermissionRequest` that carries a `Diff` patch.

### H2. Zed's native agent bypasses review entirely

**Where.**
- The native agent writes buffers directly: `crates/agent/src/tools/edit_session.rs:1025` (`buffer.edit`), `:1136` and `:208` (`save_buffer`). The other file tools do the same: `write_file_tool.rs`, `delete_path_tool.rs`, `move_path_tool.rs` (`rename_entry`, `:268`) and `copy_path_tool.rs`.
- None of them calls `AcpThread::write_text_file`. The diffs it reports are `ToolCallContent::Diff` (`acp_thread.rs:6233`, from `agent/src/thread.rs:6100`), not `LegacyDiff`, so `permission_preview` skips them too.

**Failure scenario.** The user turns review mode on and uses the built-in Zed agent. Every edit, delete and move lands as upstream does. The review UI suggests otherwise.

**Fix.** Pick one of these and document it:
1. Hook the native agent's edit session before it commits. One `fyx:` hook would go in `edit_session.rs`, calling the same `AgentWriteGate` with old and new text per buffer.
2. Declare the native agent out of scope and show "not reviewed by Fyx" on its threads while review mode is on.

Option 1 matches the product claim. delete, move and copy need at least a consent prompt from the host.

### H3. Terminal/shell writes and self-writing agents are neither reviewed nor reported

**Where.**
- `AcpThread::create_terminal` (`acp_thread.rs:8094`) runs agent commands with no review hook.
- Agents that write to disk themselves never call `fs/write_text_file`: Claude Agent's `Bash` (`sed -i`, `cat >`), codex-style `apply_patch`, and any v2 agent (H1).
- `ActionLog` does not see these writes either.

**Failure scenario.**
1. The user rejects hunk 2 in the review pane.
2. The agent runs `sed -i 's/fifth line/FIFTH LINE/' demo.txt` in a terminal.
3. The rejected change lands, and nothing in Fyx says so.

The brief asks that every write path is reviewed or reported. This path is neither.

**Fix.** Reviewing shell writes is out of reach. Reporting them is not:
- while a session has a turn in progress, record worktree file events and buffer reloads for paths with no host write in flight;
- on `AcpThreadEvent::Stopped`, list them in the review UI or a notification ("changed outside review: demo.txt");
- optionally offer the post-hoc `AgentDiffPane` keep/reject for those files.

At minimum, document in `docs/fyx/` that terminal writes bypass review.

---

## Medium

### M1. A partly accepted preview tells the agent only "denied"; preview comments are dropped

**Where.**
- `permission_verdict` (`agent_review.rs:404-412`).
- The responder (`:487-524`): for `Deny` and `ApplyAcceptedAndDeny` it sends `RejectOnce` and nothing else.
- `ProposedEdit::feedback()` (`proposed_edit.rs:210-251`) is never called on the permission path.

**Failure scenario A.** Claude Agent asks to edit `demo.txt`. The user accepts hunk 1 and rejects hunk 2 with the comment "keep lowercase".
1. The host writes hunk 1.
2. The agent gets `RejectOnce`.
3. Claude Code's denial text tells the model the user does not want to proceed and that it should stop and wait.

The model now believes nothing landed. Hunk 1 is on disk, and the comment is lost. The agent was told something false.

**Failure scenario B.** The user rejects everything and comments "use the constant instead". The agent gets a bare denial without the comment. The pane invites comments ("Comment for the agent on this hunk"), but on this path they go nowhere.

**Fix.** ACP has no feedback field on a permission answer, so the host has to supply one:
- put `edit.feedback()` in the outcome's `_meta`, for example `fyx/reviewFeedback`, for agents that read it;
- and queue it as a host note on the session. Show it in the thread, and prepend it to the next user prompt as a clearly marked block, e.g. "Fyx review: applied hunk 1 of 2 of your edit to demo.txt; hunk 2 rejected: keep lowercase".

Add a test that `Deny` with comments delivers them.

### M2. The host's write of accepted hunks fails silently and saves the user's unsaved edits (integration-3 #3, still open)

**Where.**
- `agent_review.rs:503-509`: `write_accepted_hunks` errors are only `log::error!`.
- `locate_fragment` (`:565-579`) fails if the user changed the fragment during review, if the fragment occurs twice (for example Claude `Edit` with `replace_all`), or if the agent's fragment never matched the file.
- `:560-562`: `save_buffer` saves the whole buffer, so unsaved user edits elsewhere in the file are written to disk along with the agent's hunks.

**Failure scenario.**
1. The user accepts 3 of 4 hunks. The fragment is no longer unique.
2. Nothing is written, and the agent is told "denied".
3. The user sees the review close as if it had been applied.

**Fix.**
- Return the error to the review: keep the review open, mark it "could not apply: …" in the pane, and let the user retry or deny with feedback.
- Before editing, check whether the buffer is dirty. If it is, don't save: leave the buffer dirty, or ask the user.

### M3. A held write survives a stopped turn and a closed pane

**Where.**
- `on_thread_event` (`agent_review.rs:438-460`) handles only the two authorization events. Nothing reacts to `AcpThreadEvent::Stopped`, `Error` or `ConnectionLost` (`acp_thread.rs:3855-3861`). `AcpThread::cancel` (`:7589`) cancels permission requests, which closes undecided previews, but it does not touch held writes.
- `ReviewPane` (`review_pane.rs:598-620`) has no close hook. Closing the tab leaves the `Review` and its oneshot open, and there is no indicator of open reviews; only the `agent_review: open reviews` action brings it back.
- Cleanup happens only on thread release (`agent_review.rs:48-52`).

**Failure scenario A.**
1. The agent's write is held.
2. The user presses Stop. The turn ends, and the agent treats its request as abandoned or times out.
3. Ten minutes later the user finds the pane and accepts.
4. The file changes after the agent was told the turn ended. The response goes to a request nobody is waiting for.

**Failure scenario B.** The user closes the review tab. The agent's tool call shows "in progress" forever, and nothing says why.

**Fix.**
- On `Stopped`, `Error` or `ConnectionLost`, close the session's `ReviewSource::Write` reviews ("the turn ended before the review; the file is unchanged").
- When the last pane of an undecided review closes, close the review, or ask first.
- Show an open-review count in the status bar or the thread, with a click target.

### M4. A held write is applied against a stale base

**Where.**
- `acp_thread.rs:8011-8016`: the snapshot is `shared_buffers[buffer]`, the text the agent last read. It is taken before `gate_agent_write` (`:8017-8018`), which can wait without limit.
- After the gate returns, the edits are `text_diff(snapshot, content)`, anchored in that old snapshot (`:8020-8031`), and applied to whatever the buffer holds now.
- `ProposedEdit.old_text` is that same snapshot (`fyx_review.rs:61-65`), so the pane diffs against text that may no longer be in the file.

**Failure scenarios.**
- **Two threads, same file.** Threads A and B each hold a write to `x.rs`, both based on S0. The user accepts A, so the buffer becomes S1. B's pane still diffs against S0. Accepting B's hunk that overlaps A's change replaces A's lines through the S0 anchors, and the pane never showed this.
- **User edits during review.** The user fixes a typo inside a line that an agent hunk replaces, then accepts the hunk. The fix is silently overwritten.
- **File changed on disk.** The buffer reloads during review, for example after `git checkout`. Accepted hunks are applied onto the reloaded text, at anchor positions computed for the old text.

Rejected hunks are safe, because they produce no edit. Accepted ones are not.

**Fix.**
- After the gate returns, compare the buffer's current version with `snapshot.version()`.
- If any edit since then intersects the old range of an accepted hunk, don't write. Reopen the review rebased onto the current text: build a new `ProposedEdit` from the current text to the current text plus the accepted changes, and keep the decisions where the hunks still match. Otherwise fail with feedback: "the file changed during review; re-read it".
- Serialise reviews per path. A second review of the same file waits until the first resolves and is then rebased.
- Add a GPUI test with two sessions writing overlapping hunks.

### M5. The script driver still ships in every build (integration-3 #1, still open)

**Where.**
- `agent_review.rs:10` (`mod script_driver;`) and `:58` (`script_driver::init(cx)`): no `cfg`, feature or release-channel check.
- `script_driver.rs:43-67` starts it when `FYX_AGENT_REVIEW_SCRIPT` is set.
- `:125-127` (`cx.build_action(&name, data)` plus `dispatch`) runs any registered action with JSON arguments.
- `:144-162` accepts, rejects and submits hunks.

**Failure scenario.** A launcher, shell profile or parent process sets the variable for a release `fyx`. The script then approves agent edits the user never saw. It can also run any action, for example `agent_review::ToggleReviewMode`, or ones that write settings.

**Fix (as before).**
- Gate the module and the call on `#[cfg(any(debug_assertions, feature = "review-script"))]`, with the feature off in release profiles.
- Log a `warn!` whenever the driver is active.
- Restrict `action` steps to an allowlist: `agent::NewExternalAgentThread` and `workspace::*` open actions.

### M6. Review mode: not persisted, no indicator; requirements for the coming setting and `.fyx/review.toml`

**Where.**
- `agent_review.rs:36-43`: the initial state comes only from `FYX_AGENT_REVIEW`.
- `review_pane.rs:118-129`: `ToggleReviewMode` flips an in-memory bool and logs at `info`.
- Nothing shows the state. No setting exists (see Method).

**Failure scenario.** The user turns review on with the command and restarts Fyx. Review is now off, with no sign of it, and agent edits land unreviewed.

**Requirements for W5's setting and rules work.** These are review criteria for when that code lands:
1. **Only user-level settings can loosen review.** A project `.zed/settings.json` and `.fyx/review.toml` may only tighten it: add paths that need review, never exempt paths or turn review off. An agent can write those files through the terminal (H3), or through a write the user accepted without noticing what it changes.
2. **Changing the setting must not release held writes.** Keep today's `set_enabled` semantics (`agent_review.rs:151-156`), and add a test for that live change.
3. **Rules-file changes during a session need confirmation.** If `.fyx/review.toml` changes while a session is running, apply it only after the user confirms. Never apply rules written by the session being reviewed.
4. **VS Code import must not loosen review.** `vscode_import.rs` must never map a VS Code setting onto a key that disables review or auto-accepts.
5. **Show the state.** Put the review mode on the status bar, and on the permission card when an edit will not be reviewed (H1, H2).

### M7. The pane's hunks and the model's hunks can disagree

**Where.**
- Model hunks: `language::line_diff` (`text_diff.rs:158`, histogram, no slider post-processing) on the raw strings, with a fallback to one whole-text hunk when the line diff does not reproduce the new text (`proposed_edit.rs:254-259, 263-285`). That covers newline-only changes and CRLF against LF.
- Displayed hunks: `BufferDiff` (`buffer_diff.rs:1253-1259`, histogram plus `postprocess_lines`) on buffers whose line endings are normalised (`text.rs:751, 1947`).
- The pane links the two only by start row (`review_pane.rs:644-648` for the buttons, `:350` for the cursor), through `hunk_at_new_row`.

**Failure scenarios.**
- **CRLF or newline-only change.** For example an agent writes CRLF content with two real changes against the LF buffer text. The model has one whole-file hunk, and the pane shows two hunks.
  - Both rows' Accept/Reject buttons bind to model hunk 0, and the header says "Hunk 1 of 1".
  - Accept on one row and Reject on the other: the last click wins, and either both changes land or neither.
- **Slider placement.** The model hunk is a pure deletion at new row r, but `postprocess_lines` displays it at r±1. That displayed hunk renders no controls (`hunk_at_new_row` returns `None`), and the cursor on it selects nothing ("Move the cursor into a hunk"). The user can submit only through Accept all or Reject all. The same can happen to insertions of repeated lines such as `}` or blank lines.

**Fix.**
- Use one source for both. Either derive the model's hunks from the pane's `BufferDiff` (`diff.hunks_intersecting_range` on the finalized diff), or render the controls from the model's hunks as custom blocks at the model's rows.
- Normalise line endings before `compute_hunks`. The buffer does this anyway, so CRLF never needs the whole-text fallback.
- Add a test asserting that each displayed hunk maps to exactly one model hunk, and the reverse, for CRLF, trailing-newline and slider cases.

---

## Low

- **L1. Adopted decisions ignore position.** `adopt_decisions` and `same_change` (`proposed_edit.rs:183-207, 28-38`) match only the hunk's text. The preview's `old_text` comes from the agent. An accepted preview hunk `let debug = false;` → `true;`, shown in a harmless-looking fragment, is adopted for the same line change anywhere in the written file. **Fix:** locate the preview's fragment in the write's old text (it must be unique), and adopt only for write hunks whose old rows fall inside it.
- **L2. Allowed previews never expire.** `agent_review.rs:235-240`: an allowed preview stays until a write adopts it, a newer preview of the file supersedes it, or the session closes. With a self-writing agent, or a tool that failed, it lingers and can be adopted by a later, unrelated write. **Fix:** drop it when its tool call reaches `Completed` or `Failed` (`EntryUpdated`), and on `Stopped`.
- **L3. Only the first diff is previewed.** `agent_review.rs:474-480` uses `find_map`, so a tool call with several `LegacyDiff`s (multi-file or multi-edit) shows one diff. `Allow` then also lets the unshown diffs through for agents that write themselves. **Fix:** one review per diff, and `Allow` only if all of them are accepted.
- **L4. Cleanup is keyed by session id alone.** `agent_review.rs:48-52, 248-258`: when an old `AcpThread` entity for a session is released after a new one was created for the same session (reload or reconnect), it closes the new thread's reviews. **Fix:** store the thread's `EntityId` on `Review` and close by that.
- **L5. Some changes are reviewed twice.** Fragment previews that start or end mid-line, or end in a pure insertion, never match the write's hunks: `same_change` only handles a missing final newline. The user reviews the same change twice. **Fix:** compare after locating the fragment (see L1) instead of by hunk text.
- **L6. Format-on-save after a partial write.** `acp_thread.rs:8066-8084` can reformat the regions of rejected hunks, and `buffer_edited` after formatting attributes that to the agent. This is upstream behaviour, but it breaks "exactly the accepted hunks". **Fix:** document it, or skip formatting when the resolution is partial.
- **L7. Deleted or renamed during review.** If the file is deleted during review, accepting recreates it with stale content (`save_buffer`, `acp_thread.rs:8085-8088`). If it is renamed, the pane shows the old path and the write lands at the new one. **Fix:** after the gate, check `buffer.file()` for its disk state and path; fail with feedback if the file is gone or moved. Binary and non-UTF-8 files fail in `open_buffer` before the gate, as upstream; I did not check this at runtime.
- **L8. Two consent UIs.** While a host preview is open, the thread's permission card still offers Allow and Always allow (`permission_answered`, `agent_review.rs:341-353`, closes the preview). Always allow from the card ends previews for that tool, and with self-writing agents it ends review altogether. **Fix:** while a preview is open, disable the card's buttons or label them "decided in the review pane". This needs a small `fyx:` hook in `agent_ui`.
- **L9. A new review steals focus.** `review_pane.rs:181-182` focuses the new pane in every workspace whose project contains the file, mid-typing. **Fix:** focus it only in the window that owns the thread, and only if the agent panel or the editor is not being typed in, or show a notification instead.
- **L10. Comments go to the cursor's hunk.** Clicking a hunk's Accept/Reject (`review_pane.rs:661-667`) does not move `current_hunk`, so the next comment (`:286-293`) goes to the hunk at the cursor, not the one just decided. **Fix:** set `current_hunk` in the button handler.
- **Nit.** `submit` (`agent_review.rs:214-220`) takes the responder before the fallible `write_outcome`. If that step ever fails, the review stays listed as submitted with its sender dropped. It cannot fail today, but swapping the order costs nothing.

## Verified as correct

- **No extension can decide.** The WIT has no agent or review interface (`crates/ui_ext/wit/`, no match). `ui_ext_host` dispatches only its own surface actions (`keyboard.rs:46-73`). Nothing in Rust bridges the VS Code sidecar's `executeCommand`. The decision API exists only on the host entity, in Rust.
- **The agent cannot resolve a review.** It supplies content only; decisions carry over by content, with the L1 caveat.
- **The gate fails closed.**
  - When no store exists, writes pass through as upstream (`fyx_review.rs:57-59`).
  - When the store was dropped, the write errors (`agent_review.rs:431-433`).
  - When a review is closed, the write errors with "the file is unchanged" (`:396-400`, `fyx_review.rs:71`).
  - When every hunk is rejected, nothing is written (`fyx_review.rs:74-76`).
- **A partial write is exact for the gated path.** `apply_hunks` (`proposed_edit.rs:299-321`) is correct for adjacent hunks. Hunks are separated by at least one unchanged line, overlap is rejected, and the result is the snapshot plus the accepted hunks. This holds subject to M4 and L6.
- **Feedback reaches the agent as the JSON-RPC message.** See `fyx_review.rs:83-92`, with the matching test.
- **Toggling review off does not release held writes** (`agent_review.rs:151-156`).
- **A thread's release closes its reviews** (`:48-52`), subject to L4.
- **The preview path normalises CRLF** before locating the fragment (`:536-537`).
- **Verdict mapping.** A preview with pending hunks cannot be submitted (`:211-213`), so `ApplyAcceptedAndDeny` never runs with undecided hunks.
