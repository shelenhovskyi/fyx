//! Guest SDK for Fyx UI extensions written in Rust and compiled to `wasm32-wasip2`.
//!
//! Apache-2.0 so that extension authors, proprietary ones included, stay outside the GPL.
//!
//! An extension implements [`Extension`] and exports it with [`register_extension!`]. Trees are
//! written as nested Rust code with [`TreeBuilder`], which assigns node ids and parents and emits
//! the flat, parents-first `list<node>` the `fyx:ui` world expects. [`SurfaceTree`] remembers the
//! live tree of one surface so `handle-event` can answer with a few patches instead of a full
//! `replace`:
//!
//! ```ignore
//! use ui_ext_sdk::*;
//!
//! #[derive(Default)]
//! struct Hello {
//!     tree: SurfaceTree,
//!     greeting: Option<NodeId>,
//!     button: Option<NodeId>,
//! }
//!
//! impl Extension for Hello {
//!     fn new() -> Self {
//!         Self::default()
//!     }
//!
//!     fn surfaces(&self) -> Vec<Surface> {
//!         vec![Surface::panel("hello", "Hello", DockPosition::Right, "sparkle", 240)]
//!     }
//!
//!     fn mount(&mut self, _surface_id: &str) -> Result<Vec<Node>, String> {
//!         Ok(self.tree.mount(|tree| {
//!             tree.column(|tree| {
//!                 self.greeting = Some(tree.text("Hello"));
//!                 self.button = Some(tree.button("Wave"));
//!             });
//!         })?)
//!     }
//!
//!     fn handle_event(&mut self, _surface_id: &str, event: UiEvent) -> Result<Update, String> {
//!         if let (UiEvent::Click(id), Some(greeting)) = (event, self.greeting)
//!             && Some(id) == self.button
//!         {
//!             self.tree.set_props(greeting, TextProps::new("Hello, world").bold())?;
//!         }
//!         Ok(self.tree.take_update())
//!     }
//! }
//!
//! register_extension!(Hello);
//! ```

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;

mod host_model;

pub mod bindings {
    wit_bindgen::generate!({
        path: "../ui_ext/wit",
        world: "ui-extension",
        pub_export_macro: true,
        default_bindings_module: "ui_ext_sdk::bindings",
        additional_derives: [PartialEq, Eq],
    });
}

pub use bindings::fyx::ui::host::LogLevel;
pub use bindings::fyx::ui::types::*;
/// The host's caps on a surface, which [`SurfaceTree`] enforces before the host sees an update.
pub use ui_ext::TreeLimits;

/// The `fyx:ui` WIT version this SDK is generated from, as (major, minor, patch).
pub const API_VERSION: (u16, u16, u16) = (0, 1, 0);

/// Name of the custom section [`register_extension!`] adds to every component, from which the host
/// reads [`API_VERSION`] before instantiating it.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";

/// [`API_VERSION`] as stored in the section: three big-endian `u16`s, as in Zed's
/// `zed:api-version`.
pub const API_VERSION_SECTION_BYTES: [u8; 6] = {
    let (major, minor, patch) = API_VERSION;
    let [major_high, major_low] = major.to_be_bytes();
    let [minor_high, minor_low] = minor.to_be_bytes();
    let [patch_high, patch_low] = patch.to_be_bytes();
    [
        major_high, major_low, minor_high, minor_low, patch_high, patch_low,
    ]
};

/// First id handed out inside a [`ListRow`] subtree.
///
/// Rows are addressed by `(list node, index)` and rebuilt on every scroll, so their node ids are
/// only unique within one row. They start at this value so they can never be mistaken for a node
/// of the surface's main tree, whose ids [`IdAllocator`] hands out from 1 upwards.
pub const ROW_NODE_ID_BASE: NodeId = 0x8000_0000;

/// Writes a message to the host's log. Off WASM, where there is no host (unit tests of an
/// extension), it goes to stderr.
pub fn log(level: LogLevel, message: &str) {
    #[cfg(target_arch = "wasm32")]
    bindings::fyx::ui::host::log(level, message);
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[{level:?}] {message}");
}

/// Sends an update the extension initiated itself (timers, background work) for `surface_id`.
///
/// The host applies it on its next frame. Use [`SurfaceTree::take_update`] to produce it, so the
/// tree the extension remembers matches the one the host draws. Off WASM the update is kept for
/// [`take_pushed_updates`].
pub fn push_update(surface_id: &str, update: &Update) {
    #[cfg(target_arch = "wasm32")]
    bindings::fyx::ui::host::push_update(surface_id, update);
    #[cfg(not(target_arch = "wasm32"))]
    PUSHED_UPDATES.with_borrow_mut(|pushed| pushed.push((surface_id.to_string(), update.clone())));
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static PUSHED_UPDATES: RefCell<Vec<(String, Update)>> = const { RefCell::new(Vec::new()) };
}

/// The updates [`push_update`] sent on this thread since the last call, for unit tests of an
/// extension on the host target.
#[cfg(not(target_arch = "wasm32"))]
pub fn take_pushed_updates() -> Vec<(String, Update)> {
    PUSHED_UPDATES.with_borrow_mut(std::mem::take)
}

/// A UI extension. One instance lives for the lifetime of the WASM instance; the host calls it
/// from a single thread, one call at a time.
pub trait Extension: 'static {
    fn new() -> Self
    where
        Self: Sized;

    /// Every surface the extension contributes; called once after instantiation.
    fn surfaces(&self) -> Vec<Surface>;

    /// The initial tree of `surface_id`, parents first. [`SurfaceTree::mount`] builds one.
    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String>;

    fn handle_event(&mut self, _surface_id: &str, _event: UiEvent) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }

    /// Rows `start..start + count` of the virtualized list `list_node`. [`list_rows`] clamps the
    /// range to the list's length and builds each row.
    fn list_rows(
        &mut self,
        _surface_id: &str,
        _list_node: NodeId,
        _start: u32,
        _count: u32,
    ) -> Vec<ListRow> {
        Vec::new()
    }
}

