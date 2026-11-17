# Performance review 1: Fyx's own code

Scope: `fyx/main` at `571c682` (`ui_ext`, `ui_ext_host`, `ui_ext_wasm`, `agent_review`, the
`fyx_review` hooks in `acp_thread`, `fyx_ext`), plus `crates/fyx_guard` and its hooks from
`claude/fyx-w9-guard` at `6775a87`, and `crates/fyx_registry` from `claude/fyx-registry-core` at
`a5007f7` (neither is merged yet). Upstream Zed code is out of scope except the `fyx:` hooks. No
product code changed. Correctness, security, tests and UX are other reviewers' lanes; cross-lane
items get one line at most.

## How it was measured

All benches were throwaway: `#[ignore]` integration tests under `crates/*/tests/perf_scratch.rs`
plus standalone crates in a scratch directory. None of them are committed. The machine has 4 cores
and 15 GB of memory, and other builds were running on it, so treat the numbers as ±30%.

| Bench | Build | What it runs |
|---|---|---|
| `ui_ext` tree | `release` | `Tree::from_nodes`, `apply_patches`, `replace` and clone on 16384-node trees |
| `ui_ext_wasm` | `test` (dev), but `wasmtime` and `cranelift-*` build at opt-level 3 in dev (`Cargo.toml` `[profile.dev.package]`) | a throwaway guest (`ui_ext_sdk`, 80 KB) that mounts 16384 nodes and answers events with 0, 1, 2048 or 16384-node updates; the debug `fyx_demo_counter` and `fyx_demo_gallery` |
| `ui_ext_host` renderer | dev **and** `release-fast` (no debug info) | `SurfaceView` in a test window over a fake backend: 1024, 4096 and 16384 texts or buttons, and a 1M-row list. A frame is `window.draw()`. |
| `agent_review` diff | `release`, standalone | a copy of `proposed_edit::{compute_hunks, line_hunks, apply_hunks}` over `imara-diff 0.2.0` Histogram, which is what `language::line_diff` runs, on a 50k-line (2.8 MB) file |
| review log | `release`, standalone | the `log::info!` in `AgentReview::note_changed_paths` |

Renderer numbers (`ui_ext_host`, dev build, so gpui/ui/taffy-free code is unoptimized; a release
run is pending and will replace these):

| Tree | open view (sync + first frame) | one frame, nothing changed | click → 1 `set-props` → sync + frame |
|---|---|---|---|
| 16384 texts (127 flex groups) | 1.83 s | 1.28 s | 1.28 s |
| 16384 buttons | 5.89 s | 5.62 s | 6.58 s |
| list node with 1M rows | 31 ms | 1.9 ms | 6 µs (no visible change) |

## Findings, by impact

### 1. High: a surface re-renders its whole tree whenever anything inside it changes, including a caret blink

`view.rs:444-552` (`SurfaceView::render`) and `view.rs:628-678` (`Renderer::node`, `Renderer::flex`).

GPUI views are immediate-mode, and the dock caches the panel (`workspace/src/dock.rs:1377`
`.cached(...)`), so a panel that nothing touches costs nothing. But
`Window::mark_view_dirty` (`gpui/src/window.rs:2479`) marks every ancestor of a view that notifies
as dirty. `SurfaceView` is the ancestor of every input editor (`view.rs:317`), every diff-view
editor (`view.rs:1094`) and every markdown entity it hosts. So each of these re-renders the full
tree: every flex, label and button element, plus a Taffy layout of all of them.

- every caret blink of a focused surface input, about every 500 ms. `BlinkManager` notifies the
  editor (`editor.rs:2563`).
- every keystroke in a surface input.
- every reply that changes one node (`surface.rs:497` `cx.notify()`).

**Evidence.** Dev build: one full frame of a 16384-text surface takes 1.28 s, of a 16384-button surface 5.6 s (table above); a 1M-row `list` takes 1.9 ms, so the cost is the non-virtualized tree. The cost grows linearly with the node count, and a button costs about
four times as much as a text (a `Button` plus its focus ring `div` plus a listener).

