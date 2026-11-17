# `fyx:ui` 0.2 candidates (integrator backlog)

`fyx:ui@0.1.0` stays frozen for the sprint. Every item below has a working workaround today; none
blocks Phase 1–4. Decide them together after Phase 4, as one 0.2 world.

| # | Change | Why | Today's workaround | Source |
|---|---|---|---|---|
| 1 | `invalidate-rows(node-id)` patch | rows change while the list's props do not | re-key the list node (remove + insert); host re-requests rows of touched lists | judge integration-1 #5, W4, W8 |
| 2 | `request-tick(ms)` import + `tick` export | `push-update` cannot drive timers: a component runs only inside a host call | none (no clocks/progress status items) | judge integration-1 #9 |
| 3 | Document row-local ids in the WIT | ids inside `list-row` subtrees are row-local, not surface-unique; clicks in rows are `row-click` | host never maps row ids; SDK `ROW_NODE_ID_BASE` | judge integration-1 #4 |
| 4 | Frozen icon list instead of upstream `IconName` strings | extension ABI must not follow upstream's icon enum | host parses `IconName::from_str`, fallback icon | judge integration-1 #12 |
| 5 | `surface-kind::modal` | QuickPick / InputBox | panel at the bottom dock | W8 |
| 6 | Runtime add/remove of surfaces | VS Code creates pickers/output channels after `surfaces()` | sidecar notification `fyx/surfacesChanged` | W8 |
| 7 | `ui-event::dismissed` | user closed a surface | sidecar request `fyx/surfaceClosed` | W8 |
| 8 | `indent` prop on rows, `tooltip` on button | tree views, icon buttons | flattened labels, no tooltip | W8 |
| 9 | Editor decorations: `host.place-decoration(id, location, nodes)`, `remove-decoration(id)`; `id` acts as a surface id for `push-update`/`handle-event` | layer 4: blocks under buffer lines, anchored by the host so they survive edits | host-side `ui_ext_host::Decorations` API + dev file `FYX_UI_DEV_DECORATIONS`; no extension can place one | W6 |

Details for 5–8: `docs/vscode-coverage.md` ("Schema gaps").

## 9. Decorations (W6 proposal)

```wit
// in `types`
enum decoration-placement { above, below }

record decoration-location {
    /// Worktree-relative (`src/main.rs`) or absolute. Matched in every open editor.
    path: string,
    /// Zero-based buffer row at the time of the call. The host turns it into a CRDT anchor per
    /// buffer; from then on the block moves with the line and the extension never sees rows.
    line: u32,
    placement: decoration-placement,
    /// Block height in editor lines; `none` lets the host estimate it from the tree.
    height-lines: option<u16>,
}

// in `host` (capability `ui:editor-decorations`)
/// Places (or moves, if `id` exists) a decoration. `id` is a surface id: its tree is patched with
/// `push-update(id, ...)`, and clicks in it arrive as `handle-event(id, ...)`.
place-decoration: func(id: string, location: decoration-location, nodes: list<node>) -> result<_, string>;
remove-decoration: func(id: string);
```

Why imports and not a `surface-kind`: surfaces are declared once by `surfaces()`, decorations come
and go with analysis results (one per diagnostic, test, blame line). Lists inside a decoration
work as in panels (`list-rows(id, ...)`). Open questions for 0.2: a `decoration-orphaned(id)` event
when the anchored line is deleted; anchoring to a buffer handle (layer 3) instead of a path;
inline (inlay) decorations next to text instead of blocks.
