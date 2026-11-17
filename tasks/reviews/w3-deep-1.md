# Judge review: W3 deep pass 1 (renderer, containers, hooks)

Scope: `crates/ui_ext_host` at `integration` `4eef241` (W3 commits `8224c47`, `9f7c042`,
`eb6074c`), plus the W3 hooks in `crates/workspace/src/dock.rs` and `crates/zed`. I checked it
against `fyx_ui.rs` (`Reply<T>`, `ReplyError`, touched ids, `load_from_dir`), the WIT, `tree.rs`, and
the integrator decisions in `integration-1.md` and `integration-2.md`. I read the code and compiled
nothing. Every claim was checked against the code, not against `W3.md`.

## Verdict

The structure is sound. `render()` never calls the extension. Each input event makes exactly one
`handle-event` call, on a serialized worker. Row ids never enter the surface's id space. Colours
come from theme tokens only, and the upstream hooks are minimal and marked. Two contract points are
not met:

- the pushes inside a `ReplyError` are never applied (finding 1);
- a debounced `input-changed` can reach the extension after a later click (finding 2).

Fix both before more extensions are built on this renderer. Everything else is Low. All findings
are owned by W3; finding 5 also asks something of W2.

## Findings, most severe first

### 1. Medium: `ReplyError` pushes are dropped on every path (W3)

`grep -rn ReplyError crates/ui_ext_host` finds nothing. W2 documents the contract at
`fyx_ui.rs:206-211` and in `tasks/progress/W2.md:26-27`: "apply `pushed` like a reply's, then show
the error". The renderer drops those pushes in three places:

- `handle-event`: `surface.rs:253-261` logs the error and sets `last_error`. The pushes are lost.
- `mount`: `surface.rs:147-155` sets `Phase::Failed`. Pushes for *other* surfaces are lost.
- `list-rows`: `surface.rs:412-417` applies `pushed` only on `Ok`. Validation errors from
  `fyx_ui.rs:641-659` (a row outside the range, a row without exactly one root) all become a
  `ReplyError` (`fyx_ui.rs:679-688`). Those pushes are lost in the `Err` arm (`surface.rs:434-442`).

**Scenario.** A panel's click handler pushes `set-props(badge, "3")` to the extension's status item,
then returns `Err("network down")`. The SDK mirror of the status item now holds `"3"`, while the
host still shows `"2"`. The next time the badge becomes `"3"`, the SDK diffs against its mirror and
sends nothing. The badge stays wrong, and nothing triggers a remount. The decision-2 resync only
fires when a later patch happens to name a node the host lacks.

**Fix.** Add one helper, for example
`fn take_reply_error_pushes(error: &anyhow::Error) -> Vec<PushedUpdate>`
(`error.downcast_ref::<ReplyError>().map(|e| e.pushed.clone())`). Call it in all three `Err` arms
before handling the error:

- `apply_reply`: call `apply_pushes`.
- `receive_rows`: call `apply_pushes`, then record `failed_generation`.
- `mount`: route only the pushes for other surfaces. The surface is `Failed`, so its own pushes
  have no tree to apply to.

Test it with the fake: the fake returns `Err(ReplyError { pushed: [set label] })`. Assert that the
label changed and that `last_error` is set.

### 2. Medium: a debounced `input-changed` is overtaken by later events (W3)

`input_edited` (`view.rs:216-228`) defers `InputChanged` by 150 ms. A click goes straight to
`SurfaceState::dispatch`, from `view.rs:447-449` for buttons and `view.rs:603-605` for rows. The
same holds for a submit on another input (`view.rs:243-252`). Nothing flushes the pending debounces
first, so the extension receives the events in a different order than the user made them.

**Scenario.** A form has an input `name` and a button `Save`. The user types `foo` and clicks Save
within 150 ms. The extension receives `click(save)` while its state still holds the old name, and
saves that. Then `input-changed("foo")` arrives. Neither demo has a form, but any extension with a
form will hit this.

**Fix.** In `SurfaceView::dispatch` (`view.rs:88-90`), for every event that is not `InputChanged`,
first flush every input of this view whose `debounce` is `Some` (call `flush_input(id)`, which
already compares against `last_known`). `submit_input` already cancels its own debounce. This
still makes one call per event.

### 3. Low: one rejected own push drops the remaining pushes for other surfaces (W3)

`apply_pushes` (`surface.rs:268-285`) returns at the first own push that fails
(`surface.rs:271-272`). Pushes for other surfaces later in the same list are never routed. Those
surfaces' SDK mirrors already count them as applied, so finding 1's silent desync follows.

**Fix.** After the first own failure, keep looping. Skip the remaining own pushes, still call
`route_push` for every other surface, and return `false` at the end.

### 4. Low: remount loops are not bounded (W3)

The `resyncing` guard (`surface.rs:335-338`) only prevents two remounts at once. If the extension's
fresh tree draws the same invalid push again, the loop restarts with no user input:

- **One surface.** The extension pushes an invalid own patch during `list-rows`. The sequence is
  `receive_rows`, then `apply_pushes` (`surface.rs:415`), then a remount. `set_tree` clears `lists`
  (`surface.rs:166`). The next layout calls `list-rows` again, the same push comes back, and the
  surface remounts again. That is one call per round trip for as long as the list is laid out.
- **Two surfaces.** Surface A's mount reply routes an invalid push to surface B
  (`surface.rs:140-144`), and B's mount reply routes one back to A. This repeats even while both
  panels are hidden, because remounting does not depend on visibility.

Both cases need a buggy extension. However, the rule is that a bad extension cannot cause calls
without input.

**Fix.** Count remounts since the last update that applied. After 3, set
`Phase::Failed("the extension keeps sending updates that do not fit its tree")` and stop. An
accepted update or a user retry resets the count.

### 5. Low: an extension that is disabled or whose mount failed has no terminal state (W3, W2)

- **Disabled by the time budget.** W2 marks the instance unusable (`fyx_ui.rs:719-721`), and every
  later call fails with `disabled after an earlier failure`. The renderer handles that like any
  other `handle-event` error (`surface.rs:253-261`). The panel keeps its last tree, and its buttons
  and rows stay live. Each click makes a call that is refused at once and replaces the footer
  error. Status items show nothing at all, because the footer is suppressed when
  `compact = true` (`view.rs:346`).
- **Mount fails.** `Phase::Failed` (`surface.rs:153`) has no retry. The panel shows the error
  until restart.

No tasks leak and nothing loops in either case. I checked that `failed_generation` stops row
retries (`surface.rs:362-364`).

**Fix.** An error that is not a `ReplyError` means the instance is dead: a trap, the budget, or an
earlier disable (`fyx_ui.rs:210-211`). W3 sets
`Phase::Failed("<extension> was disabled: …")` on every surface of that backend, through the
registry, which stops all dispatching. W2 can make this explicit with a typed `Disabled` error.
Later, add a "Retry" action on `Failed` that calls `mount`.

### 6. Low: one remembered answered range against three requests per layout (W3)

`uniform_list` calls the row callback three times per frame:

- with `0..1` from `measure_item` in `request_layout` (`gpui/src/elements/uniform_list.rs:283`,
  `:671-672`);
- with `0..1` again in `prepaint` (`:359`);
- with the visible range (`:492`).

`render_rows` turns each call into a `request_rows` (`view.rs:264-266`). With overscan, the
measuring call asks for `0..9`. `RowCache::answered` keeps only the last range
(`surface.rs:366-376`, `:422`). This has two effects:

- **Two calls per change.** When the list is scrolled away from the top, each invalidation makes
  two `list-rows` calls, one for `0..9` and one for the visible range. The "once per change" claim
  in `W3.md` holds only at the top.
- **Endless calls on short answers.** If the extension answers short both near the top and in the
  visible range (rows it omits), the two ranges replace each other in `answered` on every
  `receive_rows`. Each `cx.notify()` triggers the next fetch, so the calls never stop. That is the
  "call per frame" the guard is meant to prevent. A second window with another scroll position
  loops the same way, even without the measuring call.

**Fix.**
- Measure an item inside the visible range:
  `.with_width_from_item(Some(scroll_handle.logical_scroll_top_index()))` in `virtualized`
  (`view.rs:560-578`). The handle is already there.
- Replace `answered` with markers per index: store an entry of the current generation even for a
  row that was omitted, for example `CachedRow { generation, tree: Option<Tree> }`, and count an
  index as satisfied when it has a current entry.

### 7. Low: queued events are not checked again before they are sent (W3)

`dispatch` drops an event whose node is gone only when the event is queued (`surface.rs:177-180`).
The worker sends whatever it pops (`surface.rs:195-204`), even though an earlier reply in the
queue may have changed the tree. The worst case is `row-click`:

**Scenario.** In the todo demo, the user clicks row 2 while a filter reply is still in flight. The
reply changes `visible`, and then `row-click(list, 2)` selects a different item from the one the
user clicked (`fyx_demo_todo.rs:464-469`). A double click on a button whose reply removes it also
sends a second `click` for a removed node.

**Fix.** In the worker, drop the event if `!tree.contains(event.target())`. For `RowClick`, store
the list's row generation when the event is queued, and drop the event when the generation has
changed by the time it is sent.

### 8. Low: a remount overwrites text typed less than 150 ms earlier (W3)

`set_tree` counts every input as set by the extension (`surface.rs:164`, `:462-467`). On the next
sync, `sync_input` writes the mounted value into the editor and cancels the pending debounce
(`view.rs:205-212`). Text typed within the last 150 ms before a remount, or before a `replace`,
which touches every id, is therefore lost and never sent.

**Fix.** Before writing a value into an editor whose debounce is pending, flush it: call
`flush_input` first, then apply the value, or skip the write when the editor holds unsent text and
the set came from a mount. This is rare. It is listed because it silently loses input.