**User-visible effect.** A panel built with plain children instead of a `list` drops frames while
the user types into its filter box. At a few thousand nodes it stutters, and near the 16384 cap it
freezes. Nothing warns the extension author: `TreeLimits` allows it.

**Fix sketch.** Render big subtrees as their own cached views. Split the surface into
`Entity<SubtreeView>`s, one per root child and one per flex node with more than about 256
descendants. Render each with `AnyView::cached(...)` and notify only the views whose node ids are
in the update's `touched` set (`Tree::apply_update` already returns it). A caret blink then
re-renders the root and the one path down to the input, and reuses everything else. At the very
least, `fyx-ext check` and the SDK docs should warn when a mount has more than about 1000 nodes,
and point at `list`.

### 2. High: review mode off still diffs every edit permission request (v1) and opens and reads the file (v2)

`agent_review.rs:968-1062` (`on_thread_event`), `agent_review.rs:1298-1335` (`permission_preview`),
`agent_review.rs:1244-1266` (`v2_patch_preview`), and the bail-out that comes too late at
`agent_review.rs:789`.

For every `ToolAuthorizationRequested`, `on_thread_event` builds the full `ProposedEdit` before it
learns that review is off:

- **v1:** `permission_preview` clones `old_text` and `new_text` and runs `ProposedEdit::new`, which
  is the whole line diff plus two full-text reconstructions. It does this synchronously, on the main
  thread, inside the thread's event dispatch. Then `open_permission_preview` returns `None` at
  `:789` because `!self.enabled && !strict_rule`, and the work is dropped.
- **v2:** it clones the patch text, spawns a task, calls `project.open_buffer` (which loads the file
  from disk if it is not open), copies the whole buffer text, applies the patch, diffs, and drops it
  all.

**Evidence** (standalone release bench of the same code, 50k-line file):

| Edit | `compute_hunks` |
|---|---|
| 1 inserted line | 9-12 ms |
| full rewrite | 6-12 ms |
| 1000 scattered hunks | 109-118 ms (`line_diff` alone: 92 ms) |

**User-visible effect.** An agent that sends whole-file diffs (v1 `Write`-style tools) on large
files stalls the UI for 10-120 ms per permission prompt, with review mode **off**. The design says
that with review off, Fyx behaves as upstream at near-zero cost.

**Fix sketch.** Decide before building. `rule_for(path)` needs only the path, so take the path from
the tool call, compute the rule, and return when `!enabled && !matches!(rule, Deny | AlwaysAsk)`.
Only then clone the texts or open the buffer. Do the same in `v2_edit_request`, before
`patch.text.clone()` and the spawn.

### 3. High: with review on, the diff runs on the main thread, up to three times per write

`agent_review.rs:867` (`review_write` → `ProposedEdit::new`), `agent_review.rs:895-898` and
`proposed_edit.rs:207-252` (`adopt_decisions`: `match_indices` over the file, `format!` of the whole
file, and a second `compute_hunks`), and `agent_review.rs:1315` (the preview).

**Evidence.** The timings are the ones in finding 2. An `adopt_decisions` over a 50k-line file
costs another 7-9 ms. A previewed v1 edit therefore costs the preview diff, then the write's diff,
then the placement diff: up to about 250 ms of main-thread time for one agent edit to a
heavily-edited 50k-line file.

**User-visible effect.** Typing and scrolling freeze while a large agent edit arrives.

**Fix sketch.** `review_write` already returns a `Task`, so run `ProposedEdit::new` and
`adopt_decisions` in `cx.background_spawn` and insert the review when they finish. The rule check
and the `enabled` check stay synchronous. `ProposedEdit` is plain data (`String`s and `Vec<Hunk>`),
so it can move to the background thread. For the preview path, spawn from `on_thread_event` the same
way.