/// Exports a type implementing [`Extension`] as the `fyx:ui/ui-extension` world. Call it once,
/// at the root of the extension's `cdylib` crate.
#[macro_export]
macro_rules! register_extension {
    ($extension_type:ty) => {
        struct __FyxUiExtension;

        fn __fyx_new_extension() -> ::std::boxed::Box<dyn $crate::Extension> {
            ::std::boxed::Box::new(<$extension_type as $crate::Extension>::new())
        }

        impl $crate::bindings::Guest for __FyxUiExtension {
            fn surfaces() -> ::std::vec::Vec<$crate::Surface> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.surfaces()
                })
            }

            fn mount(
                surface_id: ::std::string::String,
            ) -> ::std::result::Result<::std::vec::Vec<$crate::Node>, ::std::string::String> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.mount(&surface_id)
                })
            }

            fn handle_event(
                surface_id: ::std::string::String,
                event: $crate::UiEvent,
            ) -> ::std::result::Result<$crate::Update, ::std::string::String> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.handle_event(&surface_id, event)
                })
            }

            fn list_rows(
                surface_id: ::std::string::String,
                list_node: $crate::NodeId,
                start: u32,
                count: u32,
            ) -> ::std::vec::Vec<$crate::ListRow> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.list_rows(&surface_id, list_node, start, count)
                })
            }
        }

        // In the extension's own crate rather than the SDK's rlib, so the linker cannot drop it
        // as an unreferenced archive member.
        #[cfg(target_arch = "wasm32")]
        #[unsafe(link_section = "fyx:ui-api-version")]
        #[used]
        static __FYX_UI_API_VERSION: [u8; 6] = $crate::API_VERSION_SECTION_BYTES;

        $crate::bindings::export!(__FyxUiExtension with_types_in $crate::bindings);
    };
}

#[doc(hidden)]
pub mod __private {
    use super::*;

    thread_local! {
        static EXTENSION: RefCell<Option<Box<dyn Extension>>> = const { RefCell::new(None) };
    }

    /// The export functions are free functions, so the instance lives in a thread local that is
    /// filled on the first call. WASM components are single-threaded and the host never calls
    /// back into the extension while a call is running, so the borrow cannot be contended.
    pub fn with_extension<R>(
        new_extension: fn() -> Box<dyn Extension>,
        call: impl FnOnce(&mut dyn Extension) -> R,
    ) -> R {
        EXTENSION
            .with_borrow_mut(|extension| call(extension.get_or_insert_with(new_extension).as_mut()))
    }
}

/// Hands out the node ids of one surface. Ids only ever increase, so an id is never reused for a
/// different node while the surface lives, as the `fyx:ui` contract requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdAllocator {
    next: NodeId,
}

impl IdAllocator {
    pub const fn new() -> Self {
        Self::starting_at(1)
    }

    pub const fn starting_at(first: NodeId) -> Self {
        Self { next: first }
    }

    pub fn allocate(&mut self) -> NodeId {
        let id = self.next;
        self.next += 1;
        id
    }

    /// The id the next call to [`IdAllocator::allocate`] returns.
    pub fn peek(&self) -> NodeId {
        self.next
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a flat node list from nested code. Flex nodes take a closure that builds their
/// children; every node gets a fresh id and its parent's id, and parents precede children.
pub struct TreeBuilder<'a> {
    ids: &'a mut IdAllocator,
    nodes: Vec<Node>,
    parent: Option<NodeId>,
}

impl<'a> TreeBuilder<'a> {
    /// A builder whose top-level nodes have no parent.
    pub fn new(ids: &'a mut IdAllocator) -> Self {
        Self {
            ids,
            nodes: Vec::new(),
            parent: None,
        }
    }

    /// A builder whose top-level nodes are children of `parent`, for inserting into a live tree.
    pub fn under(ids: &'a mut IdAllocator, parent: NodeId) -> Self {
        Self {
            ids,
            nodes: Vec::new(),
            parent: Some(parent),
        }
    }

    /// Adds a node without children and returns its id.
    pub fn node(&mut self, props: impl Into<Props>) -> NodeId {
        let id = self.ids.allocate();
        self.nodes.push(Node {
            id,
            parent: self.parent,
            props: props.into(),
        });
        id
    }

    /// Adds a flex node and the children `build_children` adds, and returns the node's id. Only
    /// flex nodes have children; the host rejects children under any other kind.
    pub fn flex(&mut self, props: FlexProps, build_children: impl FnOnce(&mut Self)) -> NodeId {
        let id = self.node(props);
        let outer_parent = self.parent.replace(id);
        build_children(self);
        self.parent = outer_parent;
        id
    }

    pub fn column(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.flex(FlexProps::column(), build_children)
    }

    pub fn row(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.flex(FlexProps::row(), build_children)
    }

    pub fn text(&mut self, props: impl Into<TextProps>) -> NodeId {
        self.node(props.into())
    }

    pub fn button(&mut self, props: impl Into<ButtonProps>) -> NodeId {
        self.node(props.into())
    }

    pub fn input(&mut self, props: InputProps) -> NodeId {
        self.node(props)
    }

    /// A virtualized list; the host pulls its rows with `list-rows`.
    pub fn list(&mut self, props: ListProps) -> NodeId {
        self.node(props)
    }

    pub fn table(&mut self, props: TableProps) -> NodeId {
        self.node(props)
    }

    pub fn icon(&mut self, props: impl Into<IconProps>) -> NodeId {
        self.node(props.into())
    }

    pub fn markdown(&mut self, source: impl Into<String>) -> NodeId {
        self.node(Props::Markdown(source.into()))
    }

