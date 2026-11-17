# Logic-bug review 1 (bugs-1)

Scope: Fyx's own crates and their `fyx:` hooks, at `fyx/main` 571c6826a5. That covers `agent_review`
with `acp_thread/src/fyx_review.rs`, `ui_ext`, `ui_ext_sdk`, `ui_ext_wasm`, `ui_ext_host` and `fyx_ext`.
It also covers two unmerged branches: `fyx_registry` (`origin/claude/fyx-registry-core` a5007f7) and
`fyx_guard` (`origin/claude/fyx-w9-guard` 6775a87). The review is about correctness only.
Performance, concurrency, security threat models, test coverage and UX belong to the other
reviewers. Where a finding crosses into one of those lanes, it says so in one line.

Method: I read the code, and for every candidate I wrote a failing test in a temporary module. Each test
ran with `RUSTC_WRAPPER= cargo test -p <crate> zz_repro` (sccache is not installed in this container), and
the product files were reverted afterwards. No product code changed. The repro tests are in the appendices
at the end, ready to paste back in.

* **CONFIRMED**: the test ran and failed as predicted; the failing output is quoted.
* **SUSPECTED**: established by reading the code; the reason it was not run is given.

Not reviewable at this commit:

* There is no consent code. `fyx-ui-consent.json` does not exist anywhere in the tree or its history.
* There is no WIT 0.2 host support (`MIN_VERSION = MAX_VERSION = 0.1.0`), so version dispatch reduces to
  "0.1 or refuse". That part was checked and is correct; see "Checked and correct".

## Summary