### 4. Medium-High: the review log formats every unreviewed path on every change, which is O(n²)

`agent_review.rs:419-436` (`note_changed_paths`).

Each time a worktree event adds a path, it logs the **whole** `BTreeSet` at `info`, which Zed writes
to `Zed.log` by default.

**Evidence** (standalone release bench):

| Files changed during one turn | Formatting time on the main thread | Log text written |
|---|---|---|
| 2000, one per event | 0.63 s | 96 MB |
| 5000, in batches of 10 | 0.35 s | 60 MB |
| 5000, one per event | 4.2 s | 605 MB |

**User-visible effect.** An agent runs a formatter, a code generator or a `git checkout` through
the terminal while review mode is on. The main thread stalls in total for seconds, and the log
grows by hundreds of MB. It also builds a `HashSet<PathBuf>` of every reviewed path on each event
(`:406-416`), which is O(history) per event.

**Fix sketch.** Log only the count and the paths that are new in this event, at `debug`. Keep the
reviewed-path set incrementally per turn instead of rebuilding it.

### 5. Medium: `SurfaceView::sync` is quadratic in buttons, lists, inputs and markdown nodes

`view.rs:208` (`button_focus.retain(|id, _| buttons.contains(id))`), `:214-220`, `:221-225`,
`:231-240`, `:249-250`, `:286-287`, `:302`, and `keyboard.rs:328` (`restore_focus`:
`focus_order.contains` per candidate).

Every revision, which means every event reply that changes something, runs `Vec::contains` or
`iter().any` inside a `retain` over a map of the same size.

**Evidence.** Dev build, 16384 buttons: a click costs 6.58 s against 5.62 s for a frame alone, so sync adds about 1 s, most of it the `buttons.contains` retain (16384 × ~8192 comparisons). Opening the view (one sync) adds about 0.3 s over a frame.

**User-visible effect.** Each click on a panel with thousands of buttons adds a visible delay on top
of finding 1.

**Fix sketch.** Collect `buttons`, `lists`, `inputs` and `markdown` into `HashSet`s or `HashMap`s
(`collections::HashSet`), and look up `focus_order` in a set in `restore_focus`. The change is
mechanical.

### 6. Medium: every update clones the whole retained tree

`tree.rs:196-205` (`apply_patches`: `let mut next = self.clone()`), also reached from
`surface.rs:397-401` (`set_input_locally`, after every typing pause).

**Evidence** (release, 16384 nodes):

| Operation | Time |
|---|---|
| `Tree::clone` | 1.03 ms |
| one `set-props` | 0.98 ms (all of it the clone) |
| 1000 single-patch updates | 0.97 s total |
| a 2048-patch update | 1.75 ms |
| a 200-char text per node | 1.23 ms per update |
| a tree holding a 2×1 MiB `diff-view` | 0.149 ms extra per update, for an unrelated `set-props` |

The comment at `tree.rs:196` says this is fine because updates are per event, not per frame. That
is true, but a push-heavy extension (a status ticker, a log tail with 64 pushes per call) pays
about 1 ms per update at the cap. Large `diff-view`/`markdown` strings are copied on every update.

**Fix sketch.** Apply in place with an undo log: record the inverse of each patch (the old props
for `set-props`, the removed subtree for `remove`, the id for `insert`) and replay it in reverse on
error. Alternatively, keep `Props` behind `Arc` (`Arc<str>` for text), so a clone copies pointers
only. That also removes the per-sync clones in finding 12.

### 7. Medium: the UI epoch thread wakes 100 times a second for the rest of the process

`ui_ext_wasm.rs:452-465`.

The first UI extension load starts a `std::thread` that sleeps 10 ms, increments the engine epoch
and locks `DEADLINE_WAKERS`, forever. The engine lives in a `static OnceLock`
(`ui_ext_wasm.rs:379`), so `weak_engine.upgrade()` never fails and the thread never exits, even
when no call is in flight or every UI extension has been uninstalled. Upstream's extension epoch
ticks at 100 ms.