    pub fn diff_view(&mut self, props: DiffViewProps) -> NodeId {
        self.node(props)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn finish(self) -> Vec<Node> {
        self.nodes
    }
}

/// Builds one row of a virtualized list; its ids start at [`ROW_NODE_ID_BASE`]. `build` must add
/// exactly one top-level node, the row's root, and the row must pass the host's own checks
/// (`ui_ext::Tree::from_row`), or the host would reject the whole `list-rows` answer.
pub fn list_row(index: u32, build: impl FnOnce(&mut TreeBuilder)) -> Result<ListRow, TreeError> {
    let mut ids = IdAllocator::starting_at(ROW_NODE_ID_BASE);
    let mut tree = TreeBuilder::new(&mut ids);
    build(&mut tree);
    let row = ListRow {
        index,
        nodes: tree.finish(),
    };
    ui_ext::Tree::from_row(
        row.nodes.iter().map(Into::into).collect(),
        TreeLimits::default(),
    )
    .map_err(TreeError::InvalidRow)?;
    Ok(row)
}

/// The row indices of a `list-rows(start, count)` request that exist in a list of `row_count`
/// rows. The host may ask past the end while the list shrinks.
pub fn row_range(start: u32, count: u32, row_count: u32) -> Range<u32> {
    let end = start.saturating_add(count).min(row_count);
    start.min(end)..end
}

/// Answers a `list-rows(start, count)` request for a list of `row_count` rows by calling
/// `build_row` for each row that exists. See [`list_row`]; a row the host would reject is logged
/// and left out, so the other rows still show.
pub fn list_rows(
    start: u32,
    count: u32,
    row_count: u32,
    mut build_row: impl FnMut(u32, &mut TreeBuilder),
) -> Vec<ListRow> {
    row_range(start, count, row_count)
        .filter_map(
            |index| match list_row(index, |tree| build_row(index, tree)) {
                Ok(row) => Some(row),
                Err(error) => {
                    log(
                        LogLevel::Error,
                        &format!("list row {index} left out: {error}"),
                    );
                    None
                }
            },
        )
        .collect()
}

/// An ordered batch of patches, turned into an `update` with [`PatchSet::into_update`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PatchSet {
    patches: Vec<Patch>,
}

impl PatchSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_props(&mut self, id: NodeId, props: impl Into<Props>) -> &mut Self {
        self.patches.push(Patch::SetProps((id, props.into())));
        self
    }

    /// Inserts one node under `node.parent`, before the sibling `before` or at the end.
    pub fn insert(&mut self, node: Node, before: Option<NodeId>) -> &mut Self {
        self.patches.push(Patch::Insert(InsertOp { node, before }));
        self
    }

    /// Inserts a subtree in parents-first order, such as one built with [`TreeBuilder::under`].
    /// Its top-level nodes (those whose parent is outside `nodes`) go before `before`, keeping
    /// their order; deeper nodes are appended to their new parents.
    pub fn insert_subtree(&mut self, nodes: Vec<Node>, before: Option<NodeId>) -> &mut Self {
        let subtree_ids = nodes.iter().map(|node| node.id).collect::<HashSet<_>>();
        for node in nodes {
            let is_top_level = node
                .parent
                .is_none_or(|parent| !subtree_ids.contains(&parent));
            let before = if is_top_level { before } else { None };
            self.insert(node, before);
        }
        self
    }

    /// Removes the node and its whole subtree.
    pub fn remove(&mut self, id: NodeId) -> &mut Self {
        self.patches.push(Patch::Remove(id));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn len(&self) -> usize {
        self.patches.len()
    }

    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }

    /// `unchanged` when there is nothing to send, `patches` otherwise.
    pub fn into_update(self) -> Update {
        if self.patches.is_empty() {
            Update::Unchanged
        } else {
            Update::Patches(self.patches)
        }
    }
}

impl From<PatchSet> for Update {
    fn from(patches: PatchSet) -> Self {
        patches.into_update()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    NotMounted,
    UnknownNode(NodeId),
    NotAChild {
        parent: NodeId,
        child: NodeId,
    },
    IsRoot(NodeId),
    /// Only flex nodes have children: inserting under `node`, or giving it props of `kind` while
    /// it has children, would be rejected by the host.
    NotAContainer {
        node: NodeId,
        kind: &'static str,
    },
    UpdateTooLarge {
        count: usize,
        max: usize,
    },
    TreeTooLarge {
        count: usize,
        max: usize,
    },
    /// A list row the host would reject, reported by `ui_ext::Tree::from_row`.
    InvalidRow(ui_ext::TreeError),
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMounted => write!(formatter, "surface is not mounted"),
            Self::UnknownNode(id) => write!(formatter, "node {id} is not in the surface's tree"),
            Self::NotAChild { parent, child } => {
                write!(formatter, "node {child} is not a child of node {parent}")
            }
            Self::IsRoot(id) => write!(formatter, "node {id} is a root of the surface's tree"),
            Self::NotAContainer { node, kind } => write!(
                formatter,
                "node {node} would be a {kind} with children; only flex nodes have children"
            ),
            Self::UpdateTooLarge { count, max } => write!(
                formatter,
                "update would carry {count} nodes or patches, the cap is {max}"
            ),
            Self::TreeTooLarge { count, max } => {
                write!(formatter, "tree would have {count} nodes, the cap is {max}")
            }
            Self::InvalidRow(error) => write!(formatter, "invalid list row: {error}"),
        }
    }
}

impl std::error::Error for TreeError {}

