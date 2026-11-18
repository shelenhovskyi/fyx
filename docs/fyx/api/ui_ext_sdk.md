# `ui_ext_sdk` public items

Generated from `crates/ui_ext_sdk/src/ui_ext_sdk.rs` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

Every public item, in path order. For signatures, run `script/fyx-api-docs` and open the rustdoc it builds.

| Item | Kind | Summary |
|---|---|---|
| `API_VERSION` | const |  |
| `API_VERSION` | const | The `fyx:ui` WIT version of the SDK's API and of `register_extension!(T)`, as (major, minor, patch): 0.2, or the staged 0.3 with the `fyx-ui-0-3` feature. |
| `API_VERSION_SECTION` | const | Name of the custom section `register_extension!` adds to every component, from which the host reads the version before instantiating it. |
| `API_VERSION_SECTION_BYTES` | const | `API_VERSION` as stored in the section: three big-endian `u16`s, as in Zed's `zed:api-version`. |
| `API_VERSION_V0_1` | const | The version `register_extension!(T, v0_1)` exports, for hosts that only run 0.1. |
| `API_VERSION_V0_1_SECTION_BYTES` | const | `API_VERSION_V0_1` as stored in the section. |
| `API_VERSION_V0_2` | const | The version `register_extension!(T, v0_2)` exports. |
| `API_VERSION_V0_2_SECTION_BYTES` | const | `API_VERSION_V0_2` as stored in the section. |
| `ButtonProps::disabled` | method |  |
| `ButtonProps::icon` | method |  |
| `ButtonProps::new` | method |  |
| `ButtonProps::style` | method |  |
| `ButtonProps::tooltip` | method | Shown on hover; the host truncates text over 1024 bytes. |
| `DecorationLocation::at` | method | A block above or below `line` (zero-based) of the worktree-relative `path`, with the height left to the host. The same code builds 0.2's `path` and 0.3's `target`. |
| `DecorationLocation::height_lines` | method | The block's height in editor lines. |
| `DiffViewProps::language` | method |  |
| `DiffViewProps::new` | method |  |
| `Extension` | trait | A UI extension. One instance lives for the lifetime of the WASM instance; the host calls it from a single thread, one call at a time. |
| `Extension::handle_event` | method |  |
| `Extension::list_rows` | method | Rows `start..start + count` of the virtualized list `list_node`. `list_rows` clamps the range to the list's length and builds each row. |
| `Extension::mount` | method | The whole tree of `surface_id`, parents first. `SurfaceTree::mount` builds one. |
| `Extension::new` | method |  |
| `Extension::surfaces` | method | The static surfaces, called once after instantiation. `add_surface` adds more later. |
| `Extension::tick` | method | The tick `request_tick` asked for. The update applies to `surface_id`, like an event's reply. `fyx:ui` 0.1 has no ticks, so a `v0_1` extension never gets this call. |
| `FlexProps::align` | method |  |
| `FlexProps::background` | method |  |
| `FlexProps::column` | method | A column with no gap, no padding and children aligned to the start. |
| `FlexProps::gap` | method |  |
| `FlexProps::grow` | method |  |
| `FlexProps::padding` | method |  |
| `FlexProps::row` | method | A row with no gap, no padding and children aligned to the start. |
| `HostError` | re-export | `bindings::fyx::ui::host::HostError` |
| `HostErrorCode` | re-export | `bindings::fyx::ui::host::HostErrorCode` |
| `HostErrorCode::name` | method | The WIT case name, such as `not-permitted`. |
| `Icon::all` | method | Every frozen icon, in WIT declaration order (101 in `fyx:ui` 0.2 and 0.3). |
| `Icon::name` | method | The WIT case name, such as `arrow-up`. |
| `IconProps::color` | method |  |
| `IconProps::new` | method |  |
| `IdAllocator` | struct | Hands out the node ids of one surface. Ids only ever increase, so an id is never reused for a different node while the surface lives, as the `fyx:ui` contract requires. |
| `IdAllocator::allocate` | method |  |
| `IdAllocator::new` | method |  |
| `IdAllocator::peek` | method | The id the next call to `IdAllocator::allocate` returns. |
| `IdAllocator::starting_at` | method |  |
| `ImageFormat` | re-export | `bindings::fyx::ui::images::ImageFormat` |
| `ImageProps::asset` | method | A PNG or SVG file of the package's `assets/` folder, such as `assets/logo.svg`: scaled to fit its box (`contain`), with the image's own size and no tint. |
| `ImageProps::fit` | method |  |
| `ImageProps::generated` | method | An image registered with `add_image`(crate::add_image). |
| `ImageProps::height` | method | Only the height; the width follows the aspect ratio. |
| `ImageProps::size` | method | The box in logical pixels; the image keeps its aspect ratio inside it (`ImageProps::fit`). |
| `ImageProps::tint` | method | SVG only: draw it as a one-colour mask in this theme colour, like the host's own icons. |
| `ImageProps::width` | method | Only the width; the height follows the aspect ratio. |
| `InputProps::new` | method |  |
| `InputProps::placeholder` | method |  |
| `InputProps::secret` | method | Masks the text on screen (passwords, tokens). The value still reaches the extension. |
| `ListProps::new` | method |  |
| `ListProps::selected` | method | The highlighted row, scrolled into view. |
| `ListRow::with_indent` | method | Nesting depth, drawn by the host as indentation (tree views); at most 32 levels. |
| `LogLevel` | re-export | `bindings::fyx::ui::host::LogLevel` |
| `PatchSet` | struct | An ordered batch of patches, turned into an `update` with `PatchSet::into_update`. |
| `PatchSet::insert` | method | Inserts one node under `node.parent`, before the sibling `before` or at the end. |
| `PatchSet::insert_subtree` | method | Inserts a subtree in parents-first order, such as one built with `TreeBuilder::under`. Its top-level nodes (those whose parent is outside `nodes`) go before `before`, keeping their order; deeper nodes are appended to their new parents. |
| `PatchSet::into_update` | method | `unchanged` when there is nothing to send, `patches` otherwise. |
| `PatchSet::invalidate_rows` | method | The rows of a list or table changed although its props did not: the host fetches them again, `rows` only, or all of them for `None`. |
| `PatchSet::is_empty` | method |  |
| `PatchSet::len` | method |  |
| `PatchSet::new` | method |  |
| `PatchSet::patches` | method |  |
| `PatchSet::remove` | method | Removes the node and its whole subtree. |
| `PatchSet::set_props` | method |  |
| `ROW_NODE_ID_BASE` | const | First id handed out inside a `ListRow` subtree. |
| `Surface::modal` | method | A modal, closed until `show_surface` opens it. Needs `ui:modal`. |
| `Surface::panel` | method |  |
| `Surface::status_item` | method |  |
| `SurfaceTree` | struct | The extension-side copy of one surface's tree, mirroring what the host has applied. |
| `SurfaceTree::apply_event` | method | Applies to the mirror what the host does to its own tree when it sends `event`. On `input-changed` and `input-submitted` the host writes the typed value into the input node (keeping its placeholder) without a patch, because the text is the user's; other events change nothing. Call it with every event of this surface before handling it. Then the mirror holds what the host holds: echoing the typed value back queues nothing (it would fight the user's typing), and clearing the field afterwards does queue the `set-props`, which a stale mirror would have dropped as unchanged. |
| `SurfaceTree::children` | method |  |
| `SurfaceTree::clear_children` | method | Removes every child of `parent`. |
| `SurfaceTree::contains` | method |  |
| `SurfaceTree::insert` | method | Builds a subtree under the flex node `parent`, before its child `before` or after its last child, and returns the ids of the subtree's top-level nodes. |
| `SurfaceTree::invalidate_rows` | method | Tells the host that the rows of the list or table `target` changed although its props did not; it fetches `rows` again, or all of them for `None`. Unlike rebuilding the list, this keeps its scroll position. In an extension exported as 0.1 it queues an unchanged `set-props` of the list instead, after which a 0.1 host fetches all its rows again. |
| `SurfaceTree::is_empty` | method |  |
| `SurfaceTree::is_mounted` | method |  |
| `SurfaceTree::is_unsynced` | method | Whether a failed push left the host behind the mirror. |
| `SurfaceTree::len` | method |  |
| `SurfaceTree::limits` | method |  |
| `SurfaceTree::mount` | method | Builds the surface's tree from scratch and returns it for `mount`. Pending patches are dropped. Ids keep counting up from earlier trees, so a node id from a previous mount never names a node of the new tree. Fails, keeping the previous tree, if the new one has more nodes than a tree may hold (`max_nodes`, 16384), the host's cap for a mount and a `replace`. So a tree grown past 2048 nodes with `insert` can always be mounted again. |
| `SurfaceTree::new` | method |  |
| `SurfaceTree::parent` | method |  |
| `SurfaceTree::pending` | method |  |
| `SurfaceTree::props` | method |  |
| `SurfaceTree::push` | method | Pushes the queued update to `surface_id` with `push_update`. On `Err` the host did not take it, so the mirror is ahead of the host: the next `SurfaceTree::take_update` (or push) sends the whole tree as a `replace`. |
| `SurfaceTree::rebuild` | method | Replaces node `id` and its subtree with the nodes `build` adds, at the same position, and returns their ids. The new nodes get new ids, so the host treats them as new: a list rebuilt this way has all of its rows requested again, which `set_props` cannot force when the rows changed but the list's props did not. Checked as a whole: on error nothing is removed. |
| `SurfaceTree::remove` | method | Removes a node and its whole subtree. |
| `SurfaceTree::replace` | method | Like `SurfaceTree::mount`, but returns the tree as a `replace` update, for when patching would be more work than rebuilding. |
| `SurfaceTree::roots` | method |  |
| `SurfaceTree::set_props` | method | Replaces a node's props. Returns whether they changed; unchanged props queue no patch. |
| `SurfaceTree::set_props_from_host` | method | Records props the host already shows without queuing a patch. `SurfaceTree::apply_event` covers typed input text; this is for other cases where the host and the extension agree on a change the host made itself. |
| `SurfaceTree::take_update` | method | Patches queued since the last call, as an update for `handle-event` or `push_update`. After a failed `SurfaceTree::push` it is the whole tree as a `replace` instead. |
| `SurfaceTree::to_nodes` | method | The whole live tree as a flat, parents-first node list, as the host holds it. |
| `SurfaceTree::with_limits` | method | A tree checked against `limits` instead of the host's defaults. |
| `TableProps::new` | method |  |
| `TextProps::bold` | method |  |
| `TextProps::color` | method |  |
| `TextProps::monospace` | method |  |
| `TextProps::new` | method |  |
| `TreeBuilder` | struct | Builds a flat node list from nested code. Flex nodes take a closure that builds their children; every node gets a fresh id and its parent's id, and parents precede children. |
| `TreeBuilder::button` | method |  |
| `TreeBuilder::column` | method |  |
| `TreeBuilder::diff_view` | method |  |
| `TreeBuilder::finish` | method |  |
| `TreeBuilder::flex` | method | Adds a flex node and the children `build_children` adds, and returns the node's id. Only flex nodes have children; the host rejects children under any other kind. |
| `TreeBuilder::icon` | method |  |
| `TreeBuilder::image` | method | `fyx:ui` 0.3: an image node (see `ImageProps`). |
| `TreeBuilder::input` | method |  |
| `TreeBuilder::list` | method | A virtualized list; the host pulls its rows with `list-rows`. |
| `TreeBuilder::markdown` | method |  |
| `TreeBuilder::new` | method | A builder whose top-level nodes have no parent. |
| `TreeBuilder::node` | method | Adds a node without children and returns its id. |
| `TreeBuilder::nodes` | method |  |
| `TreeBuilder::row` | method |  |
| `TreeBuilder::table` | method |  |
| `TreeBuilder::text` | method |  |
| `TreeBuilder::under` | method | A builder whose top-level nodes are children of `parent`, for inserting into a live tree. |
| `TreeError` | enum |  |
| `TreeError::InvalidRow` | variant | A list row the host would reject, reported by `ui_ext::Tree::from_row`. |
| `TreeError::IsRoot` | variant |  |
| `TreeError::NotAChild` | variant |  |
| `TreeError::NotAContainer` | variant | Only flex nodes have children: inserting under `node`, or giving it props of `kind` while it has children, would be rejected by the host. |
| `TreeError::NotAList` | variant | `invalidate-rows` names a node that is not a list or a table. |
| `TreeError::NotMounted` | variant |  |
| `TreeError::TreeTooLarge` | variant |  |
| `TreeError::UnknownNode` | variant |  |
| `TreeError::UpdateTooLarge` | variant |  |
| `TreeLimits` | re-export | The host's caps on a surface, which `SurfaceTree` enforces before the host sees an update. |
| `bindings` | module |  |
| `bindings` | module | The bindings of the SDK's world: `fyx:ui@0.2.0`, or the staged `fyx:ui@0.3.0` with the `fyx-ui-0-3` feature. Only a 0.3 build may link the 0.3 world description, or a 0.1 or 0.2 component would list 0.3 interfaces. |
| `bindings::fyx::ui::types::*` | re-export | `bindings::fyx::ui::types::*` |
| `host::*` | re-export | `host::*` |
| `list_row` | fn | Builds one row of a virtualized list; its ids start at `ROW_NODE_ID_BASE`. `build` must add exactly one top-level node, the row's root, and the row must pass the host's own checks (`ui_ext::Tree::from_row`), or the host would reject the whole `list-rows` answer. |
| `list_rows` | fn | Answers a `list-rows(start, count)` request for a list of `row_count` rows by calling `build_row` for each row that exists. See `list_row`; a row the host would reject is logged and left out, so the other rows still show. |
| `register_extension!` | macro | Exports a type implementing `Extension`. Call it once, at the root of the extension's `cdylib` crate. |
| `row_range` | fn | The row indices of a `list-rows(start, count)` request that exist in a list of `row_count` rows. The host may ask past the end while the list shrinks. |