**User-visible effect.** An idle Fyx wakes the CPU 100 extra times a second (macOS "Energy Impact",
battery). The work per wake is tiny, but the wakeups are what costs.

**Fix sketch.** Tick only while calls are in flight. Keep a counter of running UI calls; the thread
parks on a `Condvar` (or `std::thread::park`) at zero and the first `begin_call` unparks it. The
budget semantics stay the same, because epochs only matter while guest code runs.

### 8. Medium: every UI component is recompiled at every startup, one after another

`ui_ext_wasm.rs:55-87` (an in-memory `moka` incremental cache only), `ui_ext_wasm.rs:554`
(`Component::from_binary`), `registry.rs:305-306` (`scan_installed` awaits each
`load_from_dir` in turn).

**Evidence** (dev test binary, but cranelift and wasmtime at opt-level 3):

| Component | Load (compile + instantiate + `surfaces`) |
|---|---|
| release guest, 80 KB | 114 ms |
| the same guest again in the same process (incremental cache) | 71 ms |
| debug `fyx_demo_counter`, 4.2 MB (as `script/fyx-build-demos` builds it) | 349 ms, 213 ms again |
| debug `fyx_demo_gallery`, 4.4 MB | 380 ms |

The compilation runs on the background executor, so the main thread is not blocked. But each panel
shows "Loading…" until its extension's turn comes. With N extensions the last one waits for the sum
of all compile times, and the compile competes with the rest of startup for cores. None of it is
cached across restarts. Upstream's `extension_host` has the same gap; this is parity, not a
regression.

**Fix sketch.** Cache `Component::serialize()` output under the Fyx data dir, keyed by
`sha256(wasm) + wasmtime version + engine config`, and load it with `Component::deserialize_file`.
The key comes from the fingerprint that `registry.rs` already computes, plus a hash. A dev
reinstall changes the key. Load the found extensions concurrently (`join_all` over
`load_from_dir`) instead of one after another.

### 9. Medium-Low: the write gate copies the whole file on every agent write, even with review off

`acp_thread/src/fyx_review.rs:62-68` (`old_text: snapshot.text()`), with the early return at
`agent_review.rs:861-866`.

The gate is installed unconditionally (`agent_review.rs:136`). So every ACP `fs/write_text_file`
materializes the buffer rope into a `String` and makes three extra `cx.update` hops before
`review_write` sees `!enabled` and passes the text through. That is a 2.8 MB copy for a 50k-line
file, about 1-3 ms.

**Fix sketch.** Add `fn wants_review(&self, abs_path: &Path, cx: &App) -> bool` to
`AgentWriteGate`, or pass the `BufferSnapshot` and let `review_write` call `.text()` only when it
reviews.

### 10. Medium-Low: decorations do editor-wide work with no decorations, and on every decoration change

`decorations.rs:167`, `:235-277` and `:281-420`.

- `sync_editor` runs for every full-mode editor on `TitleChanged`. Upstream emits that on every
  `BufferDiffChanged`, which follows each edit in a git-tracked file, and on every edit of a buffer
  without a file (`editor.rs:10132`, `:10285`). It also runs on `BufferRangesUpdated`, which
  arrives repeatedly while a project search fills its multibuffer. Each run collects every buffer,
  allocates `full_path` for each, takes a multibuffer snapshot and moves the editor's `placed` map
  out and back. There is no early return when `decorations.decorations` is empty, so this runs in
  every Fyx session. For a 1000-buffer search multibuffer, every batch costs O(buffers).
- `cx.observe(&state, |_, _, cx| cx.notify())` (`:167`) re-notifies `Decorations` on every
  decoration `SurfaceState` change: every reply and every row fetch. Every full editor in every
  window then runs `sync_editor`.
- The block's `SurfaceView` is a child of the editor element (`:380-389`), so it re-renders on
  every editor frame (every keystroke and caret blink in that editor), which is finding 1 again.
  Blocks are clamped to 40 lines, so the trees are usually small.