impl From<TreeError> for String {
    fn from(error: TreeError) -> Self {
        error.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveNode {
    parent: Option<NodeId>,
    props: Props,
    children: Vec<NodeId>,
}

/// The extension-side copy of one surface's tree, mirroring what the host has applied.
///
/// Edits are recorded and queued as patches; [`SurfaceTree::take_update`] hands the queue to the
/// host. A `set_props` that does not change anything is dropped, so re-setting every node after a
/// state change sends only what actually changed.
///
/// The host applies an update atomically, rejects the whole batch if one patch breaks its rules,
/// and does not tell the extension. So every edit is checked against those rules first: children
/// only under flex nodes, at most [`TreeLimits::max_nodes_per_update`] queued patches and at most
/// [`TreeLimits::max_nodes`] nodes. An edit that breaks one returns a [`TreeError`] and leaves the
/// mirror and the queue as they were (ids it allocated stay unused), so the queue is always a
/// batch the host accepts.
#[derive(Debug, Default)]
pub struct SurfaceTree {
    ids: IdAllocator,
    limits: TreeLimits,
    mounted: bool,
    nodes: HashMap<NodeId, LiveNode>,
    roots: Vec<NodeId>,
    pending: PatchSet,
}

impl SurfaceTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// A tree checked against `limits` instead of the host's defaults.
    pub fn with_limits(limits: TreeLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    /// Builds the surface's tree from scratch and returns it for `mount`. Pending patches are
    /// dropped. Ids keep counting up from earlier trees, so a node id from a previous mount never
    /// names a node of the new tree. Fails, keeping the previous tree, if the new one has more
    /// nodes than one update may carry.
    pub fn mount(&mut self, build: impl FnOnce(&mut TreeBuilder)) -> Result<Vec<Node>, TreeError> {
        let mut tree = TreeBuilder::new(&mut self.ids);
        build(&mut tree);
        let nodes = tree.finish();
        self.check_update_size(nodes.len())?;
        self.check_tree_size(nodes.len())?;

        self.nodes.clear();
        self.roots.clear();
        self.pending = PatchSet::new();
        self.mounted = true;
        self.record(&nodes, None);
        Ok(nodes)
    }

    /// Like [`SurfaceTree::mount`], but returns the tree as a `replace` update, for when patching
    /// would be more work than rebuilding.
    pub fn replace(&mut self, build: impl FnOnce(&mut TreeBuilder)) -> Result<Update, TreeError> {
        Ok(Update::Replace(self.mount(build)?))
    }

    pub fn is_mounted(&self) -> bool {
        self.mounted
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn props(&self, id: NodeId) -> Option<&Props> {
        self.nodes.get(&id).map(|node| &node.props)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id).and_then(|node| node.parent)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes
            .get(&id)
            .map_or(&[], |node| node.children.as_slice())
    }

    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The whole live tree as a flat, parents-first node list, as the host holds it.
    pub fn to_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut stack = self.roots.iter().rev().copied().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if let Some(node) = self.nodes.get(&id) {
                nodes.push(Node {
                    id,
                    parent: node.parent,
                    props: node.props.clone(),
                });
                stack.extend(node.children.iter().rev().copied());
            }
        }
        nodes
    }

    /// Replaces a node's props. Returns whether they changed; unchanged props queue no patch.
    pub fn set_props(&mut self, id: NodeId, props: impl Into<Props>) -> Result<bool, TreeError> {
        let props = props.into();
        let node = self.node(id)?;
        if node.props == props {
            return Ok(false);
        }
        Self::check_container(id, node, &props)?;
        self.check_update_size(self.pending.len() + 1)?;

        self.node_mut(id)?.props = props.clone();
        self.pending.set_props(id, props);
        Ok(true)
    }

    /// Records props the host already shows without queuing a patch, such as an input's value
    /// after an `input-changed` event: echoing the user's text back could fight their typing.
    pub fn set_props_from_host(
        &mut self,
        id: NodeId,
        props: impl Into<Props>,
    ) -> Result<(), TreeError> {
        let props = props.into();
        Self::check_container(id, self.node(id)?, &props)?;
        self.node_mut(id)?.props = props;
        Ok(())
    }

    /// Builds a subtree under the flex node `parent`, before its child `before` or after its last
    /// child, and returns the ids of the subtree's top-level nodes.
    pub fn insert(
        &mut self,
        parent: NodeId,
        before: Option<NodeId>,
        build: impl FnOnce(&mut TreeBuilder),
    ) -> Result<Vec<NodeId>, TreeError> {
        let position = self.insert_position(parent, before)?;
        let mut tree = TreeBuilder::under(&mut self.ids, parent);
        build(&mut tree);
        let nodes = tree.finish();
        self.check_update_size(self.pending.len() + nodes.len())?;
        self.check_tree_size(self.nodes.len() + nodes.len())?;
        Ok(self.commit_insert(parent, position, before, nodes))
    }

    /// Removes a node and its whole subtree.
    pub fn remove(&mut self, id: NodeId) -> Result<(), TreeError> {
        self.node(id)?;
        self.check_update_size(self.pending.len() + 1)?;
        self.commit_remove(id);
        Ok(())
    }

    /// Replaces node `id` and its subtree with the nodes `build` adds, at the same position, and
    /// returns their ids. The new nodes get new ids, so the host treats them as new: a list
    /// rebuilt this way has all of its rows requested again, which `set_props` cannot force when
    /// the rows changed but the list's props did not. Checked as a whole: on error nothing is
    /// removed.
    pub fn rebuild(
        &mut self,
        id: NodeId,
        build: impl FnOnce(&mut TreeBuilder),
    ) -> Result<Vec<NodeId>, TreeError> {
        let parent = self.node(id)?.parent.ok_or(TreeError::IsRoot(id))?;
        let siblings = self.children(parent);
        let position = siblings
            .iter()
            .position(|&sibling| sibling == id)
            .ok_or(TreeError::NotAChild { parent, child: id })?;
        let next_sibling = siblings.get(position + 1).copied();
        let removed = self.subtree_len(id);

        let mut tree = TreeBuilder::under(&mut self.ids, parent);
        build(&mut tree);
        let nodes = tree.finish();
        self.check_update_size(self.pending.len() + 1 + nodes.len())?;
        self.check_tree_size(self.nodes.len() - removed + nodes.len())?;

        self.commit_remove(id);
        Ok(self.commit_insert(parent, position, next_sibling, nodes))
    }

    /// Removes every child of `parent`.
    pub fn clear_children(&mut self, parent: NodeId) -> Result<(), TreeError> {
        let children = self.node(parent)?.children.clone();
        self.check_update_size(self.pending.len() + children.len())?;
        for child in children {
            self.commit_remove(child);
        }
        Ok(())
    }

    /// Patches queued since the last call, as an update for `handle-event` or [`push_update`].
    pub fn take_update(&mut self) -> Update {
        std::mem::take(&mut self.pending).into_update()
    }

    pub fn pending(&self) -> &PatchSet {
        &self.pending
    }

