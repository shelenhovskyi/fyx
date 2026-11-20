# Local history and agent timeline (plan 6.3)

Status: design and core only. `crates/fyx_history` holds the store, the timelines, restore and
pruning, with its tests. Nothing is wired into the app yet, and the crate is not a workspace
member yet (W0 lines below).

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

## What the host does (not done yet)

A `fyx_history_host` crate, owned by the stream that owns `agent_review` (W5), on a background
thread with one `History` per worktree behind a mutex:

- **Saves:** subscribe to `BufferEvent::Saved` of the project's local buffers (no upstream hook
  needed) and call `record_save` with the saved text.
- **Agent turns:** on the first landed write of an `agent_review` turn call `begin_turn(session,
  label)` and keep the `TurnId` → `ChangeId` map for the run (`TurnId` is not stable across runs,
  `ChangeId` is). For each landed write call `record_turn_file` with the file's text before and
  after; for writes that bypassed review (`AgentReviewEvent::UnreviewedChanges`) use the observed
  before-snapshot and the disk content. Accepted, rejected and auto-accepted files are all
  recorded as they landed.
- **UI:** a "Local History" timeline for the active file and an "Agent Turns" list: each turn with
  its files and "Undo Turn"; the conflict list from `plan` with "Restore Anyway"; "Restore This
  Version" on a file snapshot. Open buffers reload from disk after a restore.
- **Settings:** `fyx.history.enabled` (default on, local only), `fyx.history.max_total_mb`,
  `fyx.history.max_age_days`, `fyx.history.max_file_mb`.

## Lines for W0, the integrator

- Root `Cargo.toml`: `"crates/fyx_history", # fyx` in `members`, and
  `fyx_history = { path = "crates/fyx_history" } # fyx` in `[workspace.dependencies]`.
- `Cargo.lock`: regenerate. All dependencies (`hex`, `serde`, `serde_json`, `sha2`, `thiserror`,
  `tempfile` for tests) are already in the workspace at the locked versions.
- Check: `cargo nextest run -p fyx_history` (10 tests).

## Open items

- The host crate and the UI above (W5 or whoever takes `agent_review`).
- Done when (plan): accept an agent turn, then undo the whole turn from the timeline in one step.
  Not seen in a running build yet.