**Fix sketch.** Return at the top of `sync_editor` when there are no decorations and nothing is
placed in this editor. Notify `Decorations` only when `estimate_lines` of that decoration actually
changes. Decorations are a dev-only data source today (`FYX_UI_DEV_DECORATIONS`), so only the first
point affects users now.

### 11. Low: sibling operations are linear in the sibling count

`tree.rs:279-289` (`insert` with `before`: `position` + `Vec::insert`) and `tree.rs:311-313`
(`remove`: `retain` over the siblings).

**Evidence** (release):

| Update | Time |
|---|---|
| remove 2048 of 16383 siblings | 13.5 ms (about 12 ms of it quadratic) |
| insert 2048 nodes before the first of 14335 siblings | 5.4 ms |

Both are bounded by `max_nodes_per_update × max_nodes` (2048 × 16384), so this worst case cannot
grow. Everything else in `Tree` is linear: validation is O(1) per node. The SDK's mirror has the
same shape.

**Fix sketch.** Only if extensions hit it: collect the removals of one update per parent and do one
`retain` against a set.

### 12. Low: diff-view and markdown props are copied and compared in full on every sync

`view.rs:188`, `:200` and `:275-279`.

`sync` clones every `diff-view`'s `old_text` and `new_text` and every markdown source on each
revision, then compares them in full against the stored props. Inside list rows, `line_diff`
(`view.rs:976-1006`) re-splits both texts and `format!`s every line on every frame. The cost is
O(text) per change; it is noticeable only with MB-sized diffs.

**Fix sketch.** Compare by touched id: only nodes in `touched` can have changed props. `Arc` props
(finding 6) make the remaining clones free.

### 13. Low: memory per review is 2-3× the file, plus the pane's buffers, with several transient full copies

