# Local history and agent timeline (plan 6.3)

Status: core (`crates/fyx_history`) and host (`crates/fyx_history_host`) written and tested.
`fyx_history_host::init` still needs its call in `main.rs` (W0), after `agent_review` is set up.
Not seen in a running build yet.

## Goals

- A snapshot of a file on every save and for every file an agent turn changed, kept on this
  machine only. Nothing is uploaded.
- A timeline per file (its snapshots, newest first) and per change (an agent turn or a restore,
  with its files).
- Restore one file to a snapshot, or undo a whole agent turn in one step, also after the turn was
  accepted in review.
- Never overwrite a later edit silently: a file that changed since the turn is a conflict, and the
  restore needs the user's "Restore anyway". Every restore is itself a change, so it can be undone.
- A size cap and an age limit, applied by pruning.

## Store

One store per worktree: `<data dir>/history/<16 hex of SHA-256 of the worktree path>/`
(`fyx_history::store_dir`). The path itself never appears in a file name.

- `index.jsonl`: append-only. The first line is `{"entry":"header","schema":"fyx-history/1"}`.
  Then `change` entries (`agent-turn` with session id and label, or `restore` with its source) and
  `snapshot` entries (path relative to the worktree with `/`, origin `save` or `change`, the
  content before for a change, the content after). A torn last line from a crash is skipped and
  dropped by the next prune.
- `blobs/<2 hex>/<sha256>`: file contents, deduplicated by hash, written atomically.
- A content is `missing` (the file did not exist), `stored`, or `too-large` (above
  `max_file_bytes`, 8 MiB by default): a too-large file keeps its hash, so conflicts are still
  seen, but it cannot be restored and a turn over it refuses to restore.
- Paths with `..`, a root or a drive are refused, so a restore only ever writes below the worktree.

## Restore

`History::plan(source)` shows what a restore does: each file and its new content, the conflicts
(the disk no longer holds what the change left) and the files that cannot be restored.
`History::restore(source, overwrite_conflicts, now)` then:

1. refuses when a content was too large, or when there are conflicts and `overwrite_conflicts` is
   false;
2. records a `restore` change;
3. for each file, stores what is on disk now as the snapshot's "before", writes the new content
   atomically (or deletes the file the turn created), and records the snapshot.

Undoing a restore is restoring its change. A file the agent edited several times in one turn goes
back to its content before the first edit. File permissions are not restored (the content only).

## Pruning

`History::prune(now)` drops snapshots older than `max_age_ms` (30 days), then the oldest until the
distinct contents fit `max_total_bytes` (256 MiB). A change goes with all its files, never in part,
and the newest save or change is always kept. The index is rewritten atomically and unreferenced
blobs are deleted. The host prunes when it opens a store and after every 100 records.

## What the host does (`fyx_history_host`)

One `History` per worktree behind a mutex, used from background tasks only.

- **Saves:** every local project's buffers are watched for `BufferEvent::Saved`; the buffer text
  with the buffer's line ending goes to `record_save` (no upstream hook).
- **Agent turns:** `agent_review` calls a decided-file observer (`set_decided_file_observer`,
  `DecidedFile`) for every file decided in a turn, with the old text and the text the decision
  lands. The first file of a turn starts a history change (`TurnId` → `ChangeId` for the run).
  The file's line ending on disk is applied to both texts. A file the turn created has no
  "before". Limits: turns are recorded only while `agent_review` reviews (review mode on, or a
  rule applies); writes that bypassed review are not recorded; a deleted file is recorded as
  emptied, so undoing it shows a conflict and needs "Restore Anyway".
- **UI:** "fyx: local history" opens a modal for the active file's worktree: agent turns and
  restores with "Undo Turn" / "Undo Restore", the active file's versions with "Restore This
  Version", and, when files changed since, their list with "Restore Anyway" and "Cancel". Open
  buffers reload from disk on their own.
- **Settings:** `fyx.history.enabled` (default true), `max_total_mb` (256), `max_age_days` (30),
  `max_file_mb` (8), in `settings_content::FyxHistorySettingsContent` and `assets/settings/fyx.json`.

## Lines for W0, the integrator

- Done: both crates are members, `zed` depends on `fyx_history_host` (c06dd72515).
- `Cargo.lock`: regenerate for the host crate's dependencies (agent_review, anyhow, fyx_history,
  gpui, language, log, paths, project, settings, ui, workspace; dev: tempfile).
- `main.rs`: `fyx_history_host::init(cx);` after `agent_review` is initialised (the observer is
  set on its global; without it agent turns are not recorded and a warning is logged).
- Check: `cargo nextest run -p fyx_history -p fyx_history_host -p agent_review`.

## Open items

- Record writes that bypass review and real deletions; a keybinding for "fyx: local history".
- Done when (plan): accept an agent turn, then undo the whole turn from the timeline in one step.
  Not seen in a running build yet.