    fn node(&self, id: NodeId) -> Result<&LiveNode, TreeError> {
        if !self.mounted {
            return Err(TreeError::NotMounted);
        }
        self.nodes.get(&id).ok_or(TreeError::UnknownNode(id))
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut LiveNode, TreeError> {
        if !self.mounted {
            return Err(TreeError::NotMounted);
        }
        self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))
    }

    fn check_container(id: NodeId, node: &LiveNode, props: &Props) -> Result<(), TreeError> {
        if node.children.is_empty() {
            return Ok(());
        }
        let host_props = ui_ext::Props::from(props);
        if host_props.is_container() {
            Ok(())
        } else {
            Err(TreeError::NotAContainer {
                node: id,
                kind: host_props.kind_name(),
            })
        }
    }

    fn check_update_size(&self, count: usize) -> Result<(), TreeError> {
        let max = self.limits.max_nodes_per_update;
        if count > max {
            return Err(TreeError::UpdateTooLarge { count, max });
        }
        Ok(())
    }

    fn check_tree_size(&self, count: usize) -> Result<(), TreeError> {
        let max = self.limits.max_nodes;
        if count > max {
            return Err(TreeError::TreeTooLarge { count, max });
        }
        Ok(())
    }

    fn insert_position(&self, parent: NodeId, before: Option<NodeId>) -> Result<usize, TreeError> {
        let parent_node = self.node(parent)?;
        let host_props = ui_ext::Props::from(&parent_node.props);
        if !host_props.is_container() {
            return Err(TreeError::NotAContainer {
                node: parent,
                kind: host_props.kind_name(),
            });
        }
        let siblings = &parent_node.children;
        match before {
            Some(before) => siblings
                .iter()
                .position(|&sibling| sibling == before)
                .ok_or(TreeError::NotAChild {
                    parent,
                    child: before,
                }),
            None => Ok(siblings.len()),
        }
    }

    fn subtree_len(&self, id: NodeId) -> usize {
        let mut count = 0;
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            count += 1;
            stack.extend_from_slice(self.children(id));
        }
        count
    }

    fn commit_insert(
        &mut self,
        parent: NodeId,
        position: usize,
        before: Option<NodeId>,
        nodes: Vec<Node>,
    ) -> Vec<NodeId> {
        let top_level = nodes
            .iter()
            .filter(|node| node.parent == Some(parent))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        self.record(&nodes, Some((parent, position)));
        self.pending.insert_subtree(nodes, before);
        top_level
    }

    fn commit_remove(&mut self, id: NodeId) {
        let parent = self.nodes.get(&id).and_then(|node| node.parent);
        let siblings = match parent.and_then(|parent| self.nodes.get_mut(&parent)) {
            Some(parent) => &mut parent.children,
            None => &mut self.roots,
        };
        siblings.retain(|&sibling| sibling != id);

        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.nodes.remove(&id) {
                stack.extend(node.children);
            }
        }
        self.pending.remove(id);
    }

    /// Records parents-first `nodes`; top-level ones are placed at `position` among the
    /// children of `parent`, or appended to the roots.
    fn record(&mut self, nodes: &[Node], insert_at: Option<(NodeId, usize)>) {
        let mut position = insert_at.map(|(_, position)| position);
        for node in nodes {
            self.nodes.insert(
                node.id,
                LiveNode {
                    parent: node.parent,
                    props: node.props.clone(),
                    children: Vec::new(),
                },
            );
            let is_top_level = insert_at.map(|(parent, _)| parent) == node.parent;
            let siblings = match node.parent.and_then(|parent| self.nodes.get_mut(&parent)) {
                Some(parent) => &mut parent.children,
                None => &mut self.roots,
            };
            match position.as_mut().filter(|_| is_top_level) {
                Some(position) => {
                    let index = (*position).min(siblings.len());
                    siblings.insert(index, node.id);
                    *position = index + 1;
                }
                None => siblings.push(node.id),
            }
        }
    }
}

impl Surface {
    pub fn panel(
        id: impl Into<String>,
        title: impl Into<String>,
        position: DockPosition,
        icon: impl Into<String>,
        default_size: u32,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: SurfaceKind::Panel(PanelDescriptor {
                position,
                icon: icon.into(),
                default_size,
            }),
        }
    }

    pub fn status_item(
        id: impl Into<String>,
        title: impl Into<String>,
        side: StatusBarSide,
        priority: i32,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: SurfaceKind::StatusItem(StatusItemDescriptor { side, priority }),
        }
    }
}

impl FlexProps {
    /// A column with no gap, no padding and children aligned to the start.
    pub fn column() -> Self {
        Self {
            direction: FlexDirection::Column,
            gap: 0,
            padding: 0,
            grow: false,
            align: Align::Start,
            background: None,
        }
    }

    /// A row with no gap, no padding and children aligned to the start.
    pub fn row() -> Self {
        Self {
            direction: FlexDirection::Row,
            ..Self::column()
        }
    }

    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn padding(mut self, padding: u16) -> Self {
        self.padding = padding;
        self
    }

    pub fn grow(mut self) -> Self {
        self.grow = true;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn background(mut self, background: ColorToken) -> Self {
        self.background = Some(background);
        self
    }
}

impl TextProps {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            color: None,
            bold: false,
            monospace: false,
        }
    }

    pub fn color(mut self, color: ColorToken) -> Self {
        self.color = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }
}

impl From<&str> for TextProps {
    fn from(content: &str) -> Self {
        Self::new(content)
    }
}

impl From<String> for TextProps {
    fn from(content: String) -> Self {
        Self::new(content)
    }
}

impl ButtonProps {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            style: ButtonStyle::Default,
            disabled: false,
        }
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl From<&str> for ButtonProps {
    fn from(label: &str) -> Self {
        Self::new(label)
    }
}

impl From<String> for ButtonProps {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

impl InputProps {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            placeholder: String::new(),
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
}

impl ListProps {
    pub fn new(row_count: u32, row_height: u16) -> Self {
        Self {
            row_count,
            row_height,
        }
    }
}

impl TableProps {
    pub fn new(columns: impl IntoIterator<Item = impl Into<String>>, row_count: u32) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            row_count,
        }
    }
}

impl IconProps {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            color: None,
        }
    }

    pub fn color(mut self, color: ColorToken) -> Self {
        self.color = Some(color);
        self
    }
}

impl From<&str> for IconProps {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl DiffViewProps {
    pub fn new(old_text: impl Into<String>, new_text: impl Into<String>) -> Self {
        Self {
            old_text: old_text.into(),
            new_text: new_text.into(),
            language: None,
        }
    }

    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }
}

macro_rules! impl_into_props {
    ($($props_type:ident => $variant:ident),* $(,)?) => {
        $(
            impl From<$props_type> for Props {
                fn from(props: $props_type) -> Self {
                    Props::$variant(props)
                }
            }
        )*
    };
}