`proposed_edit.rs:59-65` (`old_text`, `new_text`, plus each hunk's own `old_text` and `new_text`),
`review_pane.rs:390` (`review.edit().clone()` just to read it), `:398-399` (two more `to_string()`s
for `Diff::finalized`), `agent_review.rs:641` and `:699` (`review.edit.clone()` at submit).

**Evidence** (50k lines, 2.8 MB):

| Edit | `ProposedEdit` size |
|---|---|
| 1000 scattered hunks | 5.6 MB |
| full rewrite (the hunk texts repeat the whole file) | 9.3 MB |

The pane's `Diff` adds another old+new pair in buffers. Reviews are dropped on submit or close, and
the 16-turn history (`ENDED_TURNS_KEPT`) holds only paths and resolutions, so nothing leaks. It is
just heavy for large files.

**Fix sketch.** Store hunk texts as byte ranges into `old_text`/`new_text`. Read the edit by
reference in `show_review`. Move the edit out of the `Review` on submit instead of cloning it.

### 14. Low: status items re-render on every status bar render

`panel.rs:347-363`.

`ExtensionStatusItem` renders its `SurfaceView` uncached. The status bar re-renders whenever any
item in it notifies, for example the cursor-position item on every selection change. A status tree
is usually a handful of nodes, so the cost is small. With an expensive status tree (a markdown
node, a row of icons), every cursor move pays for it.

**Fix sketch.** `self.view.clone().into_any().cached(StyleRefinement::default())` in
`ExtensionStatusItem::render`. The status item's own notifications still re-render it.

### 15. Low: `count_all_inputs` clones the whole tree to find inputs

`surface.rs:714-720`.

It calls `tree.to_nodes()`, a full props clone (1.2 ms at 16384 nodes), on every mount and remount,
only to find the input ids.

**Fix sketch.** Walk `tree.get(id)` from the roots, as `sync` does.

## Fine as is (checked)

- **List virtualization works.** `render_rows` asks only for the visible range plus 8 rows of
  overscan (`surface.rs:723-729`). One layout's requests merge into one `list-rows` call
  (`RowCache::wanted`). A requested range is capped at 256 rows and the cache is pruned past 1024.
  The width measurement uses the top visible row (`view.rs:856-862`). Measured: a 1M-row list renders in 1.9 ms per frame (dev build).
- **Host↔WASM call overhead is low.**

  | Call | Time |
  |---|---|
  | `handle-event` round trip, foreground → tokio → guest → back (unchanged reply) | 57 µs |
  | 1 `set-props` | 70 µs |
  | 2048 `set-props` | 1.7 ms |
  | `replace` of 16384 nodes | 12 ms |
  | `mount` of 16384 nodes | 37 ms (first call) |
  | `list-rows` of 40 rows × 3 nodes | 0.34 ms |

  The host side of these numbers is a dev build, so release is lower. Each call allocates a few
  small things (a surface-id `String`, a boxed closure, a oneshot); that is noise next to the tokio
  hop. The `From<wit::*>` conversions move strings rather than copy them, so a lifted update is
  copied once, by wasmtime. Push accounting is O(1) per push.
- **Store and instance creation happen once per extension load**, not per call
  (`ui_ext_wasm.rs:586-643`). Nothing wasm-related runs at startup without a UI extension:
  `ui_runtime()` is lazy, and the scan only reads `extension.toml` files and stats the components.
  Fingerprints are `mtime + len`, so no whole-package hashing.
- **fyx_guard with mode off costs almost nothing.**
  - Each hook (`check_private_path`, `check_config_write`, `check_agent_edit`, `scrub_env`) reads
    the settings global and returns before allocating anything.
  - `scrub_env` returns before it enumerates the process environment, and `load_policy` returns
    before any IO.
  - Its `on_project_event` still scans each worktree event's changes for `.fyx/security.toml`
    (O(changes), string compares only). An early `mode == Off` return would make it free.
  - With a mode on, `scrub_env` enumerates the process environment once per terminal spawn, which
    is fine.
- **`acp_thread` `fyx_review` hook without a gate** is a single `try_global` (with Fyx's gate always
  installed, see finding 9).
- **Keyboard interceptor.** It runs on every keystroke in the app, but costs only a scan of the key
  context stack (`keyboard.rs:46-73`). `focused_node` is O(buttons) per Tab, which is fine.
- **Review pane.** The hunk controls do an O(hunks) `hunk_drawn_at` per visible hunk per frame;
  `hunk_at_new_row` with 1000 hunks takes 0.7 µs. `update_current_hunk` on selection change is
  cheap. A comment keystroke re-renders the diff editor (`review_pane.rs:322`); that is acceptable,
  though it could skip the editor notify for `set_comment`. `ReviewStatus` re-renders on every
  `AgentReview` notify, which is cheap.
- **`fyx_ext`.** `dev` polls the source files every 500 ms with `walkdir` and skips `target/` and
  dot-directories (`build.rs:234-262`), which is fine for an extension crate. `package` reads each
  file once into the zip. `check` validates the component once.
- **`fyx_registry`.** Each package is hashed once per `verify_package`. Index verification is one
  ed25519 check per release, plus a duplicate count in a `BTreeMap`. Comparing blocklists encodes
  each canonically once. The CLI reads the package into memory, which is fine with the 50 MiB cap.
  When a client lands, verify the cached index once per fetch, not at every startup.
- **The 16-turn history** holds paths and resolutions only. `turn_unreviewed` sets go with their
  turn.
- **Tree validation** is linear. `from_nodes` takes about 5 ms for 16384 nodes and `from_row` about
  0.27 ms for 2048. Apart from finding 11, nothing is quadratic.

## Suggested order

1. Finding 2, finding 9 and the early return in finding 10: small diffs that make "off" cost nothing.
2. Findings 3 and 4.
3. Findings 5 and 14: mechanical.
4. Finding 1: the cached-subtree design, the one larger piece of work. Findings 6 and 12 fit with it.
5. Findings 7 and 8.
