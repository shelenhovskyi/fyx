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
//!         }))
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

/// Builds a flat node list from nested code. Containers take a closure that builds their
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

    /// Adds a node and the children `build_children` adds, and returns the node's id.
    pub fn container(
        &mut self,
        props: impl Into<Props>,
        build_children: impl FnOnce(&mut Self),
    ) -> NodeId {
        let id = self.node(props);
        let outer_parent = self.parent.replace(id);
        build_children(self);
        self.parent = outer_parent;
        id
    }

    pub fn flex(&mut self, props: FlexProps, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.container(props, build_children)
    }

    pub fn column(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.container(FlexProps::column(), build_children)
    }

    pub fn row(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.container(FlexProps::row(), build_children)
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

/// Builds one row of a virtualized list. `build` must add exactly one top-level node, the row's
/// root; its ids start at [`ROW_NODE_ID_BASE`].
pub fn list_row(index: u32, build: impl FnOnce(&mut TreeBuilder)) -> ListRow {
    let mut ids = IdAllocator::starting_at(ROW_NODE_ID_BASE);
    let mut tree = TreeBuilder::new(&mut ids);
    build(&mut tree);
    ListRow {
        index,
        nodes: tree.finish(),
    }
}

/// The row indices of a `list-rows(start, count)` request that exist in a list of `row_count`
/// rows. The host may ask past the end while the list shrinks.
pub fn row_range(start: u32, count: u32, row_count: u32) -> Range<u32> {
    let end = start.saturating_add(count).min(row_count);
    start.min(end)..end
}

/// Answers a `list-rows(start, count)` request for a list of `row_count` rows by calling
/// `build_row` for each row that exists. See [`list_row`].
pub fn list_rows(
    start: u32,
    count: u32,
    row_count: u32,
    mut build_row: impl FnMut(u32, &mut TreeBuilder),
) -> Vec<ListRow> {
    row_range(start, count, row_count)
        .map(|index| list_row(index, |tree| build_row(index, tree)))
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
    NotAChild { parent: NodeId, child: NodeId },
    IsRoot(NodeId),
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
/// Edits are checked against the tree, recorded and queued as patches; [`SurfaceTree::take_update`]
/// hands the queue to the host. A `set_props` that does not change anything is dropped, so
/// re-setting every node after a state change sends only what actually changed.
#[derive(Debug, Default)]
pub struct SurfaceTree {
    ids: IdAllocator,
    mounted: bool,
    nodes: HashMap<NodeId, LiveNode>,
    roots: Vec<NodeId>,
    pending: PatchSet,
}

impl SurfaceTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the surface's tree from scratch and returns it for `mount`. Pending patches are
    /// dropped. Ids keep counting up from earlier trees, so a node id from a previous mount never
    /// names a node of the new tree.
    pub fn mount(&mut self, build: impl FnOnce(&mut TreeBuilder)) -> Vec<Node> {
        self.nodes.clear();
        self.roots.clear();
        self.pending = PatchSet::new();
        self.mounted = true;
        let mut tree = TreeBuilder::new(&mut self.ids);
        build(&mut tree);
        let nodes = tree.finish();
        self.record(&nodes, None);
        nodes
    }

    /// Like [`SurfaceTree::mount`], but returns the tree as a `replace` update, for when patching
    /// would be more work than rebuilding.
    pub fn replace(&mut self, build: impl FnOnce(&mut TreeBuilder)) -> Update {
        Update::Replace(self.mount(build))
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
        let node = self.node_mut(id)?;
        let props = props.into();
        if node.props == props {
            return Ok(false);
        }
        node.props = props.clone();
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
        self.node_mut(id)?.props = props.into();
        Ok(())
    }

    /// Builds a subtree under `parent`, before its child `before` or after its last child, and
    /// returns the ids of the subtree's top-level nodes.
    pub fn insert(
        &mut self,
        parent: NodeId,
        before: Option<NodeId>,
        build: impl FnOnce(&mut TreeBuilder),
    ) -> Result<Vec<NodeId>, TreeError> {
        let siblings = &self.node_mut(parent)?.children;
        let position = match before {
            Some(before) => siblings
                .iter()
                .position(|&sibling| sibling == before)
                .ok_or(TreeError::NotAChild {
                    parent,
                    child: before,
                })?,
            None => siblings.len(),
        };

        let mut tree = TreeBuilder::under(&mut self.ids, parent);
        build(&mut tree);
        let nodes = tree.finish();

        let top_level = nodes
            .iter()
            .filter(|node| node.parent == Some(parent))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        self.record(&nodes, Some((parent, position)));
        self.pending.insert_subtree(nodes, before);
        Ok(top_level)
    }

    /// Removes a node and its whole subtree.
    pub fn remove(&mut self, id: NodeId) -> Result<(), TreeError> {
        let parent = self.node_mut(id)?.parent;
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
        Ok(())
    }

    /// Replaces node `id` and its subtree with the nodes `build` adds, at the same position, and
    /// returns their ids. The new nodes get new ids, so the host treats them as new: a list
    /// rebuilt this way has all of its rows requested again, which `set_props` cannot force when
    /// the rows changed but the list's props did not.
    pub fn rebuild(
        &mut self,
        id: NodeId,
        build: impl FnOnce(&mut TreeBuilder),
    ) -> Result<Vec<NodeId>, TreeError> {
        let parent = self.node_mut(id)?.parent.ok_or(TreeError::IsRoot(id))?;
        let siblings = self.children(parent);
        let next_sibling = siblings
            .iter()
            .position(|&sibling| sibling == id)
            .and_then(|position| siblings.get(position + 1))
            .copied();
        self.remove(id)?;
        self.insert(parent, next_sibling, build)
    }

    /// Removes every child of `parent`.
    pub fn clear_children(&mut self, parent: NodeId) -> Result<(), TreeError> {
        let children = self.node_mut(parent)?.children.clone();
        for child in children {
            self.remove(child)?;
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

    fn node_mut(&mut self, id: NodeId) -> Result<&mut LiveNode, TreeError> {
        if !self.mounted {
            return Err(TreeError::NotMounted);
        }
        self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))
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

    /// Applies patches the way the `fyx:ui` contract describes, independently of [`SurfaceTree`],
    /// to check that the patches it emits reproduce its own view of the tree.
    fn apply(nodes: &mut Vec<Node>, update: Update) {
        let patches = match update {
            Update::Unchanged => return,
            Update::Replace(new_nodes) => {
                *nodes = new_nodes;
                return;
            }
            Update::Patches(patches) => patches,
        };
        for patch in patches {
            match patch {
                Patch::SetProps((id, props)) => {
                    let node = nodes.iter_mut().find(|node| node.id == id);
                    node.expect("set-props on an unknown node").props = props;
                }
                Patch::Remove(id) => {
                    let mut removed = HashSet::from([id]);
                    for node in nodes.iter() {
                        if node.parent.is_some_and(|parent| removed.contains(&parent)) {
                            removed.insert(node.id);
                        }
                    }
                    nodes.retain(|node| !removed.contains(&node.id));
                }
                Patch::Insert(InsertOp { node, before }) => {
                    let index = match before {
                        Some(before) => nodes
                            .iter()
                            .position(|sibling| sibling.id == before)
                            .expect("insert before an unknown node"),
                        None => subtree_end(nodes, node.parent),
                    };
                    nodes.insert(index, node);
                }
            }
        }
    }

    /// The index just past the last descendant of `parent` (or the end for roots).
    fn subtree_end(nodes: &[Node], parent: Option<NodeId>) -> usize {
        let Some(parent) = parent else {
            return nodes.len();
        };
        let start = nodes
            .iter()
            .position(|node| node.id == parent)
            .expect("insert under an unknown node");
        let mut inside = HashSet::from([parent]);
        let mut end = start + 1;
        while let Some(node) = nodes.get(end) {
            match node.parent {
                Some(node_parent) if inside.contains(&node_parent) => {
                    inside.insert(node.id);
                    end += 1;
                }
                _ => break,
            }
        }
        end
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
        surface.mount(|tree| {
            tree.column(|tree| {
                label = tree.text("0");
            });
        });

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
        surface.mount(|tree| {
            column = tree.column(|tree| {
                first = tree.text("a");
            });
        });
        assert_eq!(surface.remove(99), Err(TreeError::UnknownNode(99)));
        assert_eq!(
            surface.insert(first, Some(column), |tree| {
                tree.text("x");
            }),
            Err(TreeError::NotAChild {
                parent: first,
                child: column,
            })
        );
        let message: String = TreeError::UnknownNode(99).into();
        assert_eq!(message, "node 99 is not in the surface's tree");
    }

    #[test]
    fn surface_tree_patches_reproduce_its_tree() {
        let mut surface = SurfaceTree::new();
        let mut column = 0;
        let mut first = 0;
        let mut last = 0;
        let mut host_nodes = surface.mount(|tree| {
            column = tree.column(|tree| {
                first = tree.text("first");
                tree.row(|tree| {
                    tree.button("a");
                    tree.button("b");
                });
                last = tree.text("last");
            });
        });
        assert_eq!(host_nodes, surface.to_nodes());

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

        apply(&mut host_nodes, surface.take_update());
        assert_eq!(host_nodes, surface.to_nodes());

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
        apply(&mut host_nodes, surface.take_update());
        assert_eq!(host_nodes, surface.to_nodes());

        surface.clear_children(column).expect("clear");
        apply(&mut host_nodes, surface.take_update());
        assert_eq!(host_nodes, surface.to_nodes());
        assert_eq!(surface.len(), 1);
        assert_eq!(surface.roots(), [column]);
    }

    #[test]
    fn surface_tree_never_reuses_ids_across_mounts() {
        let mut surface = SurfaceTree::new();
        let first = surface.mount(|tree| {
            tree.text("a");
        });
        let Update::Replace(second) = surface.replace(|tree| {
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
    fn host_functions_work_off_wasm() {
        log(LogLevel::Info, "logged to stderr in tests");
        push_update("echo", &Update::Unchanged);
        assert_eq!(
            take_pushed_updates(),
            [("echo".to_string(), Update::Unchanged)]
        );
        assert!(take_pushed_updates().is_empty());
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