## Checked and correct

- **No WASM call from render or paint code.** `SurfaceView::render` (`view.rs:281-347`) and
  `Renderer` only read `SurfaceState`. Only one path starts a call from element code: the
  `uniform_list` callback, then `render_rows`, then `request_rows`, then `cx.spawn` of a
  `list-rows` (`surface.rs:391-401`). The future is lazy and runs on the extension's queue, and
  nothing awaits it inside the frame. `request_rows` does not call `notify`. With one fetch in
  flight and one queued per list, this is the lazy pull the architecture asks for ("like
  `uniform_list`"). Finding 6 covers where its guard leaks.
- **One `handle-event` per event.** One worker per surface (`surface.rs:191-218`), calls in order,
  each reply applied before the next pop. A queued `input-changed` for the same node is replaced,
  not sent twice (`surface.rs:184-189`). Typing is debounced 150 ms (`view.rs:224-227`).
  `menu::Confirm` sends `input-submitted` and cancels the debounce. In the real keymap,
  `default-windows.json:18` binds a context-free `enter` to `menu::Confirm`. `Editor && mode ==
  full` is the only `Newline` binding, so single-line inputs reach the action. The test binds the
  key itself (`tests.rs:253`). Hover is style only (`view.rs:602`). Scrolling never sends
  `handle-event`; it only pulls rows.
- **Order of `Reply.pushed`.** `handle-event` applies `pushed`, then `value`, and drops `value` once
  an own push fails (`surface.rs:248-251`). `list-rows` applies the pushes before storing the rows
  (`surface.rs:412-417`). `mount` drops its own pushes as superseded and routes the others before
  `set_tree` (`surface.rs:137-145`). Pushes made outside a call are only the instantiation ones
  (`fyx_ui.rs:498-502`). The registry batches them per wake-up (`registry.rs:122-138`). They arrive
  while the surface is `Mounting`, are dropped (`surface.rs:291-297`), and are superseded by
  `mount`, which resets the SDK mirror.
- **Remount on a rejected update** (decision 2) works, including events queued during the resync.
  An event sent before the remount is answered before it, because the extension's queue is
  serialized, and the fresh tree replaces whatever that answer did.
- **No re-entrant update.** `route_push` updates the registry and then another surface's state
  (`registry.rs:98-108`, `:141-157`), only for a different `surface_id`. A duplicate extension id
  is skipped at registration (`registry.rs:80-83`), so no update re-enters itself.
- **Colours** come only from `cx.theme()`: `token_hsla` (`view.rs:779-794`), `text_color`, the
  border, `editor_background`, `panel_background`, `ghost_element_hover`, and the status
  created/deleted colours. No colour from the extension is used.
- **Icon fallback.** `icon_name` (`view.rs:750-752`) parses snake_case through strum `EnumString`
  (`icons.rs:7-9`), with `Blocks` as the fallback. It is used for buttons, icons and panel icons
  (`panel.rs:41`).
- **Virtualized lists.** Rows are fetched lazily for the visible range plus 8. They are cached per
  `(list, index)` outside the surface tree (`surface.rs:28-49`). A touched list or table bumps its
  generation (`surface.rs:305-311`). A removed list loses its cache (`surface.rs:317-321`). Inputs
  and markdown in rows are read-only labels, and `sync` walks only the surface tree
  (`view.rs:105-123`). Buttons in rows get no handler (`view.rs:446-451`). `ButtonLike` stops
  propagation only when it has a handler, so the click reaches the row's `row-click`
  (`view.rs:603-605`). Old-generation rows stay visible until replaced; this is a deliberate
  deviation from "drop" and is fine.
- **Panel lifecycle.** Closing the dock only hides the panel: the state and tree are kept and
  nothing remounts on reopen. Closing a window drops the panel's view, editors, markdown entities,
  scroll handles and debounce tasks. The detached tasks end on their own: the event worker when its
  queue is empty or the entity is gone, row fetches after one call, and the push drain when the
  channel closes after instantiation. The workspace observer of the registry
  (`registry.rs:207-213`) is detached, but it holds only a weak workspace handle, so it does not
  leak. Each surface is mounted at load (`surface.rs:92`), not when it is first shown as the WIT
  says. That works, but every status item and panel costs one `mount` at startup.
- **Fork discipline.** `dock.rs:550-555` adds `panel_index_for_id`, five lines marked `fyx:`.
  `main.rs:756` adds `ui_ext_host::init(cx); // fyx: …`. `crates/zed/Cargo.toml:220` is marked.
  `zed.rs` has no static panel list. Activation priorities start at 100, clear of the built-ins
  (0–7), so the debug-only duplicate-priority panic in `dock.rs:794-801` cannot fire.

## For the gatekeeper

Merge W3 as is. Ask W3 for findings 1 and 2, with a test each, in its next push. Findings 3–8 can
be fixed together in one follow-up.