| # | Sev | Crate | Finding | Status |
|---|-----|-------|---------|--------|
| 1 | High | agent_review | Rules an agent wrote mid-session are applied without confirmation at the next thread start | CONFIRMED |
| 2 | High | acp_thread hook | A write to a non-canonical path (`..`, `\\?\`) fails with a false "was moved" error, even with review off | CONFIRMED (`..`), SUSPECTED (`\\?\`, Windows) |
| 3 | Medium | ui_ext_host | After a hot reload, a late error from the old instance permanently fails the new one | CONFIRMED |
| 4 | Medium | ui_ext_host | After a hot reload, a late reply from the old instance patches the new instance's tree | CONFIRMED |
| 5 | Medium | fyx_ext | On Unix, a backslashed `component` passes `check` and is packed as an `out/../../evil.wasm` entry | CONFIRMED |
| 6 | Medium | agent_review | gitignore-style rule patterns (`/Cargo.lock`, `secrets/`, `./x/**`) parse but never match | CONFIRMED |
| 7 | Medium | agent_review | A `..` segment in an agent's path escapes directory `deny`/`always_ask` rules | CONFIRMED |
| 8 | Medium | agent_review | Review rules match case-sensitively on case-insensitive file systems | CONFIRMED (matcher) |
| 9 | Medium | agent_review | An invalid rules file is offered as a "removal", and the next thread drops every rule | CONFIRMED |
| 10 | Medium | ui_ext_host | Stopping a surface while it mounts does not stick: the mount revives it | CONFIRMED |
| 11 | Medium | ui_ext_sdk | A surface grown past 2048 nodes cannot be mounted again by the SDK | CONFIRMED |
| 12 | Medium | fyx_registry | One malformed release discards the whole index | CONFIRMED |
| 13 | Medium | fyx_ext | A component under `assets/` is packed twice | CONFIRMED |
| 14 | Medium | fyx_guard | Windows-equivalent spellings (`.mcp.json.`, `::$DATA`, `.claude./`) are not protected | CONFIRMED (matcher) |
| 15 | Medium | fyx_guard | A project inside `.claude/worktrees/…` or `~/.codex/…` is read-only in shield mode | CONFIRMED |
| 16 | Medium | fyx_guard | `.fyx/security.toml` patterns match case-sensitively | CONFIRMED (matcher) |
| 17 | Low/Med | ui_ext_host | A manifest that fails to parse is treated as an uninstall, and hot reload stops | CONFIRMED |
| 18 | Low | agent_review | "Accept/reject all in turn" stops at a preview being applied and leaves the rest undecided | CONFIRMED |
| 19 | Low | agent_review | An allowed preview plus its write is recorded twice in the turn summary | CONFIRMED |
| 20 | Low | ui_ext_sdk | `apply_event` overwrites the mirror while a `set-props` for the node is queued | CONFIRMED |
| 21 | Low | fyx_registry | Build metadata (`1.2.0+local`) escapes `=`, `<=` and `>=` blocks | CONFIRMED |
| 22 | Low | fyx_registry | `fetch_due` never fires when the last attempt is stamped in the future | CONFIRMED |
| 23 | Low | fyx_ext | `new` accepts names over 64 characters that `check` rejects | CONFIRMED |
| 24 | Low | fyx_ext | `--sdk-git/--sdk-branch/--sdk-rev` are written into `Cargo.toml` without TOML escaping | CONFIRMED |
| 25 | Low | fyx_ext | A 0.2-world component passes `check` under `api_version = "0.1.0"` when it has no version section | CONFIRMED |
| 26 | Low | fyx_ext | Symlinked assets are dropped from the package without a message | CONFIRMED |
| 27 | Low | fyx_guard | Submodule hooks (`.git/modules/<m>/hooks/`) are not protected | CONFIRMED |
| 28 | Low | fyx_guard | A denial names every pattern instead of the one that matched | CONFIRMED |
| 29 | Low | ui_ext_host | The install toast reports success while loading fails | SUSPECTED |
| 30 | Low | ui_ext_host | PageDown overflows with `row_count` near `u32::MAX` | SUSPECTED |
| 31 | Low | ui_ext_host | A reinstall keeps the old title, icon and kind of an existing surface | SUSPECTED |
| 32 | Low | ui_ext_wasm | Pushes made during instantiation are cleared by `begin_call` | SUSPECTED |
| 33 | Low | fyx_ext | `\\?\UNC\` paths become relative `UNC/…` paths in `Cargo.toml` and in the dev env var | SUSPECTED (Windows) |
| 34 | Low | fyx_ext | The dev watcher looks for `assets` anywhere in the absolute path | SUSPECTED |

---

## High

### 1. Rules an agent wrote mid-session are applied without confirmation at the next thread start

`crates/agent_review/src/agent_review.rs:1398-1404` (`reload_rules`), `:545-550` (`set_rules`), `:137-145` (`init`)

**Scenario.** A session runs with `.fyx/review.toml` = `deny = ["*.env"]`. The agent rewrites the file,
for example with a shell command, to `auto_accept = ["**"]`. `on_project_event` calls
`reload_rules(FileChanged)`, and the change is held as pending with an "Apply new review rules" toast.
So far this is correct. Then the user opens another thread, or the agent's session is reloaded.
`observe_new::<AcpThread>` calls `reload_rules(ThreadStarted)`, which calls `set_rules`. That applies the
file as it is and clears the pending entry.

**Actual.** The agent-written rules take effect without the user's confirmation. In a trusted worktree,
`auto_accept = ["**"]` waves every later edit through. **Expected.** A file that differs from the applied
rules stays pending until the user applies it, whichever event loads it.

**CONFIRMED** (`zz_new_thread_applies_held_rules_without_confirmation`):
```
assertion `left == right` failed: the held rules were applied without the user's confirmation
  left: Some(AutoAccept)
 right: Some(Deny)
```

**Fix sketch.** Only the first load for a root may apply directly. If `self.rules` already has rules
for that root, or it has `pending_rules`, `ThreadStarted` must go through `propose_rules` as well.
Simplest: `RulesLoad::ThreadStarted` with a known root becomes `propose_rules`, and `set_rules` is used
only for a root never seen before. The security reviewer should also decide whether a first load after
a restart counts as confirmation; that is a policy question, so it is mentioned only here.

### 2. A write to a non-canonical path fails with a false "was moved" error, even with review mode off

`crates/acp_thread/src/fyx_review.rs:90-98`; the gate is called at `crates/acp_thread/src/acp_thread.rs:8019`

**Scenario.** An agent calls `fs/write_text_file` with `/p/src/../a.txt`. Upstream resolves the path
through `project_path_for_absolute_path` → `RelPath::new`, which normalizes `.` and `..`, and opens
`/p/a.txt`. The Fyx hook then compares the raw path the agent sent with the buffer's
`file.abs_path(cx)`. They differ, so the hook returns "was moved to /p/a.txt while your edit was under
review". The gate is installed globally by `agent_review::init`, so this happens with review mode
**off**, the default. It is a regression against upstream.

On Windows, the same applies to any path that upstream accepts through `SanitizedPath`/`dunce`
simplification, but whose raw form is not component-equal to the worktree path. The main case is a
`\\?\C:\…` verbatim path, which is what `std::fs::canonicalize` returns, so it is common in agent
tooling. Such a path fails **every** write. Forward-slash and drive-letter-case differences compare
equal, so they are not affected.

**Actual.** The write is refused with a misleading error; with review off, upstream would write it.
**Expected.** The path is compared after the same normalization upstream uses, so the write lands.

**CONFIRMED** for `..` (`zz_dotdot_path_write_fails_with_review_off`). The control test
`zz_dotdot_path_write_lands_without_the_gate` passes.
```
write failed with review off: /p/src/../a.txt was moved to /p/a.txt while your edit was under review, so nothing was written.
```
SUSPECTED for `\\?\`: it needs Windows, and std `Path` treats `\` as an ordinary character on Linux.

**Fix sketch.** Compare the buffer's path as it was **before** the review with its path after,
instead of comparing with the agent's string. Capture `file.abs_path(cx)` next to `version_before` and
compare `current != before`. Also pass that canonical path, not the agent's raw path, to
`AgentWrite::abs_path`, so `rule_for`, the turn history and `note_changed_paths` all see one spelling
(this also fixes #7 for v1 writes).

---

## Medium

### 3. After a hot reload, a late error from the old instance permanently fails the new one

`crates/ui_ext_host/src/surface.rs:213-223` (`replace_backend`), event worker `:299-324`, `apply_reply`
`:404-433`, `fail_extension` `:248-255`; `receive_rows` `:632-648` has the same gap

**Scenario.** A `handle-event` call is running on the old instance when a hot reload or reinstall
lands. `replace_backend` swaps the backend and remounts. Each instance has its own call queue
(`ui_ext_wasm.rs:839`), so the old call can still finish after the new mount. The typical case is the
200 ms budget interrupting a hang that the developer has just fixed. Nothing checks which instance a
result came from. The old trap goes to `fail_extension`, which fails every surface of the extension
(now on the new instance) with `retryable: false`.

**Actual.** The new instance shows "stopped", with no Retry. **Expected.** Results from a replaced
instance are ignored.

**CONFIRMED** (`zz_repro_stale_fatal_reply_after_reload`):
```
left: "Failed(gated stopped: wasm trap: interrupt, retryable=false)"
right: "Ready(v2)"
```

**Fix sketch.** Add `backend_epoch: u64`, incremented by `replace_backend`. The event worker,
`fetch_rows` and the mount task capture the epoch when they start a call. `apply_reply` and
`receive_rows` drop results from an older epoch, including their errors and pushes.
`Rc::ptr_eq` on the captured backend works too.

### 4. After a hot reload, a late reply from the old instance patches the new instance's tree

Same code as #3.

**Scenario.** Same as #3, but the old call succeeds after the new mount. SDK node ids restart at 1 on
every mount, so the old patch usually fits the new tree and is applied silently to the wrong node. The
new instance's SDK mirror never saw that patch, so host and guest diverge with nothing to bring them
back in line.

**CONFIRMED** (`zz_repro_stale_reply_applied_after_reload`):
```
left: "Ready(stale from v1)"
right: "Ready(v2)"
```

**Fix sketch.** The same epoch check as #3.

### 5. On Unix, a backslashed `component` passes `check` and is packed as a `../` entry

`crates/fyx_ext/src/manifest.rs:230-235` (`is_relative_inside` uses host `Path::components`),
`crates/fyx_ext/src/package.rs:29` (`replace('\\', "/")`)

**Scenario.** `component = 'out\..\..\evil.wasm'` on Linux or macOS. There it is a single `Normal`
component, so `check` passes. `package` then turns the backslashes into `/`. The same manifest fails
`check` on Windows.

**Actual.** The archive contains the entry `out/../../evil.wasm`, a path-traversal name for whoever
extracts it. **Expected.** The same verdict on every OS, and no `..` in any archive entry.

**CONFIRMED** (`zz_repro_backslash_component_becomes_traversal_entry`):
```
check accepted "out\\..\\..\\evil.wasm"; package entries: Ok(["extension.toml", "out/../../evil.wasm"])
```

**Fix sketch.** Validate `component` the same way on every OS: split on both `/` and `\`, and reject
empty, `.`, `..` and drive-like (`C:`) segments. Build the archive name from the normalized segments.
`bin/./x.wasm` is currently packed unnormalized too. (Security lane: the installer's handling of such
entries.)

### 6. gitignore-style rule patterns parse but never match

`crates/agent_review/src/review_rules.rs:84-107` (`Pattern::new`/`matches`)

**Scenario.** `deny = ["/Cargo.lock", "secrets/", "./private/**"]` follows the habit of `.gitignore`
and most other tools. Each of these contains `/`, so it is matched against the whole relative path,
like `Cargo.lock` or `secrets/key.pem`. A leading `/` or `./` cannot match a relative path. A trailing `/`
only matches the directory name itself, and rules are evaluated on files.

**Actual.** The file loads without error and the rules silently do nothing. **Expected.** Either the
documented forms work (anchor `/x`, directory `x/` = `x/**`, strip `./`), or `parse` rejects them with a
message.

**CONFIRMED** (`zz_anchored_and_directory_patterns_never_match`):
```
  left: (None, None, None)
 right: (Some(Deny), Some(Deny), Some(Deny))
```

**Fix sketch.** Normalize in `Pattern::new`: strip a leading `./`; strip a leading `/` and anchor;
turn a trailing `/` into `/**`. Reject an empty pattern, and reject `..` segments. Document the result
in `docs/fyx/review-rules.md`.

### 7. A `..` segment in an agent's path escapes directory `deny`/`always_ask` rules

`crates/agent_review/src/agent_review.rs:536-542` (`rule_for`), `review_rules.rs:153-156` (`evaluate`),
and the callers at `agent_review.rs:988, 1025, 858`

**Scenario.** With `deny = ["secrets/**"]`, a v1 permission preview or a v2 patch names
`/p/src/../secrets/key.pem`. `evaluate` strips the root lexically and matches `src/../secrets/key.pem`,
which does not match. Upstream opens the normalized file. With review mode off, no review opens and
no Deny is sent. A v2 agent writes the file itself once the stock permission prompt is allowed. A v1
write with that path currently fails because of #2, but only by accident, and only for local projects:
`file.as_local()` is `None` remotely. File-name-only patterns like `*.env` are not affected.

**Actual.** The deny rule is not applied. **Expected.** The rule applies to the file the path
resolves to.

**CONFIRMED** (`zz_dotdot_path_escapes_a_deny_rule`):
```
assertion `left == right` failed: the deny rule must apply to /p/src/../secrets/key.pem
  left: []
 right: [Deny]
```

**Fix sketch.** Resolve the path first. Either look it up with `project.project_path_for_absolute_path`
and evaluate the `RelPath` against the rules of that worktree, or at least normalize `.` and `..`
lexically (as `RelPath::new` does) before `strip_prefix`. Evaluating on the `RelPath` also removes the
`\\?\` and separator differences on Windows. The guard does this correctly: its reviewer verified that
`/p/src/../Makefile` is still blocked there.

### 8. Review rules match case-sensitively on case-insensitive file systems

`crates/agent_review/src/review_rules.rs:87-91`

**Scenario.** `deny = ["*.env", "secrets/**"]`, and an agent writes `config/PROD.ENV` or
`Secrets/key.pem`. On macOS (default APFS) and Windows these are the same files as the lower-case
ones, or create files that tools treat the same.

**Actual.** No rule matches. **Expected.** On case-insensitive platforms, matching ignores case, or
the path is first resolved to the worktree entry's on-disk case.

**CONFIRMED** for the matcher (`zz_rules_are_case_sensitive`); the effect on macOS and Windows follows
from their file systems:
```
  left: (None, None)
 right: (Some(Deny), Some(Deny))
```

**Fix sketch.** `GlobBuilder::case_insensitive(cfg!(any(target_os = "macos", target_os = "windows")))`,
or match against the worktree entry's path. See #16 for the same issue in `fyx_guard`; one shared
helper would be better.

### 9. An invalid rules file is offered as a "removal", and the next thread drops every rule

`crates/agent_review/src/agent_review.rs:1379-1404`

**Scenario.** The module doc and the error toast say a file that does not parse "is ignored rather
than half applied". In the code, a parse error becomes `rules = None`, the same value as "no file".

1. **During a session.** `propose_rules(None)` offers "Apply new review rules", next to the "is
   ignored" error. Applying the offer removes every rule.
2. **On a thread start.** `set_rules(None)` removes every rule silently. A half-typed edit by the
   user, or a deliberately broken file from an agent, drops all `deny` and `always_ask` protection
   (combine with #1).

**Actual.** After the invalid write, `has_pending_rules` is true; after the next thread, `rule_for(.env)`
is `None`. **Expected.** An invalid file keeps the last valid rules of that root and only reports the
error.

**CONFIRMED** (`zz_invalid_rules_file_drops_the_previous_rules`):
```
invalid file: pending removal offered = true, rule after next thread = None
```

**Fix sketch.** Distinguish the cases: `enum Loaded { Missing, Invalid(error), Valid(ReviewRules) }`.
`Invalid` leaves `rules` and `pending_rules` untouched.

### 10. Stopping a surface while it mounts does not stick: the mount revives it

`crates/ui_ext_host/src/surface.rs:237-244` (`fail`), mount completion `:174-186`

**Scenario.** `fail` sets the phase but leaves `_mount` running, and the mount's `set_tree` makes the
surface `Ready` again. There are two ways to get here. One is a reinstall that no longer declares a
surface that is mounting (`registry.rs:214-228`). The other is an uninstall (`plan_scan` →
`fail_extension`, `registry.rs:434-444`) while the surface mounts; the old instance is never disabled,
so its mount succeeds.

**Actual.** A working "zombie" surface remains for a removed surface or an uninstalled extension, still
connected to the old instance. **Expected.** Stopped stays stopped.

**CONFIRMED** (`zz_repro_fail_during_mount_is_undone`):
```
a stopped surface stays stopped, got Ready(v1)
```

**Fix sketch.** In `fail`, set `self._mount = None`; dropping the task cancels it.

### 11. A surface grown past 2048 nodes cannot be mounted again by the SDK

`crates/ui_ext_sdk/src/ui_ext_sdk.rs:610-614` (`SurfaceTree::mount` → `check_update_size`, `:834`)

**Scenario.** `docs/fyx/limits-and-gotchas.md:60-62` advises growing a surface with `insert`. Later the
host calls `mount` again: the doc says it does so after any rejected update, and tells authors to
rebuild from their state. With more than 2048 nodes, the SDK refuses its own rebuild. The host's
`Tree::from_nodes` accepts up to 16384 nodes ("any tree that can be built up through patches can also
be mounted"), and a mount reply is not a push.

**Actual.** `UpdateTooLarge { count: 3001, max: 2048 }`. The host shows the error and does not retry,
so the surface stays broken until restart. **Expected.** The mount succeeds.

**CONFIRMED** (`zz_repro_grown_tree_cannot_be_remounted`):
```
SDK refuses the remount: Err(UpdateTooLarge { count: 3001, max: 2048 })
```

**Fix sketch.** `mount` checks only `check_tree_size`, like the host. Keep the 2048 cap only for
updates that are pushed, and update the gotchas doc.

### 12. One malformed release discards the whole index (`fyx_registry`, branch `claude/fyx-registry-core`)

`crates/fyx_registry/src/index.rs:31-35` (`releases: Vec<Signed<Release>>`, strictly typed), `:289-292`

**Scenario.** The module doc (`index.rs:3-5`) and `registry-format.md` promise that a client "uses
every release that verifies even if another entry is damaged". In practice, one entry that fails to
deserialize makes `Index::from_json` fail as a whole. Examples: an upper-case sha256, a bad id
character, an unknown field, or bad base64.

**Actual.** `Err(Parse { document: Index, message: "\"BC4A…\" is not 64 lowercase hex digits …" })`, so
no extension can be installed or updated. **Expected.** The good `todo` release verifies, and the bad
entry is listed in `rejected`.

**CONFIRMED** (`zz_repro_one_malformed_entry_discards_the_whole_index`; output above).

**Fix sketch.** Deserialize `releases` as `Vec<serde_json::Value>` and parse each entry on its own,
recording failures in `rejected`. `RejectedRelease` then needs an optional id and version. The
alternative is to correct the docs.

### 13. A component under `assets/` is packed twice

`crates/fyx_ext/src/package.rs:26-50`

**Scenario.** `component = "assets/fyx_ui.wasm"`. The component is added explicitly, and then again by
the `assets/` walk.

**Actual.** The zip has two entries with the same name, and which one an extractor keeps is up to the
extractor. **Expected.** One entry.

**CONFIRMED**:
```
["extension.toml", "assets/fyx_ui.wasm", "assets/fyx_ui.wasm"]   left: 2
```

**Fix sketch.** Skip asset files whose archive name equals the component's, or dedupe entries by name.

### 14. Windows-equivalent spellings are not protected (`fyx_guard`, branch `claude/fyx-w9-guard`)

`crates/fyx_guard/src/policy.rs:52-71`, which compares components with `eq_ignore_ascii_case` only

**Scenario.** Win32 path normalization strips a trailing `.` from path segments, and a trailing `.` or
space from the last one; `::$DATA` names a file's main stream. So `/p/.mcp.json.`, `/p/.mcp.json `,
`/p/.mcp.json::$DATA`, `/p/.claude./settings.json` and `/p/.git/hooks./pre-commit` all open protected
files on Windows.

**Actual.** `protected_config` is `None` for each of them. **Expected.** `Some`.

**CONFIRMED** for the matcher. That Windows opens the protected file follows from the documented Win32
rules; it was not run on Windows.
```
not protected: ["/p/.claude./settings.json", "/p/.mcp.json.", "/p/.mcp.json ", "/p/.mcp.json::$DATA", "/p/.git/hooks./pre-commit"]
```

**Fix sketch.** Before comparing, strip trailing `.` and spaces and any `:stream[:type]` suffix from
each component. On Windows, refuse names containing `:` or ending in `.` or a space. (Security lane:
8.3 short names such as `CLAUDE~1` need a lookup on the file system.)

### 15. A project inside `.claude/worktrees/…` or `~/.codex/…` is read-only in shield mode (`fyx_guard`)

`crates/fyx_guard/src/policy.rs:35-44`, called at `crates/fyx_guard/src/fyx_guard.rs:158` on the full
absolute path

**Scenario.** Claude Code creates worktrees at `<repo>/.claude/worktrees/<name>`, and Codex at
`~/.codex/worktrees/…`. When such a worktree is opened as the project, every path in it contains a
protected component.

**Actual.** Every agent write is refused with "agent configs are read-only". **Expected.** Only config
files below the worktree root are protected.

**CONFIRMED**:
```
/repo/.claude/worktrees/feature/src/main.rs  left: Some(".claude/")  right: None
```

**Fix sketch.** When `find_project_path` succeeds, apply only `protected_config_rel` to the worktree-
relative path. For paths outside every worktree, ignore protected components that are ancestors of a
worktree root.

### 16. `.fyx/security.toml` patterns match case-sensitively (`fyx_guard`)

`crates/fyx_guard/src/policy.rs:90-98`; the config rule at `:53-54` is deliberately case-insensitive

**Scenario.** `private_files = ["secrets/**"]`, and an agent reads `Secrets/prod.json`. On macOS and
Windows this opens the private file. ACP `read_text_file` strips the worktree prefix lexically and keeps
the agent's casing.

**CONFIRMED** for the matcher:
```
Secrets/prod.json opens secrets/prod.json on macOS/Windows
```

**Fix sketch.** The same as #8; share one helper.

---

## Low / Medium

### 17. A manifest that fails to parse is treated as an uninstall, and hot reload stops

`crates/ui_ext_host/src/registry.rs:489-535` (`find_ui_extensions` skips the folder), `:427-445`
(`plan_scan`)

**Scenario.** A developer with a linked dev install typos the `[fyx_ui]` table, or bumps
`api_version`, and rebuilds. The scan skips the folder, and `plan_scan` treats the id as gone. It
removes the id from `installed` and `watches` and stops its surfaces with "X was uninstalled". Fixing
the typo then reloads nothing, because the watch was dropped. The extension store's watcher does not
look behind the link, so only a restart recovers.

**Actual.** The watch is gone and the message is wrong. **Expected.** A folder that is still present
keeps its watch, and its surfaces show the real parse error.

**CONFIRMED**:
```
panicked at registry.rs:896 'a manifest that does not parse for now is not an uninstall: the watch stays'
```

**Fix sketch.** `find_ui_extensions` also returns folders that exist but failed to read, with their
errors. `plan_scan` counts an id as gone only when its folder disappeared (key `installed` by folder).

---

## Low

### 18. "Accept/reject all in turn" stops at a preview being applied, and leaves the rest undecided

`crates/agent_review/src/agent_review.rs:497-517` (`decide_turn`); the error is only logged at
`review_pane.rs:624-632`

**Scenario.** A partly accepted preview is submitted, and its host write (`ApplyAcceptedAndDeny`) is in
flight. While it is, the user presses "accept all in turn". `open_reviews_in_turn` includes the applying
review, because its responder is still `Some`. `open_review_mut` returns "is being applied", and the
loop's `?` returns. Before that, `turn_decisions` was already set.

**Actual.** The decision covers later edits, but the reviews already open after the applying one stay
undecided. The only feedback is `log::info!`. **Expected.** Applying reviews are skipped, and every other
open review is decided.

**CONFIRMED** (`zz_decide_turn_stops_at_a_preview_being_applied`):
```
decide_turn = Err(review ReviewId(0) is being applied); second review still open = true; verdicts = []
```

**Fix sketch.** Filter `!review.applying` in `decide_turn` (or in `open_reviews_in_turn`). Collect
per-review errors instead of returning on the first one, and show them in the pane.

### 19. An allowed preview plus its write is recorded twice in the turn summary

`crates/agent_review/src/agent_review.rs:700` (`finish_permission`), `:912-913` (`review_write`), `:817`
(`open_permission_preview`, turn-decided path)

**Scenario.** A v1 preview is accepted. `finish_permission` records it. The agent's
`fs/write_text_file` then adopts the decisions and records the same file again. The same happens when a
turn decision answers the preview: it is recorded at `:817`, and again when the write comes.

**Actual.** The pane says "2 decided" for one file and lists it twice. **Expected.** One entry per
decided change.

**CONFIRMED**:
```
one file decided once: [TurnEntry { path: "/p/a.txt", resolution: AcceptedAll, .. }, TurnEntry { path: "/p/a.txt", resolution: AcceptedAll, .. }]
  left: 2  right: 1
```

**Fix sketch.** Don't record in `review_write` when every hunk was adopted from a preview. Or record a
`TurnEntry` keyed by preview id and update it in place.

### 20. `apply_event` overwrites the mirror while a `set-props` for the node is queued (`ui_ext_sdk`)

`crates/ui_ext_sdk/src/ui_ext_sdk.rs:704-716` (`apply_event`), `:721-730` (`set_props_from_host`)

**Scenario.** The extension calls `set_props(input, "")`, which is queued and not yet sent. Then the user
types "typed", and the extension calls `apply_event` before `take_update()`. The host applies the queued
`""` after the event, but the mirror holds "typed". A later `set_props(input, "")` is dropped as unchanged,
so the two stay out of sync. This only happens when patches are batched across calls.

**CONFIRMED**: `mirror and host diverged` (host `value: ""`, mirror `value: "typed"`).

**Fix sketch.** When `pending` holds a `SetProps` or `Insert` for the id, leave the mirror's props
alone, or drop the queued patch. Make that choice deliberately.

### 21. Build metadata escapes `=`, `<=` and `>=` blocks (`fyx_registry`)

`crates/fyx_registry/src/blocklist.rs:73-81`

**Scenario.** `semver::Version`'s `Eq`/`Ord` compare build metadata, which the semver spec ignores for
precedence. A local-file install of `1.2.0+local` is therefore not blocked by `=1.2.0`, although the
format doc says blocks apply to local files.

**CONFIRMED**: `=1.2.0 does not block 1.2.0+local; plan = []`.

**Fix sketch.** Compare with build metadata cleared, or use `cmp_precedence`.

### 22. `fetch_due` never fires when the last attempt is stamped in the future (`fyx_registry`)

`crates/fyx_registry/src/blocklist.rs:409-411`; `now.since(last)` saturates to 0

**Scenario.** The last attempt was stamped while the clock was a year fast. After the clock is corrected,
no block list is fetched for a year. `apply_fetch` guards against clocks going backwards; `fetch_due`
does not.

**CONFIRMED**: `no block list fetch for a year`.

**Fix sketch.** `last_attempt > now || now.since(last_attempt) >= BLOCKLIST_FETCH_INTERVAL`.

### 23. `fyx-ext new` accepts names over 64 characters that `check` then rejects

`crates/fyx_ext/src/template.rs:68-76` against `crates/fyx_ext/src/manifest.rs:216-227`

**CONFIRMED**: `fyx-ext new accepted it, check rejects: Report { errors: ["id \"aaaa…a\": use lower-case letters ..."] }`.

**Fix sketch.** In `names()`, also require `manifest::is_valid_id`.

### 24. `--sdk-git/--sdk-branch/--sdk-rev` are written into `Cargo.toml` without TOML escaping

`crates/fyx_ext/src/template.rs:127-136`

**Scenario.** With `--sdk-git 'C:\fyx'`, TOML reads `\f` as a form feed. `\U…` is a parse error, and a
`"` breaks the file. The `Checkout` path has the same problem with `"`.

**CONFIRMED**:
```
left: Some("C:\u{c}yx")  right: Some("C:\\fyx")
```

**Fix sketch.** Emit the values with `toml::Value::String(..).to_string()`, or build the table with
`toml`.

### 25. A 0.2-world component passes `check` under `api_version = "0.1.0"` when it has no version section

`crates/fyx_ext/src/inspect.rs:45-48` (`import_need` drops `@version`), `:159-163`

**Scenario.** The component imports `fyx:ui/host@0.2.0` and has no `fyx:ui-api-version` section. The
warning says the host falls back to the import versions, but `check` never compares them with the
manifest. The 0.1-only host would refuse the component, or fail to link it.

**CONFIRMED**: `Report { errors: [], warnings: ["no fyx:ui-api-version section: ..."] }`.

**Fix sketch.** When there is no section, parse the `@x.y.z` of the `fyx:ui/*` imports and run
`check_api_version` on it.

### 26. Symlinked assets are dropped from the package without a message

`crates/fyx_ext/src/package.rs:37`; `is_file()` is false for a symlink, and `WalkDir` doesn't follow links

**CONFIRMED**: `["extension.toml", "fyx_ui.wasm"]` (`assets/icon.svg` is missing).

**Fix sketch.** Handle `is_symlink()`: error out, or follow the link after checking that the canonical
target is inside the extension directory.

### 27. Submodule hooks are not protected (`fyx_guard`)

`crates/fyx_guard/src/policy.rs:63-68` matches only `.git` immediately followed by `hooks`

**CONFIRMED**: `/p/.git/modules/vendor/hooks/pre-commit` is not protected.

**Fix sketch.** After a `.git` component, treat any later `hooks` component as protected. (Security
lane: `core.hooksPath` in `.git/config`.)

### 28. A denial names every pattern instead of the one that matched (`fyx_guard`)

`crates/fyx_guard/src/policy.rs:113-119`; the doc comment says "the pattern that `path` matches"

**CONFIRMED**:
```
left: Some("keys/**, *.p12, infra/**")  right: Some("keys/**")
```

**Fix sketch.** Keep one matcher per pattern, and return the first one that matches.

### 29. The install toast reports success while loading fails (SUSPECTED)

`crates/ui_ext_host/src/install.rs:150-163`; load errors go only to `log::error!` at
`registry.rs:315-321, 338-341`

**Scenario.** `prompt_and_install` starts a detached `scan_installed` and returns `Ok(Some(id))` at once.
A panel surface without `ui:panel`, a component that fails to compile, or an id already claimed by
`FYX_UI_DEV_EXTENSIONS` then fails only in the log. The user sees "Installed the UI extension X" and no
panel. A hot reload of a bad build likewise fails silently and keeps the old instance.

**Not run because** it needs the folder picker and the confirmation prompt simulated.

**Fix sketch.** Have `scan_installed` return a `Task` with each id's result, and show it in the toast.

### 30. PageDown overflows with `row_count` near `u32::MAX` (SUSPECTED)

`crates/ui_ext_host/src/keyboard.rs:225`: `current.map_or(0, |index| index + page).min(last)`

**Scenario.** The guest controls `row_count`, and nothing caps it. Press End, then PageDown, with
`row_count = u32::MAX`. This panics in debug builds; in release it wraps to near the top.

**Not run because** rendering a list of that size was OOM-killed in this shared container. The arithmetic
is direct.

**Fix sketch.** Use `index.saturating_add(page)`. (Performance lane: an upper bound on `row_count`.)

### 31. A reinstall keeps the old title, icon and kind of an existing surface (SUSPECTED)

`crates/ui_ext_host/src/registry.rs:229-241`; for an existing surface, the new `SurfaceInfo` is only
logged

**Scenario.** A new version changes a surface from a status item to a panel. `add_missing_surfaces`
builds a panel from the old info, so it gets no slot (`SHARED_SLOT`) and the Blocks icon. The old status
item also stays. Title, icon and position changes are ignored until restart. Related, cosmetic:
`capability_chips` still shows an uninstalled extension's chips, because its stopped surfaces stay in
the registry.

**Fix sketch.** Store the new info. If the kind changed, remove the old item and add the new one.

### 32. Pushes made during instantiation are cleared (`ui_ext_wasm`, SUSPECTED)

`crates/ui_ext_wasm/src/ui_ext_wasm.rs:622`; `begin_call(UI_INIT_BUDGET)` runs after `instantiate_async`
(`:611-621`) and clears `pushes` and `pushed_items` (`:317-322`)

The doc comments at `:538-540` and `:634` say instantiation pushes are delivered.

**Not run because** it needs a fixture that calls `push-update` from a component start function, which
Rust/wit-bindgen components may never do. If that is the case, only the comments are wrong.

**Fix sketch.** Call `begin_call` before `instantiate_async`, or correct the comments.

### 33. `\\?\UNC\` paths become relative `UNC/…` paths (`fyx_ext`, SUSPECTED, Windows)

`crates/fyx_ext/src/template.rs:93-96` (`toml_path`), `crates/fyx_ext/src/build.rs:212-215` (`display_path`)

**Scenario.** `canonicalize` of a checkout on a network share returns `\\?\UNC\server\share\…`. That
becomes `UNC/server/share/…` in `Cargo.toml`, which Cargo resolves relative to the crate, and the same
form goes into `FYX_UI_DEV_EXTENSIONS`.

**Fix sketch.** Use `dunce::simplified`, or map the `\\?\UNC\` prefix to `\\`.

### 34. The dev watcher looks for `assets` anywhere in the absolute path (`fyx_ext`, SUSPECTED)

`crates/fyx_ext/src/build.rs:251-253` checks `components()` of the absolute path

**Scenario.** An extension under `/home/u/assets/ext` tracks every file, README included. A non-`.wasm`
component inside `assets/` would retrigger a rebuild after every build.

**Fix sketch.** Test `strip_prefix(dir)`, and exclude `component_path()` explicitly.

---

## Checked and correct

* **`ProposedEdit`** (`agent_review/src/proposed_edit.rs`):
  * CRLF normalization, and CRLF-only changes.
  * Newline-only changes fall back to a whole-file hunk.
  * The `line_hunks` fallback (`get(..)?`) when `line_diff` rows lie outside the text.
  * `apply_hunks` order and bounds checks.
  * Empty old or new text.
  * The `hunk_drawn_at` arithmetic (no underflow for empty or non-empty ranges).
  * `adopt_decisions` byte slicing (it only uses `match_indices` offsets, which are char boundaries).
  * Multiple occurrences of the fragment.
  * `locate_fragment` on an empty or repeated fragment.
* **`agent_review` state:**
  * `submit` and `finish_permission` keep a failed host write open with its error.
  * A second submit while applying is refused.
  * `permission_answered` closes only undecided previews.
  * `thread_released` counts threads per session.
  * `pane_closed` saturates.
  * `end_turn` prunes to 16 ended turns.
  * The stale-buffer check after review (version and deletion).
* **`ui_ext` tree:**
  * Patch batches are atomic, and their error indices count from 0.
  * Duplicate ids, self or forward parents, and `before` from another parent are rejected.
  * Removing a node removes its subtree.
  * `set_props` cannot turn a node with children into a leaf.
  * The node cap is checked per step.
  * There is no move operation, so no cycles.
  * `from_row` requires exactly one root.
  * There is no string slicing on guest input.
* **SDK and host conversions:** every field and variant maps in both directions; `row_range` and the
  host's `requested.contains` agree near `u32::MAX`.
* **Version handling:**
  * Without the section, the version falls back to the import version, and loading fails without
    either.
  * Duplicate sections are rejected.
  * 0.2.0, pre-release and build versions are rejected by `MIN..=MAX`.
  * The manifest and component versions must match.
  * A 0.1 section with 0.2 imports fails at link time.
* **Push caps:** at most 64 pushes and 4096 items per call; both limits allow exactly the cap and trap
  one past it.
* **`ui_ext_host`:**
  * Row cache generations and the in-flight/wanted handoff.
  * `fetch_range` clamping, including zero-row lists.
  * Keyboard wrap-around and First/Last bounds.
  * Status items are not added twice.
  * Duplicate surface ids are rejected.
  * Manifest id and component path validation.
  * Removing a dev-install junction on Windows.
  * Decoration 1→0-based lines.
  * The `fyx:` hooks in `dock.rs`, `extension_card.rs` and `main.rs`.
* **`fyx_ext`:**
  * Name and id validation (`""`, `-a`, `a__b`, `A`, `2x`); keyword crate names build.
  * `new` refuses to write into an existing directory.
  * `..` and absolute components are rejected on the host OS.
  * Malformed wasm is validated before it is inspected, so it is not a panic path.
  * The package size cap.
* **`fyx_registry`:**
  * Canonical encoding is length-prefixed and domain-tagged.
  * Duplicate JSON keys are rejected.
  * Range parsing and its bounds.
  * Sequence and replay handling.
  * The 24-hour age gate boundary.
  * The update planner: equal versions, downgrades, blocked, held, pinned and local cases.
  * Ids are lower-case only.
* **`fyx_guard`:**
  * `env_name_matches` wildcards.
  * Off→Audit→Shield→Off transitions, with the generation guard.
  * Fail-closed while loading or on a parse error.
  * `..` is normalized before `read_only` matching.

## Cross-lane notes (one line each)

* Security: #1, #7, #8, #9 and #14–#16 weaken or bypass protections; #5 is a traversal name in a package.
* Concurrency: #3, #4 and #10 are reply-ordering races across a backend swap.
* UX: #18 and #29 lose errors in logs; #9 shows a contradictory toast pair.

---

## Appendix A: `agent_review` repros (#1, #2, #6–#9, #18, #19)

Put the file at `crates/agent_review/src/zz_repro.rs` and add `#[cfg(test)] mod zz_repro;` at the end of
`agent_review.rs`. Run `RUSTC_WRAPPER= cargo test -p agent_review zz_`. Result: 8 failed;
`zz_dotdot_path_write_lands_without_the_gate` (the upstream control) passed.

```rust
use std::{cell::RefCell, path::Path, path::PathBuf, sync::Arc};

use super::*;
use gpui::TestAppContext;
use language::Buffer;

const OLD: &str = "one\ntwo\nthree\nfour\nfive\n";
const NEW: &str = "ONE\ntwo\nthree\nfour\nFIVE\n";

fn session() -> acp_v2::SessionId {
    acp_v2::SessionId::new("session")
}

fn new_request_id() -> PermissionRequestId {
    serde_json::from_value(serde_json::Value::String(uuid::Uuid::new_v4().to_string())).unwrap()
}

fn agent_write(path: &str, cx: &mut TestAppContext) -> AgentWrite {
    AgentWrite {
        session_id: session(),
        abs_path: PathBuf::from(path),
        buffer: cx.new(|cx| Buffer::local(OLD, cx)),
        old_text: OLD.into(),
        new_text: NEW.into(),
    }
}

fn recording_responder() -> (PermissionResponder, Rc<RefCell<Vec<PermissionVerdict>>>) {
    let verdicts = Rc::new(RefCell::new(Vec::new()));
    let respond: PermissionResponder = Rc::new({
        let verdicts = verdicts.clone();
        move |verdict, _, _| {
            verdicts.borrow_mut().push(verdict);
            Task::ready(Ok(()))
        }
    });
    (respond, verdicts)
}

fn init_settings(with_review: bool, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = settings::SettingsStore::test(cx);
        cx.set_global(settings_store);
        if with_review {
            crate::init(cx);
        }
    });
}

async fn project_and_thread(
    files: serde_json::Value,
    cx: &mut TestAppContext,
) -> (
    Arc<project::FakeFs>,
    Entity<Project>,
    Rc<acp_thread::StubAgentConnection>,
    Entity<AcpThread>,
) {
    let fs = project::FakeFs::new(cx.executor());
    fs.insert_tree(util::path!("/p"), files).await;
    let project = Project::test(fs.clone(), [util::path!("/p").as_ref()], cx).await;
    let connection = Rc::new(acp_thread::StubAgentConnection::new());
    let thread = new_thread(&connection, &project, cx).await;
    (fs, project, connection, thread)
}

async fn new_thread(
    connection: &Rc<acp_thread::StubAgentConnection>,
    project: &Entity<Project>,
    cx: &mut TestAppContext,
) -> Entity<AcpThread> {
    let thread = cx
        .update(|cx| {
            acp_thread::AgentConnection::new_session(
                connection.clone(),
                project.clone(),
                workspace::PathList::new(&[Path::new(util::path!("/p"))]),
                cx,
            )
        })
        .await
        .unwrap();
    cx.run_until_parked();
    thread
}

/// Upstream resolves `..` in an agent's path; the Fyx hook compares the raw path with the buffer's.
#[gpui::test]
async fn zz_dotdot_path_write_fails_with_review_off(cx: &mut TestAppContext) {
    init_settings(true, cx);
    let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
    assert!(!review.read_with(cx, |review, _| review.is_enabled()));
    let (fs, _project, _connection, thread) =
        project_and_thread(serde_json::json!({ "a.txt": OLD, "src": {} }), cx).await;
    let path = PathBuf::from(util::path!("/p/src/../a.txt"));
    let result = thread
        .update(cx, |thread, cx| thread.write_text_file(path, NEW.into(), cx))
        .await;
    assert!(
        result.is_ok(),
        "write failed with review off: {:#}",
        result.unwrap_err()
    );
    assert_eq!(fs.load(Path::new(util::path!("/p/a.txt"))).await.unwrap(), NEW);
}

/// The same write without Fyx's gate (upstream behaviour): it lands.
#[gpui::test]
async fn zz_dotdot_path_write_lands_without_the_gate(cx: &mut TestAppContext) {
    init_settings(false, cx);
    let (fs, _project, _connection, thread) =
        project_and_thread(serde_json::json!({ "a.txt": OLD, "src": {} }), cx).await;
    let path = PathBuf::from(util::path!("/p/src/../a.txt"));
    thread
        .update(cx, |thread, cx| thread.write_text_file(path, NEW.into(), cx))
        .await
        .unwrap();
    assert_eq!(fs.load(Path::new(util::path!("/p/a.txt"))).await.unwrap(), NEW);
}

/// A deny rule on a directory is not applied when the agent's path has a `..` segment; with review
/// off the preview is not even opened, so no Deny is sent.
#[gpui::test]
async fn zz_dotdot_path_escapes_a_deny_rule(cx: &mut TestAppContext) {
    let review = cx.new(|_| AgentReview::default());
    let rules = ReviewRules::parse(PathBuf::from("/p"), r#"deny = ["secrets/**"]"#, false).unwrap();
    review.update(cx, |review, cx| {
        review.set_rules(PathBuf::from("/p"), Some(rules), cx)
    });
    let direct = review.read_with(cx, |review, _| {
        review
            .rule_for(Path::new("/p/secrets/key.pem"))
            .map(|rule| rule.kind)
    });
    assert_eq!(direct, Some(RuleKind::Deny));

    let (respond, verdicts) = recording_responder();
    let preview = ProposedEdit::new(
        PathBuf::from("/p/src/../secrets/key.pem"),
        "a\n".into(),
        "b\n".into(),
    );
    let rule = review.read_with(cx, |review, _| review.rule_for(preview.abs_path()));
    review.update(cx, |review, cx| {
        review.open_permission_preview(session(), new_request_id(), preview, respond, rule, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        *verdicts.borrow(),
        [PermissionVerdict::Deny],
        "the deny rule must apply to /p/src/../secrets/key.pem"
    );
}

/// Rules an agent wrote during a session are held for the user, but the next thread start applies
/// them without asking.
#[gpui::test]
async fn zz_new_thread_applies_held_rules_without_confirmation(cx: &mut TestAppContext) {
    init_settings(true, cx);
    let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
    let (fs, project, connection, _thread) = project_and_thread(
        serde_json::json!({
            ".fyx": { "review.toml": "deny = [\"*.env\"]\n" },
            "a.txt": "a\n",
        }),
        cx,
    )
    .await;
    let env = PathBuf::from(util::path!("/p/.env"));
    let kind = |cx: &mut TestAppContext| {
        review.read_with(cx, |review, _| review.rule_for(&env).map(|rule| rule.kind))
    };
    assert_eq!(kind(cx), Some(RuleKind::Deny));

    // The agent rewrites the rules (e.g. with a shell command) during its session.
    fs.save(
        Path::new(util::path!("/p/.fyx/review.toml")),
        &"auto_accept = [\"**\"]\n".into(),
        Default::default(),
    )
    .await
    .unwrap();
    cx.run_until_parked();
    let root = PathBuf::from(util::path!("/p"));
    assert!(review.read_with(cx, |review, _| review.has_pending_rules(&root)));
    assert_eq!(kind(cx), Some(RuleKind::Deny), "held until the user applies");

    // The user opens another thread (or the agent's session is reloaded).
    let _second = new_thread(&connection, &project, cx).await;
    assert_eq!(
        kind(cx),
        Some(RuleKind::Deny),
        "the held rules were applied without the user's confirmation"
    );
}

/// A rules file that does not parse is "ignored", yet a thread start drops every rule of the
/// worktree.
#[gpui::test]
async fn zz_invalid_rules_file_drops_the_previous_rules(cx: &mut TestAppContext) {
    init_settings(true, cx);
    let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
    let (fs, project, connection, _thread) = project_and_thread(
        serde_json::json!({
            ".fyx": { "review.toml": "deny = [\"*.env\"]\n" },
            "a.txt": "a\n",
        }),
        cx,
    )
    .await;
    let env = PathBuf::from(util::path!("/p/.env"));
    fs.save(
        Path::new(util::path!("/p/.fyx/review.toml")),
        &"deny = [\"*.env\"\n".into(),
        Default::default(),
    )
    .await
    .unwrap();
    cx.run_until_parked();
    let root = PathBuf::from(util::path!("/p"));
    let pending = review.read_with(cx, |review, _| review.has_pending_rules(&root));
    let _second = new_thread(&connection, &project, cx).await;
    let kind = review.read_with(cx, |review, _| review.rule_for(&env).map(|rule| rule.kind));
    assert!(
        !pending && kind == Some(RuleKind::Deny),
        "invalid file: pending removal offered = {pending}, rule after next thread = {kind:?}"
    );
}

/// "Accept all in turn" stops at a preview whose host write is in flight and leaves the others.
#[gpui::test]
async fn zz_decide_turn_stops_at_a_preview_being_applied(cx: &mut TestAppContext) {
    let review = cx.new(|_| AgentReview::default());
    review.update(cx, |review, cx| review.set_enabled(true, cx));
    let never: PermissionResponder = Rc::new(|_, _, cx| {
        cx.spawn(async move |_| {
            futures::future::pending::<()>().await;
            Ok(())
        })
    });
    let (respond, verdicts) = recording_responder();
    let first = review
        .update(cx, |review, cx| {
            review.open_permission_preview(
                session(),
                new_request_id(),
                ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into()),
                never,
                None,
                cx,
            )
        })
        .unwrap();
    let second = review
        .update(cx, |review, cx| {
            review.open_permission_preview(
                session(),
                new_request_id(),
                ProposedEdit::new(PathBuf::from("/p/b.txt"), OLD.into(), NEW.into()),
                respond,
                None,
                cx,
            )
        })
        .unwrap();
    let turn = review.read_with(cx, |review, _| review.review(first).unwrap().turn());
    review.update(cx, |review, cx| {
        review
            .decide_all(first, HunkDecision::Accepted, cx)
            .unwrap();
        review.submit(first, cx).unwrap();
    });
    cx.run_until_parked();
    let result = review.update(cx, |review, cx| {
        review.decide_turn(turn, HunkDecision::Accepted, cx)
    });
    cx.run_until_parked();
    let second_open = review.read_with(cx, |review, _| {
        review
            .review(second)
            .is_some_and(|review| !review.is_submitted())
    });
    assert!(
        result.is_ok() && !second_open,
        "decide_turn = {result:?}; second review still open = {second_open}; verdicts = {:?}",
        verdicts.borrow()
    );
}

/// An allowed preview is recorded when it is answered and again when the write adopts it.
#[gpui::test]
async fn zz_previewed_write_is_recorded_twice(cx: &mut TestAppContext) {
    let review = cx.new(|_| AgentReview::default());
    review.update(cx, |review, cx| review.set_enabled(true, cx));
    let (respond, _) = recording_responder();
    let id = review
        .update(cx, |review, cx| {
            review.open_permission_preview(
                session(),
                new_request_id(),
                ProposedEdit::new(PathBuf::from("/p/a.txt"), OLD.into(), NEW.into()),
                respond,
                None,
                cx,
            )
        })
        .unwrap();
    let turn = review.read_with(cx, |review, _| review.review(id).unwrap().turn());
    review.update(cx, |review, cx| {
        review.decide_all(id, HunkDecision::Accepted, cx).unwrap();
        review.submit(id, cx).unwrap();
    });
    cx.run_until_parked();
    let gate = WriteGate(review.downgrade());
    let write = agent_write("/p/a.txt", cx);
    let outcome = cx.update(|cx| gate.review_write(write, cx)).await.unwrap();
    assert_eq!(outcome.content.as_deref(), Some(NEW));
    let history = review.read_with(cx, |review, _| review.turn_history(turn).to_vec());
    assert_eq!(history.len(), 1, "one file decided once: {history:?}");
}

/// Patterns written the `.gitignore` way are accepted but never match.
#[test]
fn zz_anchored_and_directory_patterns_never_match() {
    let rules = ReviewRules::parse(
        PathBuf::from("/repo"),
        r#"deny = ["/Cargo.lock", "secrets/", "./private/**"]"#,
        true,
    )
    .unwrap();
    let kind = |path: &str| {
        rules
            .evaluate(&Path::new("/repo").join(path))
            .map(|rule| rule.kind)
    };
    assert_eq!(
        (
            kind("Cargo.lock"),
            kind("secrets/key.pem"),
            kind("private/a.txt")
        ),
        (
            Some(RuleKind::Deny),
            Some(RuleKind::Deny),
            Some(RuleKind::Deny)
        )
    );
}

/// Rules are matched case-sensitively, while macOS and Windows file systems are not.
#[test]
fn zz_rules_are_case_sensitive() {
    let rules =
        ReviewRules::parse(PathBuf::from("/repo"), r#"deny = ["*.env", "secrets/**"]"#, true)
            .unwrap();
    let kind = |path: &str| {
        rules
            .evaluate(&Path::new("/repo").join(path))
            .map(|rule| rule.kind)
    };
    assert_eq!(
        (kind("PROD.ENV"), kind("Secrets/key.pem")),
        (Some(RuleKind::Deny), Some(RuleKind::Deny))
    );
}
```

## Appendix B: `ui_ext_host` repros (#3, #4, #10, #17)

`crates/ui_ext_host/src/zz_repro.rs`, with `#[cfg(test)] mod zz_repro;` added to `ui_ext_host.rs`. Run `RUSTC_WRAPPER= cargo test -p ui_ext_host zz_repro`. Result: 4 failed.

```rust
use std::{cell::RefCell, rc::Rc};

use anyhow::{Result, anyhow};
use futures::{FutureExt as _, channel::oneshot, future::LocalBoxFuture};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use ui_ext::{
    ButtonProps, FlexDirection, FlexProps, ListProps, Node, NodeId, Patch, Props, Surface,
    TextProps, Tree, TreeLimits, UiEvent, Update,
};

use crate::{
    Phase, SurfaceState, SurfaceView, UiExtensionRegistry,
    backend::{Reply, UiBackend},
    tests::init_test,
};

const LABEL: NodeId = 2;
const BUTTON: NodeId = 3;
const LIST: NodeId = 4;

/// Mounts `Count v<version>`; `handle_event` waits until the test answers it.
struct GatedBackend {
    version: u32,
    row_count: u32,
    pending: RefCell<Vec<oneshot::Sender<Result<Reply<Update>>>>>,
}

impl GatedBackend {
    fn new(version: u32) -> Rc<Self> {
        Rc::new(Self {
            version,
            row_count: 3,
            pending: RefCell::default(),
        })
    }

    fn answer(&self, result: Result<Reply<Update>>) {
        let sender = self.pending.borrow_mut().remove(0);
        sender.send(result).ok();
    }
}

impl UiBackend for GatedBackend {
    fn extension_id(&self) -> &str {
        "gated"
    }

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        let nodes = vec![
            Node {
                id: 1,
                parent: None,
                props: Props::Flex(FlexProps {
                    direction: FlexDirection::Column,
                    grow: true,
                    ..FlexProps::default()
                }),
            },
            Node {
                id: LABEL,
                parent: Some(1),
                props: text(&format!("v{}", self.version)),
            },
            Node {
                id: BUTTON,
                parent: Some(1),
                props: Props::Button(ButtonProps {
                    label: "Go".into(),
                    ..ButtonProps::default()
                }),
            },
            Node {
                id: LIST,
                parent: Some(1),
                props: Props::List(ListProps {
                    row_count: self.row_count,
                    row_height: 24,
                }),
            },
        ];
        let tree = Tree::from_nodes(nodes, TreeLimits::default()).map_err(anyhow::Error::from);
        async move {
            tree.map(|value| Reply {
                pushed: Vec::new(),
                value,
            })
        }
        .boxed_local()
    }

    fn handle_event(&self, _: &str, _: UiEvent) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        let (sender, receiver) = oneshot::channel();
        self.pending.borrow_mut().push(sender);
        async move { receiver.await.map_err(|_| anyhow!("dropped"))? }.boxed_local()
    }

    fn list_rows(
        &self,
        _: &str,
        _: NodeId,
        _: u32,
        _: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<(u32, Tree)>>>> {
        async move {
            Ok(Reply {
                pushed: Vec::new(),
                value: Vec::new(),
            })
        }
        .boxed_local()
    }
}

fn text(content: &str) -> Props {
    Props::Text(TextProps {
        content: content.into(),
        ..TextProps::default()
    })
}

fn surface() -> Surface {
    Surface {
        id: "main".into(),
        title: "Main".into(),
        kind: ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
            position: ui_ext::DockPosition::Right,
            icon: "hash".into(),
            default_size: 320,
        }),
    }
}

fn describe(state: &Entity<SurfaceState>, cx: &mut TestAppContext) -> String {
    state.read_with(cx, |state, _| match state.phase() {
        Phase::Mounting => "Mounting".to_string(),
        Phase::Failed { message, retryable } => format!("Failed({message}, retryable={retryable})"),
        Phase::Ready(tree) => match tree.get(LABEL).map(|node| node.props()) {
            Some(Props::Text(props)) => format!("Ready({})", props.content),
            _ => "Ready(?)".to_string(),
        },
    })
}

/// A call to the replaced (old) instance that fails after a hot reload stops the new instance.
#[gpui::test]
async fn zz_repro_stale_fatal_reply_after_reload(cx: &mut TestAppContext) {
    init_test(cx);
    let old = GatedBackend::new(1);
    let backend: Rc<dyn UiBackend> = old.clone();
    let state = cx.new(|cx| SurfaceState::new(backend, surface(), None, cx));
    cx.run_until_parked();
    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(BUTTON), cx));
    cx.run_until_parked();
    assert_eq!(old.pending.borrow().len(), 1, "click in flight on v1");

    let new: Rc<dyn UiBackend> = GatedBackend::new(2);
    state.update(cx, |state, cx| state.replace_backend(new, surface(), cx));
    cx.run_until_parked();
    assert_eq!(describe(&state, cx), "Ready(v2)");

    old.answer(Err(anyhow!("wasm trap: interrupt")));
    cx.run_until_parked();
    assert_eq!(
        describe(&state, cx),
        "Ready(v2)",
        "a failure of the replaced instance must not stop the new one"
    );
}

/// A reply from the replaced instance is applied to the new instance's tree.
#[gpui::test]
async fn zz_repro_stale_reply_applied_after_reload(cx: &mut TestAppContext) {
    init_test(cx);
    let old = GatedBackend::new(1);
    let backend: Rc<dyn UiBackend> = old.clone();
    let state = cx.new(|cx| SurfaceState::new(backend, surface(), None, cx));
    cx.run_until_parked();
    state.update(cx, |state, cx| state.dispatch(UiEvent::Click(BUTTON), cx));
    cx.run_until_parked();

    let new: Rc<dyn UiBackend> = GatedBackend::new(2);
    state.update(cx, |state, cx| state.replace_backend(new, surface(), cx));
    cx.run_until_parked();

    old.answer(Ok(Reply {
        pushed: Vec::new(),
        value: Update::Patches(vec![Patch::SetProps(LABEL, text("stale from v1"))]),
    }));
    cx.run_until_parked();
    assert_eq!(
        describe(&state, cx),
        "Ready(v2)",
        "the old instance's reply must not patch the new instance's tree"
    );
}

/// `fail` while a mount is in flight: the mount lands afterwards and revives the surface.
/// Reached through the registry when a reinstall drops a surface (or an uninstall stops an
/// extension) while that surface is mounting.
#[gpui::test]
async fn zz_repro_fail_during_mount_is_undone(cx: &mut TestAppContext) {
    init_test(cx);
    let registry = cx.new(|_| UiExtensionRegistry::default());
    registry.update(cx, |registry, cx| {
        let v1: Rc<dyn UiBackend> = GatedBackend::new(1);
        registry.register_backend(v1, &[surface()], cx);
        // The next version no longer declares "main", before v1's mount has answered.
        let v2: Rc<dyn UiBackend> = GatedBackend::new(2);
        registry.register_backend(v2, &[], cx);
    });
    let state = registry.read_with(cx, |registry, _| {
        registry
            .surface_of("gated", "main")
            .map(|surface| surface.state.clone())
            .expect("registered")
    });
    assert!(describe(&state, cx).starts_with("Failed("), "{}", describe(&state, cx));
    cx.run_until_parked();
    let phase = describe(&state, cx);
    assert!(
        phase.starts_with("Failed("),
        "a stopped surface stays stopped, got {phase}"
    );
}

```

This module goes at the end of `crates/ui_ext_host/src/registry.rs` (#17):

```rust
mod zz_repro_registry {
    use super::*;
    use gpui::TestAppContext;

    const GOOD: &str = "id = \"foo\"\nname = \"Foo\"\nversion = \"0.1.0\"\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"foo.wasm\"\ncapabilities = [\"ui:panel\"]\n";
    // A typo while editing the manifest of a dev extension.
    const TYPO: &str = "id = \"foo\"\nname = \"Foo\"\nversion = \"0.1.0\"\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"foo.wasm\"\ncapabilites = [\"ui:panel\"]\n";

    /// A manifest that fails to parse (or a component missing for a moment) is taken for an
    /// uninstall: the surfaces stop as "uninstalled" and the hot-reload watch is dropped, so the
    /// fixed manifest and the next build are never picked up.
    #[gpui::test]
    async fn zz_repro_transient_read_error_is_an_uninstall(cx: &mut TestAppContext) {
        crate::tests::init_test(cx);
        let fs = fs::FakeFs::new(cx.executor());
        fs.create_dir(Path::new("/installed/foo")).await.expect("dir");
        fs.insert_file("/installed/foo/extension.toml", GOOD.as_bytes().to_vec())
            .await;
        fs.insert_file("/installed/foo/foo.wasm", vec![0, 1, 2]).await;
        let found = find_ui_extensions(Path::new("/installed"), fs.as_ref()).await;
        assert_eq!(found.len(), 1);

        let registry = cx.new(|_| UiExtensionRegistry::default());
        registry.update(cx, |registry, _| {
            let found = &found[0];
            registry
                .installed
                .insert(found.extension_id.clone(), found.fingerprint);
            registry
                .watches
                .insert(found.extension_id.clone(), Task::ready(()));
        });

        fs.insert_file("/installed/foo/extension.toml", TYPO.as_bytes().to_vec())
            .await;
        let found = find_ui_extensions(Path::new("/installed"), fs.as_ref()).await;
        let to_load = registry.update(cx, |registry, cx| registry.plan_scan(found, cx));
        assert!(to_load.is_empty());
        registry.read_with(cx, |registry, _| {
            assert!(
                registry.watches.contains_key("foo"),
                "a manifest that does not parse for now is not an uninstall: the watch stays"
            );
        });
    }
}
```

## Appendix C: `ui_ext_sdk` repros (#11, #20)

`crates/ui_ext_sdk/tests/zz_repro_bugs.rs`. Run `RUSTC_WRAPPER= cargo test -p ui_ext_sdk --test zz_repro_bugs`.

```rust
use ui_ext_sdk::*;

fn to_host(nodes: &[Node]) -> Vec<ui_ext::Node> {
    nodes.iter().map(Into::into).collect()
}

/// A surface grown past 2048 nodes through `insert` (which the SDK and the host accept) cannot be
/// mounted again from the same state, although the host's `mount` accepts up to 16384 nodes.
#[test]
fn zz_repro_grown_tree_cannot_be_remounted() {
    let mut surface = SurfaceTree::new();
    let mounted = surface
        .mount(|tree| {
            tree.column(|_| {});
        })
        .expect("mount");
    let column = mounted[0].id;
    let mut host = ui_ext::Tree::from_nodes(to_host(&mounted), TreeLimits::default())
        .expect("host accepts mount");
    for _ in 0..3 {
        surface
            .insert(column, None, |tree| {
                for index in 0..1000 {
                    tree.text(format!("item {index}"));
                }
            })
            .expect("insert under the cap");
        host.apply_update((&surface.take_update()).into())
            .expect("host accepts each batch");
    }
    assert_eq!(host.len(), 3001);

    // The host remounts (panel reopened, or after a rejected update): rebuild from the same state.
    let host_accepts = ui_ext::Tree::from_nodes(host.to_nodes(), TreeLimits::default()).is_ok();
    assert!(host_accepts, "the host would accept this mount");
    let remount = surface.mount(|tree| {
        tree.column(|tree| {
            for _ in 0..3 {
                for index in 0..1000 {
                    tree.text(format!("item {index}"));
                }
            }
        });
    });
    assert!(remount.is_ok(), "SDK refuses the remount: {remount:?}");
}

/// `apply_event` overwrites the mirror while a `set-props` for the same input is still queued.
/// The host applies the queued patch after the event, so it ends up showing the queued value while
/// the mirror holds the typed one, and a later `set_props` back to the queued value is dropped.
#[test]
fn zz_repro_apply_event_with_pending_set_props_diverges() {
    let mut surface = SurfaceTree::new();
    let mut input = 0;
    let mounted = surface
        .mount(|tree| {
            tree.column(|tree| {
                input = tree.input(InputProps::new("start"));
            });
        })
        .expect("mount");
    let mut host = ui_ext::Tree::from_nodes(to_host(&mounted), TreeLimits::default())
        .expect("host accepts mount");

    // Queued, not yet sent (e.g. the extension batches patches for a later push or reply).
    assert_eq!(surface.set_props(input, InputProps::new("")), Ok(true));

    // The user types: the host writes the value first, then sends the event.
    host.apply_patches(vec![ui_ext::Patch::SetProps(
        input,
        ui_ext::Props::Input(ui_ext::InputProps {
            value: "typed".into(),
            placeholder: String::new(),
        }),
    )])
    .expect("host records typed text");
    let event = UiEvent::InputChanged((input, "typed".into()));
    surface.apply_event(&event);

    // The reply carries the queued patch, which the host applies after the event.
    host.apply_update((&surface.take_update()).into())
        .expect("host accepts the reply");
    assert_eq!(
        host.to_nodes(),
        to_host(&surface.to_nodes()),
        "mirror and host diverged"
    );
}
```

## Appendix D: `fyx_ext` repros (#5, #13, #23–#26)

`crates/fyx_ext/src/zz_repro.rs`, with `#[cfg(test)] mod zz_repro;` added to `fyx_ext.rs`. Run `RUSTC_WRAPPER= cargo test -p fyx_ext zz_repro`.

```rust
use std::path::Path;

use async_zip::base::read::seek::ZipFileReader;
use futures::io::BufReader;

use crate::Report;
use crate::inspect::API_VERSION_SECTION;
use crate::inspect::tests::{component_v0_1, component_v0_2, with_custom_section};
use crate::manifest::MANIFEST_FILE;

fn entry_names(path: &Path) -> Vec<String> {
    smol::block_on(async {
        let file = smol::fs::File::open(path).await.expect("open");
        let reader = ZipFileReader::new(BufReader::new(file))
            .await
            .expect("read zip");
        reader
            .file()
            .entries()
            .iter()
            .map(|entry| entry.filename().as_str().expect("utf-8").to_string())
            .collect()
    })
}

fn write_ext(root: &Path, component: &str) {
    std::fs::write(
        root.join(MANIFEST_FILE),
        format!(
            "id = \"my-panel\"\nname = \"P\"\nversion = \"1.2.3\"\nschema_version = 1\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = {component:?}\ncapabilities = [\"ui:panel\"]\n"
        ),
    )
    .expect("manifest");
    let path = root.join(component);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(
        &path,
        with_custom_section(component_v0_1(), API_VERSION_SECTION, &[0, 0, 0, 1, 0, 0]),
    )
    .expect("component");
}

#[test]
fn zz_repro_component_inside_assets_is_packed_twice() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_ext(dir.path(), "assets/fyx_ui.wasm");
    let package_path = crate::package::package(dir.path(), &dir.path().join("dist")).expect("package");
    let names = entry_names(&package_path);
    let count = names.iter().filter(|n| *n == "assets/fyx_ui.wasm").count();
    assert_eq!(count, 1, "{names:?}");
}

#[cfg(unix)]
#[test]
fn zz_repro_backslash_component_becomes_traversal_entry() {
    let dir = tempfile::tempdir().expect("temp dir");
    let component = r"out\..\..\evil.wasm";
    write_ext(dir.path(), component);
    let (_, report) = crate::check(dir.path()).expect("check");
    let package_path = crate::package::package(dir.path(), &dir.path().join("dist"));
    let names = package_path.as_ref().map(|path| entry_names(path));
    assert!(
        report.has_errors(),
        "check accepted {component:?}; package entries: {names:?}"
    );
}

#[cfg(unix)]
#[test]
fn zz_repro_symlinked_asset_silently_dropped() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_ext(dir.path(), "fyx_ui.wasm");
    std::fs::create_dir_all(dir.path().join("assets")).expect("mkdir");
    std::fs::write(dir.path().join("real.svg"), "<svg/>").expect("write");
    std::os::unix::fs::symlink(dir.path().join("real.svg"), dir.path().join("assets/icon.svg"))
        .expect("symlink");
    let package_path = crate::package::package(dir.path(), &dir.path().join("dist")).expect("package");
    let names = entry_names(&package_path);
    assert!(names.iter().any(|n| n == "assets/icon.svg"), "{names:?}");
}

#[test]
fn zz_repro_v0_2_component_under_0_1_manifest_without_section() {
    let info = crate::inspect::inspect(&component_v0_2()).expect("inspect");
    eprintln!("imports: {:?}\nexports: {:?}", info.imports, info.exports);
    let manifest = crate::manifest::Manifest {
        dir: "ext".into(),
        id: "x".into(),
        version: "0.1.0".into(),
        api_version: Some(semver::Version::new(0, 1, 0)),
        component: "fyx_ui.wasm".into(),
        capabilities: vec!["ui:panel".into(), "ui:editor-decorations".into()],
    };
    let mut report = Report::default();
    crate::inspect::check_component(&manifest, &component_v0_2(), &mut report);
    eprintln!("{report:#?}");
    assert!(report.has_errors(), "{report:?}");
}

#[test]
fn zz_repro_long_name_creates_invalid_manifest() {
    let dir = tempfile::tempdir().expect("temp dir");
    let name = "a".repeat(65);
    let created = crate::template::create(&crate::template::NewExtension {
        name: name.clone(),
        parent: dir.path().to_path_buf(),
        sdk: crate::template::SdkSource::Git {
            url: "https://example.com/fyx.git".into(),
            reference: crate::template::GitReference::Rev("abc".into()),
        },
    });
    if let Ok(crate_dir) = created {
        let mut report = Report::default();
        crate::manifest::read(&crate_dir, &mut report).expect("manifest");
        assert!(report.errors.is_empty(), "fyx-ext new accepted it, check rejects: {report:?}");
    }
}

#[test]
fn zz_repro_git_url_with_backslashes_is_mangled() {
    let dir = tempfile::tempdir().expect("temp dir");
    let url = r"C:\fyx";
    let crate_dir = crate::template::create(&crate::template::NewExtension {
        name: "g".into(),
        parent: dir.path().to_path_buf(),
        sdk: crate::template::SdkSource::Git {
            url: url.into(),
            reference: crate::template::GitReference::Branch("main".into()),
        },
    })
    .expect("create");
    let text = std::fs::read_to_string(crate_dir.join("Cargo.toml")).expect("read");
    let parsed: toml::Value = toml::from_str(&text).expect("Cargo.toml parses");
    let git = parsed["dependencies"]["ui_ext_sdk"]["git"].as_str().map(str::to_string);
    assert_eq!(git.as_deref(), Some(url), "{text}");
}
```

## Appendix E: `fyx_registry` repros (#12, #21, #22), branch `claude/fyx-registry-core`

`crates/fyx_registry/src/zz_repro.rs`, with `#[cfg(test)] mod zz_repro;` added to `fyx_registry.rs`. Run
`RUSTC_WRAPPER= cargo test -p fyx_registry zz_repro`.

```rust
use semver::Version;

use crate::{
    BlocklistCache, Index, InstalledExtension, InstalledVersion, Origin, Permissions, Signed,
    Timestamp, VerifiedIndex,
    blocklist::test_support::{blocklist, entry, verified},
    index::test_support::release,
    plan_updates,
    signing::test_keys::{keyset, registry_key},
};

#[test]
fn zz_repro_one_malformed_entry_discards_the_whole_index() {
    let mut index = Index::new(Timestamp(0));
    index.releases = vec![
        Signed::sign(release("todo", "1.0.0"), &registry_key()),
        Signed::sign(release("lint", "1.0.0"), &registry_key()),
    ];
    let mut value: serde_json::Value =
        serde_json::from_slice(&index.to_json().expect("serializes")).expect("json");
    let sha = value["releases"][1]["signed"]["package"]["sha256"]
        .as_str()
        .expect("sha")
        .to_uppercase();
    value["releases"][1]["signed"]["package"]["sha256"] = serde_json::Value::String(sha);
    let parsed = Index::from_json(&serde_json::to_vec(&value).expect("json"));
    let todo = crate::ExtensionId::new("todo").expect("id");
    let usable = parsed
        .as_ref()
        .map(|index| index.verify(&keyset()).latest(&todo).is_some());
    assert_eq!(usable, Ok(true), "{parsed:?}");
}

#[test]
fn zz_repro_build_metadata_escapes_exact_block() {
    let list = verified(blocklist(1, vec![entry("todo", "=1.2.0")]));
    let extension = InstalledExtension {
        id: crate::ExtensionId::new("todo").expect("id"),
        active: InstalledVersion {
            version: Version::parse("1.2.0+local").expect("v"),
            origin: Origin::LocalFile,
        },
        previous: None,
        pinned: false,
        approved: Permissions::default(),
    };
    let plan = plan_updates(
        std::slice::from_ref(&extension),
        &VerifiedIndex::default(),
        Some(&list),
        Timestamp(0),
    );
    assert!(
        list.is_blocked(&extension.id, &Version::parse("1.2.0+local").expect("v")),
        "=1.2.0 does not block 1.2.0+local; plan = {plan:?}"
    );
}

#[test]
fn zz_repro_fetch_never_due_after_clock_goes_back() {
    let now = Timestamp(1_800_000_000);
    let last_attempt = Timestamp(now.0 + 365 * 24 * 60 * 60);
    assert!(
        BlocklistCache::fetch_due(Some(last_attempt), now),
        "no block list fetch for a year"
    );
}
```

## Appendix F: `fyx_guard` repros (#14–#16, #27, #28), branch `claude/fyx-w9-guard`

`crates/fyx_guard/src/zz_repro.rs`, with `#[cfg(test)] mod zz_repro;` added to `fyx_guard.rs`. Run
`RUSTC_WRAPPER= cargo test -p fyx_guard zz_repro`.

```rust
use std::path::Path;

use util::rel_path::rel_path;

use crate::*;

#[test]
fn zz_repro_windows_equivalent_names_are_not_protected() {
    let missed = [
        "/p/.claude./settings.json",
        "/p/.mcp.json.",
        "/p/.mcp.json ",
        "/p/.mcp.json::$DATA",
        "/p/.git/hooks./pre-commit",
    ]
    .into_iter()
    .filter(|path| protected_config(Path::new(path)).is_none())
    .collect::<Vec<_>>();
    assert!(missed.is_empty(), "not protected: {missed:?}");
}

#[test]
fn zz_repro_project_inside_claude_worktrees_is_entirely_read_only() {
    for path in [
        "/repo/.claude/worktrees/feature/src/main.rs",
        "/home/u/.codex/worktrees/1a2b/repo/README.md",
    ] {
        assert_eq!(protected_config(Path::new(path)), None, "{path}");
    }
}

#[test]
fn zz_repro_repo_patterns_are_case_sensitive_unlike_config_rule() {
    let policy = RepoPolicy::parse(r#"private_files = ["secrets/**"]"#).expect("parses");
    assert!(policy.private_match(rel_path("secrets/prod.json")).is_some());
    assert!(
        policy.private_match(rel_path("Secrets/prod.json")).is_some(),
        "Secrets/prod.json opens secrets/prod.json on macOS/Windows"
    );
}

#[test]
fn zz_repro_submodule_hooks_are_not_protected() {
    let path = "/p/.git/modules/vendor/hooks/pre-commit";
    assert!(protected_config(Path::new(path)).is_some(), "{path}");
}

#[test]
fn zz_repro_denial_names_every_pattern_not_the_matching_one() {
    let policy = RepoPolicy::parse(r#"private_files = ["keys/**", "*.p12", "infra/**"]"#)
        .expect("parses");
    assert_eq!(
        policy.private_match(rel_path("keys/a.json")).as_deref(),
        Some("keys/**")
    );
}
```