impl_into_props! {
    FlexProps => Flex,
    TextProps => Text,
    ButtonProps => Button,
    InputProps => Input,
    ListProps => List,
    TableProps => Table,
    IconProps => Icon,
    DiffViewProps => DiffView,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: &str) -> Props {
        TextProps::new(content).into()
    }

    fn to_host(nodes: &[Node]) -> Vec<ui_ext::Node> {
        nodes.iter().map(Into::into).collect()
    }

    /// The host's tree after `mount`, built by `ui_ext` itself.
    fn host_tree(nodes: &[Node]) -> ui_ext::Tree {
        ui_ext::Tree::from_nodes(to_host(nodes), TreeLimits::default())
            .expect("the host accepts the mount")
    }

    /// Applies an update with the host's own rules and checks the host now holds the SDK's view.
    fn apply(host: &mut ui_ext::Tree, surface: &mut SurfaceTree) {
        host.apply_update((&surface.take_update()).into())
            .expect("the host accepts the update");
        assert_eq!(host.to_nodes(), to_host(&surface.to_nodes()));
    }

    #[test]
    fn builder_flattens_nested_code_parents_first() {
        let mut ids = IdAllocator::new();
        let mut tree = TreeBuilder::new(&mut ids);
        let mut leaves = Vec::new();
        let root = tree.column(|tree| {
            leaves.push(tree.text("title"));
            tree.row(|tree| {
                leaves.push(tree.button("a"));
                leaves.push(tree.button("b"));
            });
            leaves.push(tree.list(ListProps::new(3, 20)));
        });
        let nodes = tree.finish();

        let shape = nodes
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(3)),
                (5, Some(3)),
                (6, Some(1)),
            ]
        );
        assert_eq!(root, 1);
        assert_eq!(leaves, [2, 4, 5, 6]);
        assert_eq!(nodes[1].props, text("title"));
        assert_eq!(ids.peek(), 7);
    }

    #[test]
    fn builder_under_parent_and_sibling_roots() {
        let mut ids = IdAllocator::starting_at(10);
        let mut tree = TreeBuilder::under(&mut ids, 3);
        tree.text("first");
        tree.column(|tree| {
            tree.text("nested");
        });
        let shape = tree
            .finish()
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(shape, [(10, Some(3)), (11, Some(3)), (12, Some(11))]);
    }

    #[test]
    fn props_builders() {
        let flex = FlexProps::row()
            .gap(4)
            .padding(8)
            .grow()
            .background(ColorToken::Surface);
        assert_eq!(flex.direction, FlexDirection::Row);
        assert_eq!((flex.gap, flex.padding, flex.grow), (4, 8, true));
        assert_eq!(flex.background, Some(ColorToken::Surface));

        let button = ButtonProps::new("Go")
            .style(ButtonStyle::Filled)
            .icon("play")
            .disabled(true);
        assert_eq!(
            Props::from(button),
            Props::Button(ButtonProps {
                label: "Go".into(),
                icon: Some("play".into()),
                style: ButtonStyle::Filled,
                disabled: true,
            })
        );

        let table = TableProps::new(["File", "Line"], 2);
        assert_eq!(table.columns, ["File", "Line"]);
    }

    #[test]
    fn patch_set_insert_subtree_only_positions_top_level_nodes() {
        let mut ids = IdAllocator::starting_at(20);
        let mut tree = TreeBuilder::under(&mut ids, 1);
        tree.row(|tree| {
            tree.text("child");
        });
        tree.text("second");
        let mut patches = PatchSet::new();
        patches.insert_subtree(tree.finish(), Some(7));

        let befores = patches
            .patches()
            .iter()
            .map(|patch| match patch {
                Patch::Insert(insert) => (insert.node.id, insert.before),
                _ => panic!("expected only inserts"),
            })
            .collect::<Vec<_>>();
        assert_eq!(befores, [(20, Some(7)), (21, None), (22, Some(7))]);
    }

    #[test]
    fn patch_set_into_update() {
        assert_eq!(PatchSet::new().into_update(), Update::Unchanged);

        let mut patches = PatchSet::new();
        patches.set_props(2, TextProps::new("x")).remove(5);
        assert_eq!(patches.len(), 2);
        assert_eq!(
            Update::from(patches),
            Update::Patches(vec![Patch::SetProps((2, text("x"))), Patch::Remove(5),])
        );
    }

    #[test]
    fn surface_tree_skips_unchanged_props() {
        let mut surface = SurfaceTree::new();
        let mut label = 0;
        surface
            .mount(|tree| {
                tree.column(|tree| {
                    label = tree.text("0");
                });
            })
            .expect("mount");

        assert_eq!(surface.set_props(label, TextProps::new("0")), Ok(false));
        assert_eq!(surface.take_update(), Update::Unchanged);

        assert_eq!(surface.set_props(label, TextProps::new("1")), Ok(true));
        assert_eq!(surface.set_props(label, TextProps::new("1")), Ok(false));
        assert_eq!(
            surface.take_update(),
            Update::Patches(vec![Patch::SetProps((label, text("1")))])
        );
        assert_eq!(surface.props(label), Some(&text("1")));
        assert_eq!(surface.take_update(), Update::Unchanged);

        surface
            .set_props_from_host(label, TextProps::new("2"))
            .expect("known node");
        assert_eq!(surface.props(label), Some(&text("2")));
        assert_eq!(surface.take_update(), Update::Unchanged);
    }

    #[test]
    fn surface_tree_rejects_edits_before_mount_and_on_unknown_nodes() {
        let mut surface = SurfaceTree::new();
        assert_eq!(
            surface.set_props(1, TextProps::new("x")),
            Err(TreeError::NotMounted)
        );

        let mut column = 0;
        let mut first = 0;
        surface
            .mount(|tree| {
                column = tree.column(|tree| {
                    first = tree.text("a");
                });
            })
            .expect("mount");
        assert_eq!(surface.remove(99), Err(TreeError::UnknownNode(99)));
        assert_eq!(
            surface.insert(column, Some(column), |tree| {
                tree.text("x");
            }),
            Err(TreeError::NotAChild {
                parent: column,
                child: column,
            })
        );
        assert!(surface.contains(first));
        let message: String = TreeError::UnknownNode(99).into();
        assert_eq!(message, "node 99 is not in the surface's tree");
    }

    #[test]
    fn surface_tree_patches_reproduce_its_tree() {
        let mut surface = SurfaceTree::new();
        let mut column = 0;
        let mut first = 0;
        let mut last = 0;
        let mounted = surface
            .mount(|tree| {
                column = tree.column(|tree| {
                    first = tree.text("first");
                    tree.row(|tree| {
                        tree.button("a");
                        tree.button("b");
                    });
                    last = tree.text("last");
                });
            })
            .expect("mount");
        assert_eq!(mounted, surface.to_nodes());
        let mut host = host_tree(&mounted);

        let inserted = surface
            .insert(column, Some(last), |tree| {
                tree.row(|tree| {
                    tree.text("inserted child");
                });
                tree.text("inserted sibling");
            })
            .expect("insert");
        assert_eq!(inserted.len(), 2);
        surface
            .set_props(first, TextProps::new("first, edited").bold())
            .expect("set props");
        surface
            .insert(column, None, |tree| {
                tree.text("appended");
            })
            .expect("append");
        surface.remove(inserted[0]).expect("remove");
        assert!(!surface.contains(inserted[0] + 1));

        apply(&mut host, &mut surface);

        let order = surface
            .children(column)
            .iter()
            .map(|&id| surface.props(id).cloned())
            .collect::<Vec<_>>();
        assert_eq!(order.len(), 5);
        assert_eq!(
            order[0],
            Some(TextProps::new("first, edited").bold().into())
        );
        assert_eq!(order[2], Some(text("inserted sibling")));
        assert_eq!(order[3], Some(text("last")));
        assert_eq!(order[4], Some(text("appended")));

        let middle = surface.children(column)[2];
        let rebuilt = surface
            .rebuild(middle, |tree| {
                tree.text("rebuilt");
            })
            .expect("rebuild");
        assert!(!surface.contains(middle));
        assert_eq!(surface.children(column)[2], rebuilt[0]);
        assert_eq!(
            surface.rebuild(column, |_| {}),
            Err(TreeError::IsRoot(column))
        );
        apply(&mut host, &mut surface);

        surface.clear_children(column).expect("clear");
        apply(&mut host, &mut surface);
        assert_eq!(surface.len(), 1);
        assert_eq!(surface.roots(), [column]);
    }

    #[test]
    fn surface_tree_never_reuses_ids_across_mounts() {
        let mut surface = SurfaceTree::new();
        let first = surface
            .mount(|tree| {
                tree.text("a");
            })
            .expect("mount");
        let Ok(Update::Replace(second)) = surface.replace(|tree| {
            tree.text("b");
        }) else {
            panic!("expected a replace");
        };
        assert_eq!(first[0].id, 1);
        assert_eq!(second[0].id, 2);
        assert!(!surface.contains(1));
    }

    #[test]
    fn list_rows_clamp_and_use_row_ids() {
        assert_eq!(row_range(0, 10, 4), 0..4);
        assert_eq!(row_range(6, 10, 4), 4..4);
        assert_eq!(row_range(u32::MAX, 10, u32::MAX), u32::MAX..u32::MAX);

        let rows = list_rows(2, 5, 4, |index, tree| {
            tree.row(|tree| {
                tree.text(format!("row {index}"));
            });
        });
        assert_eq!(rows.iter().map(|row| row.index).collect::<Vec<_>>(), [2, 3]);
        let row = &rows[0];
        assert_eq!(row.nodes[0].id, ROW_NODE_ID_BASE);
        assert_eq!(row.nodes[0].parent, None);
        assert_eq!(row.nodes[1].parent, Some(ROW_NODE_ID_BASE));
        assert_eq!(row.nodes[1].props, text("row 2"));
    }

    #[test]
    fn rows_the_host_would_reject_are_left_out() {
        assert_eq!(
            list_row(0, |_| {}),
            Err(TreeError::InvalidRow(ui_ext::TreeError::RowRootCount(0)))
        );
        assert_eq!(
            list_row(0, |tree| {
                tree.text("a");
                tree.text("b");
            }),
            Err(TreeError::InvalidRow(ui_ext::TreeError::RowRootCount(2)))
        );

        let rows = list_rows(0, 3, 3, |index, tree| {
            if index != 1 {
                tree.text(format!("row {index}"));
            }
        });
        assert_eq!(rows.iter().map(|row| row.index).collect::<Vec<_>>(), [0, 2]);
        for row in &rows {
            ui_ext::Tree::from_row(to_host(&row.nodes), TreeLimits::default())
                .expect("the host accepts the row");
        }
    }

    #[test]
    fn host_functions_work_off_wasm() {
        log(LogLevel::Info, "logged to stderr in tests");
        push_update("echo", &Update::Unchanged);
        assert_eq!(
            take_pushed_updates(),
            [("echo".to_string(), Update::Unchanged)]
        );
        assert!(take_pushed_updates().is_empty());
    }

    #[test]
    fn api_version_matches_the_wit_package() {
        assert_eq!(API_VERSION_SECTION_BYTES, [0, 0, 0, 1, 0, 0]);
        let wit = include_str!("../../ui_ext/wit/fyx-ui.wit");
        let (major, minor, patch) = API_VERSION;
        assert!(wit.contains(&format!("package fyx:ui@{major}.{minor}.{patch};")));
    }

    /// A mounted column holding a text and a row with one button; returns (column, text, row).
    fn small_surface(limits: TreeLimits) -> (SurfaceTree, NodeId, NodeId, NodeId) {
        let mut surface = SurfaceTree::with_limits(limits);
        let (mut text_id, mut row_id) = (0, 0);
        let column = surface
            .mount(|tree| {
                tree.column(|tree| {
                    text_id = tree.text("label");
                    row_id = tree.row(|tree| {
                        tree.button("go");
                    });
                });
            })
            .expect("mount")[0]
            .id;
        (surface, column, text_id, row_id)
    }

    fn snapshot(surface: &SurfaceTree) -> (Vec<Node>, PatchSet) {
        (surface.to_nodes(), surface.pending().clone())
    }

    #[test]
    fn insert_only_under_flex() {
        let (mut surface, column, text_id, _) = small_surface(TreeLimits::default());
        surface
            .set_props(text_id, TextProps::new("queued"))
            .expect("set");
        let before = snapshot(&surface);

        let error = surface.insert(text_id, None, |tree| {
            tree.text("child of a text");
        });
        assert_eq!(
            error,
            Err(TreeError::NotAContainer {
                node: text_id,
                kind: "text",
            })
        );
        assert_eq!(snapshot(&surface), before);

        assert_eq!(
            surface.insert(column, Some(column), |_| {}),
            Err(TreeError::NotAChild {
                parent: column,
                child: column,
            })
        );
        assert_eq!(snapshot(&surface), before);
        assert!(
            surface
                .insert(column, None, |tree| {
                    tree.text("ok");
                })
                .is_ok()
        );
    }

    #[test]
    fn flex_with_children_stays_flex() {
        let (mut surface, column, text_id, row_id) = small_surface(TreeLimits::default());
        let before = snapshot(&surface);

        assert_eq!(
            surface.set_props(row_id, TextProps::new("leaf")),
            Err(TreeError::NotAContainer {
                node: row_id,
                kind: "text",
            })
        );
        assert_eq!(
            surface.set_props_from_host(column, ButtonProps::new("leaf")),
            Err(TreeError::NotAContainer {
                node: column,
                kind: "button",
            })
        );
        assert_eq!(snapshot(&surface), before);

        assert_eq!(
            surface.set_props(row_id, FlexProps::column().gap(2)),
            Ok(true)
        );
        assert_eq!(
            surface.set_props(text_id, ButtonProps::new("leaf")),
            Ok(true)
        );
        surface.clear_children(row_id).expect("clear");
        assert_eq!(
            surface.set_props(row_id, TextProps::new("now a leaf")),
            Ok(true)
        );
    }

    #[test]
    fn updates_stay_under_the_patch_cap() {
        let limits = TreeLimits {
            max_nodes_per_update: 4,
            max_nodes: 100,
        };
        let (mut surface, column, text_id, row_id) = small_surface(limits);
        for index in 0..4 {
            surface
                .set_props(text_id, TextProps::new(index.to_string()))
                .expect("under the cap");
        }
        let before = snapshot(&surface);
        let too_large = Err(TreeError::UpdateTooLarge { count: 5, max: 4 });
        assert_eq!(surface.set_props(text_id, TextProps::new("5")), too_large);
        assert_eq!(surface.remove(text_id), too_large.clone().map(|_: bool| ()));
        assert_eq!(
            surface
                .insert(column, None, |tree| {
                    tree.text("x");
                })
                .map(|_| ()),
            too_large.map(|_: bool| ())
        );
        assert_eq!(snapshot(&surface), before);

        assert!(matches!(surface.take_update(), Update::Patches(patches) if patches.len() == 4));
        let error = surface.insert(column, None, |tree| {
            tree.row(|tree| {
                tree.text("a");
                tree.text("b");
                tree.text("c");
                tree.text("d");
            });
        });
        assert_eq!(error, Err(TreeError::UpdateTooLarge { count: 5, max: 4 }));
        let error = surface.rebuild(row_id, |tree| {
            tree.row(|tree| {
                tree.text("a");
                tree.text("b");
                tree.text("c");
            });
        });
        assert_eq!(error, Err(TreeError::UpdateTooLarge { count: 5, max: 4 }));
        assert!(surface.contains(row_id));
        assert_eq!(surface.take_update(), Update::Unchanged);

        let error = surface.mount(|tree| {
            tree.column(|tree| {
                for _ in 0..4 {
                    tree.text("x");
                }
            });
        });
        assert_eq!(error, Err(TreeError::UpdateTooLarge { count: 5, max: 4 }));
        assert_eq!(surface.roots(), [column]);
        assert!(surface.contains(text_id));
    }

    #[test]
    fn tree_stays_under_the_node_cap() {
        let limits = TreeLimits {
            max_nodes_per_update: 10,
            max_nodes: 5,
        };
        let (mut surface, column, text_id, _) = small_surface(limits);
        surface
            .insert(column, None, |tree| {
                tree.text("fifth");
            })
            .expect("at the cap");
        let before = snapshot(&surface);
        assert_eq!(
            surface
                .insert(column, None, |tree| {
                    tree.text("sixth");
                })
                .map(|_| ()),
            Err(TreeError::TreeTooLarge { count: 6, max: 5 })
        );
        assert_eq!(
            surface
                .rebuild(text_id, |tree| {
                    tree.text("a");
                    tree.text("b");
                })
                .map(|_| ()),
            Err(TreeError::TreeTooLarge { count: 6, max: 5 })
        );
        assert_eq!(snapshot(&surface), before);
        assert!(
            surface
                .rebuild(text_id, |tree| {
                    tree.text("replacement");
                })
                .is_ok()
        );
    }

    struct Echo {
        events: u32,
    }

    impl Extension for Echo {
        fn new() -> Self {
            Self { events: 0 }
        }

        fn surfaces(&self) -> Vec<Surface> {
            vec![Surface::panel(
                "echo",
                "Echo",
                DockPosition::Left,
                "echo",
                200,
            )]
        }

        fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
            let mut ids = IdAllocator::new();
            let mut tree = TreeBuilder::new(&mut ids);
            tree.text(surface_id);
            Ok(tree.finish())
        }

        fn handle_event(&mut self, _surface_id: &str, _event: UiEvent) -> Result<Update, String> {
            self.events += 1;
            let mut patches = PatchSet::new();
            patches.set_props(1, TextProps::new(self.events.to_string()));
            Ok(patches.into_update())
        }
    }

    #[test]
    fn dispatch_keeps_one_instance() {
        fn new_echo() -> Box<dyn Extension> {
            Box::new(Echo::new())
        }

        let surfaces = __private::with_extension(new_echo, |extension| extension.surfaces());
        assert_eq!(surfaces[0].id, "echo");
        for expected in ["1", "2"] {
            let update = __private::with_extension(new_echo, |extension| {
                extension.handle_event("echo", UiEvent::Click(1))
            });
            assert_eq!(
                update,
                Ok(Update::Patches(vec![Patch::SetProps((1, text(expected)))]))
            );
        }
        let rows =
            __private::with_extension(new_echo, |extension| extension.list_rows("echo", 1, 0, 10));
        assert!(rows.is_empty());
    }
}
