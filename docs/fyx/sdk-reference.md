# `ui_ext_sdk` reference

`ui_ext_sdk` is the Rust guest SDK for Fyx UI extensions, built on `wit-bindgen` 0.41 over the
`fyx:ui@0.1.0` world. It is licensed Apache-2.0, so extensions (proprietary ones included) stay
outside the GPL. It is not on crates.io yet, so depend on it by path (see
[Project layout](extension-authoring.md#project-layout)).

Source: `crates/ui_ext_sdk/src/ui_ext_sdk.rs` (cited below as `sdk:<line>`) and
`crates/ui_ext_sdk/src/host_model.rs`.

The page lists every public item. For each one it gives what it does, the host rule it enforces,
and the error it returns. "Host" means Fyx's side: `crates/ui_ext` (the tree model),
`crates/ui_ext_wasm/src/ui_ext_wasm.rs` (the WASM runtime), and `crates/ui_ext_host` (the
renderer).

Contents:

- [Imports and re-exports](#imports-and-re-exports)
- [Constants](#constants)
- [`Extension` and `register_extension!`](#extension-and-register_extension)
- [Props builders](#props-builders)
- [`IdAllocator`](#idallocator)
- [`TreeBuilder`](#treebuilder)
- [`SurfaceTree`](#surfacetree)
- [`PatchSet`](#patchset)
- [List rows: `list_rows`, `list_row`, `row_range`](#list-rows-list_rows-list_row-row_range)
- [`TreeError`](#treeerror)
- [`log`, `push_update`, `take_pushed_updates`](#log-push_update-take_pushed_updates)
- [Which helper checks what](#which-helper-checks-what)

## Imports and re-exports

```rust
use ui_ext_sdk::*;
```

This brings in:

| Item | What |
|---|---|
| every type of WIT interface `fyx:ui/types` | `NodeId` (`u32`), `ColorToken`, `FlexDirection`, `Align`, `FlexProps`, `TextProps`, `ButtonStyle`, `ButtonProps`, `InputProps`, `ListProps`, `TableProps`, `IconProps`, `DiffViewProps`, `Props`, `Node`, `InsertOp`, `Patch`, `Update`, `ListRow`, `UiEvent`, `DockPosition`, `PanelDescriptor`, `StatusBarSide`, `StatusItemDescriptor`, `SurfaceKind`, `Surface` |
| `LogLevel` | `Trace`, `Debug`, `Info`, `Warn`, `Error` (from `fyx:ui/host`) |
| `TreeLimits` | the host's caps, re-exported from `ui_ext` (see [`SurfaceTree`](#surfacetree)) |
| `bindings` | the raw `wit-bindgen` output, `bindings::Guest` and `bindings::export!` included. `register_extension!` uses it, and you normally do not need it. |

<!-- sdk:59-72 -->

The generated types are plain Rust records and enums. They derive `Clone` and `Debug` (from
`wit-bindgen`), plus `PartialEq` and `Eq` (SDK `additional_derives`). WIT names become Rust
names: `%list` becomes `Props::List` and `diff-view` becomes `Props::DiffView`.

A variant whose payload is a WIT tuple takes **one** tuple field, which is why the patterns have
double parentheses:

```rust
UiEvent::Click(id)
UiEvent::InputChanged((id, text))
UiEvent::InputSubmitted((id, text))
UiEvent::RowClick((list_id, row_index))
Patch::SetProps((id, props))
Patch::Insert(InsertOp { node, before })
Patch::Remove(id)
```

<!-- crates/ui_ext/wit/fyx-ui.wit:107-132; sdk:1302 (Patch::SetProps((2, ...))) -->

`Props::Markdown(String)` is the only props variant without a record. Build it with
`Props::Markdown(source)` or [`TreeBuilder::markdown`](#treebuilder).

## Constants

| Constant | Value | Meaning |
|---|---|---|
| `API_VERSION: (u16, u16, u16)` | `(0, 1, 0)` | The `fyx:ui` version the SDK is generated from. A test checks it against the `package fyx:ui@…` line of the WIT. |
| `API_VERSION_SECTION: &str` | `"fyx:ui-api-version"` | Name of the custom section `register_extension!` embeds. |
| `API_VERSION_SECTION_BYTES: [u8; 6]` | `[0, 0, 0, 1, 0, 0]` | `API_VERSION` as three big-endian `u16`s, the same layout as Zed's `zed:api-version`. |
| `ROW_NODE_ID_BASE: NodeId` | `0x8000_0000` | First id inside a list row, so row ids can never be mistaken for main-tree ids (which start at 1). |

<!-- sdk:74-98, 1520-1526 -->

## `Extension` and `register_extension!`

```rust
pub trait Extension: 'static {
    fn new() -> Self where Self: Sized;
    fn surfaces(&self) -> Vec<Surface>;
    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String>;
    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }
    fn list_rows(&mut self, surface_id: &str, list_node: NodeId, start: u32, count: u32)
        -> Vec<ListRow> {
        Vec::new()
    }
    /// fyx:ui 0.2: the tick `request_tick` asked for.
    fn tick(&mut self, surface_id: &str) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }
}
```

<!-- sdk:133-161 -->

One instance lives as long as the WASM instance. The SDK creates it with `new()` on the first
export call, which is `surfaces` during load, and keeps it in a thread local. The host calls one
export at a time from a single thread and never calls back into the extension during a call, so
`&mut self` is never contended.

<!-- sdk:225-238 -->

| Method | Called by the host | Host rules on the result |
|---|---|---|
| `surfaces` | Once, right after instantiation. The 2 s init budget covers instantiation and this call (and `new`). | Each panel surface needs `ui:panel` and each status item `ui:status-bar` in the manifest. Surface ids must be unique per extension; a duplicate `extension/surface` key is skipped. An empty `title` falls back to the id. |
| `mount` | Once per surface when the extension loads, and again to resync after the host rejected an update. All windows share one mount. | A valid flat tree: parents first, unique ids, children only under `flex`, at most 16384 nodes. On `Err` or an invalid tree the surface shows the error instead of a tree, and it is not retried. |
| `handle_event` | Once per input event, serialized (see the [mental model](extension-authoring.md#the-mental-model)). | `Err` is shown in the panel footer and is not fatal. An update that does not apply leaves the tree as it was, shows the error and triggers one `mount`. |
| `list_rows` | When rows of a `list` or `table` node scroll into view (visible range plus 8 rows of overscan), and again after the list node is touched. | Each row index must be inside `start..start + count`, and each row must be a valid tree with exactly one root and at most 2048 nodes. One bad row makes the host reject the **whole** answer, and it does not ask again until the list changes. |

Every call has a 200 ms budget (2 s for instantiation and `surfaces`). A call that exceeds it,
traps or panics disables the extension for the session.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:62-71, 188-203, 476-495, 579-592 (mount validation), 620-660 (list-rows validation), 663-689 (Err → ReplyError, not fatal), 707-721 (disable on trap/budget); crates/ui_ext_host/src/registry.rs:76-83; crates/ui_ext_host/src/panel.rs:51-55; crates/ui_ext_host/src/surface.rs:130-161, 240-263, 287-343, 357-402, 20 (ROW_OVERSCAN) -->

```rust
register_extension!(MyExtension);        // exports fyx:ui 0.2
register_extension!(MyExtension, v0_1);  // exports fyx:ui 0.1, for hosts that only run 0.1
```

The SDK's types and functions are those of `fyx:ui@0.2.0` (`API_VERSION`); one extension source
exports either world. Call the macro once, at the root of your `cdylib` crate. It:

- implements the chosen world's `Guest` trait by forwarding each export to your type. With
  `v0_1`, values are converted at the boundary. Icons become upstream `IconName`s, and
  tooltips, list selection and row indents are dropped. What 0.1 cannot express is an error that
  names 0.2: modal surfaces are left out of `surfaces` (logged), and secret inputs, the `blank`
  icon node and `invalidate-rows` patches make the call return `Err`. `tick` is never called.
- defines `extern "Rust"` shims through which the SDK's host functions reach that world's
  imports, so a 0.1 component imports only `fyx:ui/*@0.1.0`. A 0.2 component always imports
  `fyx:ui/decorations`; the host checks `ui:editor-decorations` per call, not at import.
- on `wasm32` only, adds a `#[used]` static in the `fyx:ui-api-version` link section holding
  `API_VERSION_SECTION_BYTES` (0.2) or `API_VERSION_V0_1_SECTION_BYTES` (0.1). The static lives in
  your crate, so the linker cannot drop it as an unused member of the SDK's rlib;
- calls the export macro wit-bindgen generates for the world. The 0.1 bindings live in the crate `ui_ext_sdk_v0_1`,
  because wit-bindgen names the export macro after the world, and both worlds are called
  `ui-extension`.

<!-- crates/ui_ext_sdk/src/ui_ext_sdk.rs (register_extension!, __fyx_host_shims!), crates/ui_ext_sdk/src/v0_1.rs, crates/ui_ext_sdk/src/host.rs -->

<!-- sdk:163-219 -->

Do not emit the version section yourself as well. The linker concatenates both copies, and the
host refuses a section of 12 bytes with "N copies … emit it only via
ui_ext_sdk::register_extension!".

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:94-103 -->

## Props builders

The SDK adds builder methods to the generated records. Every one of them is infallible and
checks nothing: the rules apply when a node goes into a tree.

| Type | Constructor and defaults | Setters |
|---|---|---|
| `Surface` | `Surface::panel(id, title, DockPosition, icon, default_size: u32)`<br>`Surface::status_item(id, title, StatusBarSide, priority: i32)` | — |
| `FlexProps` | `FlexProps::column()`, `FlexProps::row()`: gap 0, padding 0, no grow, `Align::Start`, no background | `.gap(u16)`, `.padding(u16)`, `.grow()`, `.align(Align)`, `.background(ColorToken)` |
| `TextProps` | `TextProps::new(content)`: no colour, not bold, not monospace; also `From<&str>` and `From<String>` | `.color(ColorToken)`, `.bold()`, `.monospace()` |
| `ButtonProps` | `ButtonProps::new(label)`: no icon, `ButtonStyle::Default`, enabled; also `From<&str>` and `From<String>` | `.icon(name)`, `.style(ButtonStyle)`, `.disabled(bool)` |
| `InputProps` | `InputProps::new(value)`: empty placeholder | `.placeholder(text)` |
| `ListProps` | `ListProps::new(row_count: u32, row_height: u16)` | — |
| `TableProps` | `TableProps::new(columns: impl IntoIterator<Item = impl Into<String>>, row_count: u32)` | — |
| `IconProps` | `IconProps::new(name)`: no colour; also `From<&str>` | `.color(ColorToken)` |
| `DiffViewProps` | `DiffViewProps::new(old_text, new_text)`: no language | `.language(name)` |

`From<XProps> for Props` exists for all eight records, so any function that takes
`impl Into<Props>` accepts them directly.

<!-- sdk:919-1163 -->

How the host renders each kind, and the units it uses:

| Kind | Rendering and notes |
|---|---|
| flex | `h_flex` or `v_flex`. `gap` and `padding` are pixels, `grow` makes it `flex_1`, `align` sets `items_*`, `background` is a theme colour. There is no justify, width, height or border. |
| text | `Label`. `bold` gives bold weight, `monospace` the buffer font, `color` a theme colour. |
| button | `ui::Button`. `Default` → outlined, `Filled` → filled, `Subtle` → subtle, `Danger` → error tint. An empty label with an icon renders an icon-only button. |
| input | A single-line editor in a bordered box. |
| list | A `uniform_list` with fixed `row_height` (0 means 24 px). It is at least `min(row_count, 3)` rows tall, and rows come from `list-rows`. |
| table | A header from `columns`, then rows from `list-rows` at a fixed 24 px. If a row's root is a flex node, its children become equal-width cells. |
| icon | `ui::Icon` by snake_case `IconName`. An unknown name shows the `blocks` icon. |
| markdown | Zed's markdown element. Inside list rows it is a plain label. |
| diff-view | **Placeholder**: common leading and trailing lines as context, the middle as removed then added lines. It is not Zed's diff editor, and `language` is unused. |

<!-- crates/ui_ext_host/src/view.rs:401-438 (flex, text), 440-470 (button), 472-506 (input), 509-584 (table, list, row height, min height), 585-627 (rows, table cells), 629-636 (icon), 638-654 (markdown), 656-690 (diff placeholder), 749-760 (icon fallback, button styles) -->

## `IdAllocator`

```rust
pub struct IdAllocator { /* next: NodeId */ }
impl IdAllocator {
    pub const fn new() -> Self;                    // first id 1
    pub const fn starting_at(first: NodeId) -> Self;
    pub fn allocate(&mut self) -> NodeId;          // returns next, then increments
    pub fn peek(&self) -> NodeId;                  // the id allocate() returns next
}
impl Default for IdAllocator                       // same as new()
```

Ids only increase, which satisfies the WIT rule that an id is never reused for a different node
while the surface lives. `allocate` does a plain `+= 1`, which would overflow (a panic in debug
builds) only after about 4 billion ids. In practice `SurfaceTree` and `TreeBuilder` own the allocator, and you rarely create one yourself.

<!-- sdk:241-273; crates/ui_ext/wit/fyx-ui.wit:5-7 -->

## `TreeBuilder`

Builds a flat, parents-first `Vec<Node>` from nested code. Every node gets the next id from the
allocator and its enclosing flex node as parent.

```rust
impl<'a> TreeBuilder<'a> {
    pub fn new(ids: &'a mut IdAllocator) -> Self;                   // top-level nodes have parent None
    pub fn under(ids: &'a mut IdAllocator, parent: NodeId) -> Self; // top-level nodes are children of `parent`
    pub fn node(&mut self, props: impl Into<Props>) -> NodeId;      // a node without children
    pub fn flex(&mut self, props: FlexProps, build_children: impl FnOnce(&mut Self)) -> NodeId;
    pub fn column(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId; // flex(FlexProps::column(), ..)
    pub fn row(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId;    // flex(FlexProps::row(), ..)
    pub fn text(&mut self, props: impl Into<TextProps>) -> NodeId;
    pub fn button(&mut self, props: impl Into<ButtonProps>) -> NodeId;
    pub fn input(&mut self, props: InputProps) -> NodeId;
    pub fn list(&mut self, props: ListProps) -> NodeId;
    pub fn table(&mut self, props: TableProps) -> NodeId;
    pub fn icon(&mut self, props: impl Into<IconProps>) -> NodeId;
    pub fn markdown(&mut self, source: impl Into<String>) -> NodeId;
    pub fn diff_view(&mut self, props: DiffViewProps) -> NodeId;
    pub fn nodes(&self) -> &[Node];
    pub fn finish(self) -> Vec<Node>;
}
```

<!-- sdk:275-371 -->

- **Rule enforced by construction:** only `flex`, `column` and `row` take a children closure, so a
  builder cannot put a child under a non-flex node. `node(FlexProps…)` adds a flex node without
  children.
- **Order:** a parent is pushed before its closure runs, so the output is always parents-first,
  with siblings in call order.
- **No other checks:** the builder knows nothing about caps. `SurfaceTree::mount`/`insert`/
  `rebuild` and `list_row` check its output.
- Every method returns the new node's id. Keep the ids you need for events and patches.

Inside `SurfaceTree::mount`, `insert` and `rebuild`, and inside `list_row`/`list_rows`, you get a
`&mut TreeBuilder` from the SDK. Create one yourself only to drive a raw [`PatchSet`](#patchset)
or a hand-written `mount`.

## `SurfaceTree`

The extension's copy of one surface's tree, mirroring what the host has applied. Use one per
surface.

```rust
#[derive(Debug, Default)]
pub struct SurfaceTree { /* ids, limits, mounted, nodes, roots, pending */ }
```

<!-- sdk:562-582 -->

What it guarantees:

- **Ids** come from its own `IdAllocator`, which survives `mount`/`replace`. A remount therefore
  never reuses an id of an earlier tree (in the counter demo the first mount is 1–7, the second
  8–14).
- **Edits are queued as patches.** `take_update()` returns them as one `Update`.
- **`set_props` with props equal to the current ones queues nothing**, so you can re-set every
  node after a state change and only real changes are sent.
- **Every edit is checked against the host's rules first.** An edit that would break one returns
  `Err(TreeError)` and leaves the mirror and the queue exactly as they were (ids it allocated stay
  unused). The queue is therefore always a batch the host accepts, which matters because the host
  rejects a whole batch if one patch is bad and does not tell the extension.

<!-- sdk:564-573, 601-604; extensions/fyx_demo_counter/src/fyx_demo_counter.rs:19-23 -->

The rules come from `ui_ext` itself, not from a copy: `ui_ext::Props::is_container` and
`kind_name`, through the conversions in `host_model.rs`.

<!-- crates/ui_ext_sdk/src/host_model.rs:1-3; sdk:792-805 -->

### Construction and limits

| Method | What |
|---|---|
| `SurfaceTree::new()` / `default()` | Checked against `TreeLimits::default()`: `max_nodes_per_update` 2048 and `max_nodes` 16384, the host's defaults. |
| `SurfaceTree::with_limits(TreeLimits)` | Smaller caps, for tests. |
| `limits(&self) -> TreeLimits` | The caps in use. |

<!-- sdk:584-599; crates/ui_ext/src/tree.rs:9-25 -->

`TreeLimits { max_nodes_per_update: usize, max_nodes: usize }`: on the host,
`max_nodes_per_update` caps the patches in one `patches` update and the nodes in one list row, and
`max_nodes` caps the whole tree (and with it a `mount` or `replace`).

### Building a tree

| Method | Returns | Checks, and errors |
|---|---|---|
| `mount(build)` | `Result<Vec<Node>, TreeError>`: the tree to return from `Extension::mount` | Node count ≤ `max_nodes_per_update` (**2048**, stricter than the host's 16384 for a mount) → `UpdateTooLarge`. Node count ≤ `max_nodes` → `TreeTooLarge`. On success, the previous mirror **and any queued patches are dropped**. On error, the previous tree is kept. |
| `replace(build)` | `Result<Update, TreeError>`: `Update::Replace(nodes)` | Same as `mount`. For a `handle_event` answer where patching would be more work than rebuilding. |

<!-- sdk:601-624 -->

### Reading the mirror

| Method | Returns |
|---|---|
| `is_mounted()` | whether `mount`/`replace` has succeeded once |
| `contains(id)` | whether `id` is in the mirror |
| `props(id)` | `Option<&Props>` |
| `parent(id)` | `Option<NodeId>`: `None` for roots and unknown ids |
| `children(id)` | `&[NodeId]`: empty for leaves and unknown ids |
| `roots()` | `&[NodeId]` |
| `len()`, `is_empty()` | node count |
| `to_nodes()` | the whole mirror as a flat, parents-first `Vec<Node>`, as the host holds it |
| `pending()` | `&PatchSet`: what `take_update` would return |

<!-- sdk:626-675, 774-776 -->

### Editing

Every editing method fails with `NotMounted` before the first successful `mount`/`replace`, and
with `UnknownNode(id)` for an id that is not in the mirror. "Queue" below means the patches
waiting for `take_update`, counted against `max_nodes_per_update` (2048). An inserted subtree
counts one patch per node.

| Method | Effect | Further checks, and errors |
|---|---|---|
| `set_props(id, props) -> Result<bool, _>` | Replaces the props and queues `set-props`. Returns `Ok(false)` and queues nothing if they are equal to the current props. | `NotAContainer { node, kind }` if `id` has children and `props` is not flex. `UpdateTooLarge` if the queue is full. |
| `set_props_from_host(id, props) -> Result<(), _>` | Records props **without** queueing a patch, for a value the host already shows (an input's text after `input-changed`). | `NotAContainer` as above. No queue check, since nothing is queued. |
| `insert(parent, before, build) -> Result<Vec<NodeId>, _>` | Builds a subtree under `parent`, before its child `before` (or appended for `None`), and returns the ids of the subtree's top-level nodes. | `NotAContainer` if `parent` is not flex. `NotAChild { parent, child }` if `before` is not a child of `parent`. `UpdateTooLarge` if queue + new nodes > 2048. `TreeTooLarge` if the tree would exceed 16384. |
| `remove(id) -> Result<(), _>` | Removes the node and its whole subtree (one `remove` patch). | `UpdateTooLarge`. |
| `rebuild(id, build) -> Result<Vec<NodeId>, _>` | Replaces `id` and its subtree with the nodes `build` adds, at the same position, **with new ids** (`remove` + inserts). The host treats them as new nodes, so a rebuilt list has all its rows requested again. Checked as a whole: on error nothing is removed. | `IsRoot(id)` if `id` has no parent: wrap anything you want to rebuild in a flex node. `NotAChild` if the mirror is inconsistent. `UpdateTooLarge` if queue + 1 + new nodes > 2048. `TreeTooLarge`. |
| `clear_children(parent) -> Result<(), _>` | Removes every child of `parent` (one `remove` per child). | `UpdateTooLarge`. |
| `take_update() -> Update` | Returns the queue, `Update::Unchanged` if empty, and clears it. | — |

<!-- sdk:677-772, 778-843 -->

Use `take_update()` for both the return value of `handle_event` and the argument of
[`push_update`](#log-push_update-take_pushed_updates), so the mirror and the host's tree stay
identical. If you return `Err` from `handle_event` after editing the `SurfaceTree`, the queued
patches stay queued and go out with the next `take_update()`.

When the host rejects an update, it mounts the surface again. `SurfaceTree::mount` resets the
mirror, so an extension that rebuilds its tree in `mount` (as the demos do) is back in sync after
one round trip. Store the new node ids on every mount.

<!-- crates/ui_ext_host/src/surface.rs:287-343 -->

## `PatchSet`

An ordered batch of raw patches, with **no checks at all**. `SurfaceTree` uses one internally. Use
it directly only when you manage the tree yourself.

```rust
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PatchSet { /* patches: Vec<Patch> */ }
impl PatchSet {
    pub fn new() -> Self;
    pub fn set_props(&mut self, id: NodeId, props: impl Into<Props>) -> &mut Self;
    pub fn insert(&mut self, node: Node, before: Option<NodeId>) -> &mut Self;
    pub fn insert_subtree(&mut self, nodes: Vec<Node>, before: Option<NodeId>) -> &mut Self;
    pub fn remove(&mut self, id: NodeId) -> &mut Self;
    pub fn is_empty(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn patches(&self) -> &[Patch];
    pub fn into_update(self) -> Update;   // Unchanged when empty, Patches otherwise
}
impl From<PatchSet> for Update
```

<!-- sdk:424-493 -->

- `insert(node, before)` inserts under `node.parent`, before the sibling `before` or at the end.
- `insert_subtree(nodes, before)` takes a parents-first subtree, such as one built with
  `TreeBuilder::under`. Its top-level nodes (those whose parent is not inside `nodes`) get
  `before`, in order. Deeper nodes are appended to their new parents.

The host applies the patches in order, atomically, and fails the whole batch on the first error:

| Host error (`ui_ext::TreeError`) | Cause |
|---|---|
| `UpdateTooLarge { count, max }` | more than 2048 patches |
| `TreeTooLarge { max }` | the tree would exceed 16384 nodes |
| `DuplicateId(id)` | inserting an id that is already in the tree |
| `UnknownParent { node, parent }` | inserting under a parent that is not in the tree (yet) |
| `NotAContainer { parent, kind }` | a child under a non-flex node, or `set-props` that turns a flex with children into a leaf |
| `UnknownNode(id)` | `set-props` or `remove` on an id that is not in the tree |
| `UnknownSibling { node, before }` | `before` is not a child of the same parent |
| `Patch { index, error }` | wraps any of the above with the index of the failing patch |

<!-- crates/ui_ext/src/tree.rs:27-49, 188-322 -->

The host shows the error in the panel footer and calls `mount` again. A rejected batch is not
fatal to the extension.

## List rows: `list_rows`, `list_row`, `row_range`

```rust
pub fn list_rows(start: u32, count: u32, row_count: u32,
                 build_row: impl FnMut(u32, &mut TreeBuilder)) -> Vec<ListRow>;
pub fn list_row(index: u32, build: impl FnOnce(&mut TreeBuilder)) -> Result<ListRow, TreeError>;
pub fn row_range(start: u32, count: u32, row_count: u32) -> Range<u32>;
```

<!-- sdk:373-422 -->

- `row_range` returns the indices of a `list-rows(start, count)` request that exist in a list of
  `row_count` rows: `start..min(start + count, row_count)`, saturating and never inverted. The
  host may ask past the end while a list shrinks.
- `list_row` builds one row with a fresh `IdAllocator` starting at `ROW_NODE_ID_BASE`, then
  validates it with the host's own `ui_ext::Tree::from_row` under default limits. It fails with
  `TreeError::InvalidRow(ui_ext::TreeError)`: `RowRootCount(n)` if the closure added zero or
  several top-level nodes, `UpdateTooLarge` if the row has more than 2048 nodes.
- `list_rows` calls `build_row(index, tree)` for each index in `row_range`. A row that fails
  `list_row` is logged at `Error` level (`list row N left out: …`) and left out, so the other rows
  still show. The host draws a left-out row as an empty row and does not ask for it again until
  the list changes.

<!-- crates/ui_ext/src/tree.rs:95-104; crates/ui_ext_host/src/surface.rs:41-43, 366-376 (left-out rows not re-requested); crates/ui_ext_host/src/view.rs:606-609 (empty row) -->

Use `list_rows` rather than building `ListRow`s by hand. The host rejects the whole answer if any
row is invalid or has an index outside `start..start + count`.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:640-658 -->

`Extension::list_rows` is also called for `table` nodes, with the table's id. See
[Rows](limits-and-gotchas.md#rows-row-local-ids-row-click-and-refresh) for row-local ids,
`row-click` and when rows are requested again.

## `TreeError`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    NotMounted,
    UnknownNode(NodeId),
    NotAChild { parent: NodeId, child: NodeId },
    IsRoot(NodeId),
    NotAContainer { node: NodeId, kind: &'static str },
    UpdateTooLarge { count: usize, max: usize },
    TreeTooLarge { count: usize, max: usize },
    InvalidRow(ui_ext::TreeError),
}
impl Display, std::error::Error, From<TreeError> for String
```

<!-- sdk:495-553 -->

| Variant | Display | Returned by |
|---|---|---|
| `NotMounted` | `surface is not mounted` | any `SurfaceTree` edit before a successful mount |
| `UnknownNode(id)` | `node {id} is not in the surface's tree` | `SurfaceTree` edits on an unknown id |
| `NotAChild { parent, child }` | `node {child} is not a child of node {parent}` | `insert` with a bad `before`; `rebuild` |
| `IsRoot(id)` | `node {id} is a root of the surface's tree` | `rebuild` of a root |
| `NotAContainer { node, kind }` | `node {node} would be a {kind} with children; only flex nodes have children` | `insert` under a non-flex node; `set_props`/`set_props_from_host` that would make a node with children a non-flex |
| `UpdateTooLarge { count, max }` | `update would carry {count} nodes or patches, the cap is {max}` | `mount`, `replace`, and any edit that overfills the queue |
| `TreeTooLarge { count, max }` | `tree would have {count} nodes, the cap is {max}` | `mount`, `replace`, `insert`, `rebuild` |
| `InvalidRow(error)` | `invalid list row: {error}` | `list_row` |

`kind` is the host's kind name: `flex`, `text`, `button`, `input`, `list`, `table`, `icon`,
`markdown` or `diff-view`.

<!-- crates/ui_ext/src/types.rs:141-153 -->

`From<TreeError> for String` lets you use `?` on SDK calls inside `mount` and `handle_event`, which
return `Result<_, String>`. The string is what the user sees.

## `log`, `push_update`, `take_pushed_updates`

```rust
pub fn log(level: LogLevel, message: &str);
pub fn push_update(surface_id: &str, update: &Update) -> Result<(), HostError>;
#[cfg(not(target_arch = "wasm32"))]
pub fn take_pushed_updates() -> Vec<(String, Update)>;
```

`fyx:ui` 0.2 adds these host functions; each is checked when called and its effect happens after
the current call returns (design.md, section A):

```rust
pub fn request_tick(surface_id: &str, delay_ms: u32) -> Result<(), HostError>;
pub fn cancel_tick(surface_id: &str);
pub fn add_surface(surface: &Surface) -> Result<(), HostError>;
pub fn remove_surface(surface_id: &str) -> Result<(), HostError>;
pub fn show_surface(surface_id: &str) -> Result<(), HostError>;
pub fn hide_surface(surface_id: &str) -> Result<(), HostError>;
pub fn place_decoration(id: &str, location: &DecorationLocation, nodes: &[Node])
    -> Result<(), HostError>;
pub fn remove_decoration(id: &str) -> Result<(), HostError>;
#[cfg(not(target_arch = "wasm32"))]
pub fn take_host_effects() -> Vec<HostEffect>;
```

In an extension exported with `v0_1` they return `HostError` with code `unavailable`
("needs fyx:ui 0.2"), and `cancel_tick` does nothing. `push_update` converts the update to 0.1 and
returns `invalid-argument` for one 0.1 cannot express. `SurfaceTree::push(surface_id)` pushes the
queued update; after a refused push it sends the whole tree as a `replace` next time.

<!-- sdk:100-131 -->

**`log`** calls the host's `log` import. The host writes the message to Fyx's log at that level,
under target `fyx_ui`, prefixed with the extension id. Off WASM it prints `[Level] message` to
stderr.

<!-- crates/ui_ext_wasm/src/ui_ext_wasm.rs:289-299 -->

**`push_update`** sends an update the extension initiated for `surface_id`. The surface can be
another surface of the same extension. Rules:

- It only does anything **during a host call**, since the component only runs then. Pushes made
  during `handle-event` or `list-rows` are applied in order **before** that call's reply, so a
  reply may depend on nodes a push inserted.
- Pushes made during `mount` for the surface being mounted are dropped, as the returned tree
  supersedes them. Pushes for other surfaces still apply.
- Pushes made during instantiation or `surfaces` go to a channel that the registry drains. At that
  point the surfaces are usually still mounting, and an update that arrives before a surface's
  mount finishes is dropped.
- If the call then returns `Err` or its reply fails validation, the runtime hands the pushes back
  inside a `ReplyError`. The current renderer does **not** apply them: it only logs and shows the
  error. Your `SurfaceTree` already counts those pushes as applied, so do not push and then fail
  in the same call (see [Limits](limits-and-gotchas.md#push-update-only-during-a-call)). If the
  call traps or runs out of time, its pushes are discarded and the extension is disabled.
- At most **64 pushes per call**. The 65th traps the call and disables the extension.
- A push for an unknown surface id is logged and dropped. A push that does not fit the tree is
  treated like a bad reply: the error is shown and the surface remounts.
- Build the update with `SurfaceTree::take_update()`, so your mirror matches what the host draws.

<!-- tasks/progress/W2.md:21-32 (push ordering rule); crates/ui_ext_wasm/src/ui_ext_wasm.rs:206-241, 277-287 (cap 64), 498-502, 693-734; crates/ui_ext_host/src/surface.rs:137-145, 240-285 (apply_reply: the Err arm never downcasts ReplyError), 291-298, 404-417; crates/ui_ext_host/src/registry.rs:109-156 -->

Off WASM, every host function records a `HostEffect` in a thread local and succeeds.
`take_host_effects()` returns and clears what was recorded on the current thread;
`take_pushed_updates()` takes only the pushes. The SDK does this because `wit-bindgen`'s off-WASM
import stubs abort the test process.

<!-- tasks/progress/W4.md:124-126 -->

## Which helper checks what

| Helper | Children only under flex | Patch / row cap (2048) | Tree cap (16384) | Single-root rows | Unknown ids / siblings |
|---|---|---|---|---|---|
| `TreeBuilder` | by construction | — | — | — | — |
| `SurfaceTree` | yes | yes (also caps `mount` at 2048) | yes | — | yes |
| `PatchSet` | — | — | — | — | — |
| `list_row` / `list_rows` | by construction | yes (per row) | — | yes | — |
| host (`ui_ext::Tree`) | yes | patches and rows | yes (mount, replace, patches) | yes | yes |

What nothing in the SDK checks: duplicate ids in a hand-built `Vec<Node>` and row indices outside
the requested range (both rejected by the host), and the time budget.
