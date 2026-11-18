//! The retained tree of one surface over the 0.2 model (design.md, G): the same rules as the 0.1
//! [`crate::Tree`], which the SDK keeps using until it moves to 0.2 (platform plan 2.4), plus
//! `invalidate-rows` and a [`Touched`] result that tells a props change from a row
//! invalidation.
//!
//! Every update is validated and applied atomically: a bad update leaves the previous tree as it
//! was. Patches apply in place with an undo log (perf-1 #6), and props are shared behind an
//! [`Arc`], so neither an update nor a clone copies text (perf-1 #12).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use super::{InsertOp, InvalidateRowsOp, Node, NodeId, Patch, Props, RowRange, Update};
use crate::canvas::MAX_CANVASES_PER_SURFACE;
use crate::tree::{TreeError, TreeLimits};

/// String bytes one surface's retained tree may hold, across all its props (triage-3 #7): each
/// call's lifted text is capped, but many calls could otherwise grow a tree without bound. An
/// update that would pass it is refused with [`TreeError::TreeTextTooLarge`] and changes nothing.
pub const MAX_TREE_TEXT_BYTES: usize = 8 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    props: Arc<Props>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

impl TreeNode {
    pub fn props(&self) -> &Props {
        &self.props
    }

    /// The shared props. Every `set-props`, insert and replace makes a new `Arc`, so
    /// `Arc::ptr_eq` between two revisions is true exactly when the node's props were not
    /// replaced in between: a renderer can keep these and compare by pointer.
    pub fn props_arc(&self) -> &Arc<Props> {
        &self.props
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn children(&self) -> &[NodeId] {
        &self.children
    }
}

/// Which rows of a list or table must be fetched again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowInvalidation {
    All,
    /// Clamped to the row count; several ranges of one update merge into their hull.
    Range(RowRange),
}

impl RowInvalidation {
    fn merge(self, other: RowInvalidation) -> RowInvalidation {
        match (self, other) {
            (RowInvalidation::Range(a), RowInvalidation::Range(b)) => {
                let start = a.start.min(b.start);
                let end = (a.start + a.count).max(b.start + b.count);
                RowInvalidation::Range(RowRange {
                    start,
                    count: end - start,
                })
            }
            _ => RowInvalidation::All,
        }
    }
}

/// What an update changed: the nodes it touched (`set-props` targets, inserted ids, every removed
/// id; for `replace`, the old and the new ids) and, per list or table node, the rows to fetch
/// again under the 0.2 rule (`invalidate-rows`, a change to `row-count`, `row-height` or
/// `columns`, an insert or a replace). 0.1 components refetch every touched list instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Touched {
    pub nodes: BTreeSet<NodeId>,
    pub rows: BTreeMap<NodeId, RowInvalidation>,
}

impl Touched {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.rows.is_empty()
    }

    fn invalidate(&mut self, id: NodeId, rows: RowInvalidation) {
        let merged = match self.rows.get(&id) {
            Some(existing) => existing.merge(rows),
            None => rows,
        };
        self.rows.insert(id, merged);
    }
}

fn is_rows_node(props: &Props) -> bool {
    matches!(props, Props::List(_) | Props::Table(_))
}

fn row_count(props: &Props) -> u32 {
    match props {
        Props::List(list) => list.row_count,
        Props::Table(table) => table.row_count,
        _ => 0,
    }
}

/// Whether going from `old` to `new` changes which rows exist or how they are drawn: the kind,
/// `row-count`, `row-height` (lists) or `columns` (tables). `selected` alone does not.
fn rows_changed(old: &Props, new: &Props) -> bool {
    match (old, new) {
        (Props::List(old), Props::List(new)) => {
            old.row_count != new.row_count || old.row_height != new.row_height
        }
        (Props::Table(old), Props::Table(new)) => {
            old.row_count != new.row_count || old.columns != new.columns
        }
        (old, new) => is_rows_node(old) || is_rows_node(new),
    }
}

/// How to take back one applied patch, for [`Tree::apply_patches`].
enum Undo {
    /// `set-props`: the props it replaced.
    Props(NodeId, Arc<Props>),
    /// `insert`: the node it added (with no children yet when undone, since later patches are
    /// undone first).
    Inserted(NodeId),
    /// `remove`: the removed subtree, and where its root was among its siblings.
    Removed {
        id: NodeId,
        parent: Option<NodeId>,
        index: usize,
        nodes: Vec<(NodeId, TreeNode)>,
    },
}

/// A forest: nodes with `parent: None` are roots, kept in insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    nodes: HashMap<NodeId, TreeNode>,
    roots: Vec<NodeId>,
    limits: TreeLimits,
    /// Canvas nodes in the tree, at most [`MAX_CANVASES_PER_SURFACE`] (design.md §13).
    canvases: usize,
    /// [`Props::text_len`] of every node, at most [`MAX_TREE_TEXT_BYTES`].
    text_bytes: usize,
}

fn is_canvas(props: &Props) -> bool {
    matches!(props, Props::Canvas(_))
}

impl Tree {
    pub fn new(limits: TreeLimits) -> Self {
        Self {
            nodes: HashMap::default(),
            roots: Vec::new(),
            limits,
            canvases: 0,
            text_bytes: 0,
        }
    }

    /// The string bytes the tree retains (for the Extension Inspector).
    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    fn check_text(&self, removed: usize, added: usize) -> Result<(), TreeError> {
        if self.text_bytes - removed + added > MAX_TREE_TEXT_BYTES {
            return Err(TreeError::TreeTextTooLarge {
                max: MAX_TREE_TEXT_BYTES,
            });
        }
        Ok(())
    }

    pub fn from_nodes(nodes: Vec<Node>, limits: TreeLimits) -> Result<Self, TreeError> {
        let mut tree = Self::new(limits);
        tree.replace(nodes)?;
        Ok(tree)
    }

    /// Builds the subtree of one virtualized list row; it must have exactly one root.
    pub fn from_row(nodes: Vec<Node>, limits: TreeLimits) -> Result<Self, TreeError> {
        let mut tree = Self::new(limits);
        tree.check_update_size(nodes.len())?;
        if let Some(canvas) = nodes.iter().find(|node| is_canvas(&node.props)) {
            return Err(TreeError::CanvasInRow(canvas.id));
        }
        tree.replace(nodes)?;
        if tree.roots.len() != 1 {
            return Err(TreeError::RowRootCount(tree.roots.len()));
        }
        Ok(tree)
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn get(&self, id: NodeId) -> Option<&TreeNode> {
        self.nodes.get(&id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Children of `parent`, or the roots for `None`. Unknown ids have no children.
    pub fn children(&self, parent: Option<NodeId>) -> &[NodeId] {
        match parent {
            Some(id) => self.nodes.get(&id).map_or(&[], |node| &node.children),
            None => &self.roots,
        }
    }

    /// The tree as a flat list in pre-order, which satisfies "parents precede children".
    pub fn to_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<NodeId> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            nodes.push(Node {
                id,
                parent: node.parent,
                props: Props::clone(&node.props),
            });
            stack.extend(node.children.iter().rev().copied());
        }
        nodes
    }

    /// Applies an update atomically; see [`Touched`].
    pub fn apply_update(&mut self, update: Update) -> Result<Touched, TreeError> {
        match update {
            Update::Unchanged => Ok(Touched::default()),
            Update::Replace(nodes) => self.replace(nodes),
            Update::Patches(patches) => self.apply_patches(patches),
        }
    }

    /// Replaces the whole tree atomically: every old and new id is touched, every list and table
    /// of the new tree is fetched again. At most `max_nodes` nodes, as a mount.
    pub fn replace(&mut self, nodes: Vec<Node>) -> Result<Touched, TreeError> {
        if nodes.len() > self.limits.max_nodes {
            return Err(TreeError::TreeTooLarge {
                max: self.limits.max_nodes,
            });
        }
        let mut next = Self::new(self.limits);
        for node in nodes {
            next.insert(InsertOp { node, before: None })?;
        }
        let mut touched = Touched {
            nodes: self
                .nodes
                .keys()
                .chain(next.nodes.keys())
                .copied()
                .collect(),
            rows: BTreeMap::new(),
        };
        for (id, node) in &next.nodes {
            if is_rows_node(&node.props) {
                touched.invalidate(*id, RowInvalidation::All);
            }
        }
        *self = next;
        Ok(touched)
    }

    /// Applies patches in order, atomically: on error the tree is left as it was. They apply in
    /// place; each applied patch logs its inverse, and an error replays the log backwards. A
    /// failing patch itself changes nothing, as every check comes before its first change.
    pub fn apply_patches(&mut self, patches: Vec<Patch>) -> Result<Touched, TreeError> {
        self.check_update_size(patches.len())?;
        let mut touched = Touched::default();
        let mut undo = Vec::new();
        for (index, patch) in patches.into_iter().enumerate() {
            if let Err(error) = self.apply_patch(patch, &mut touched, &mut undo) {
                self.undo(undo);
                return Err(TreeError::Patch {
                    index,
                    error: Box::new(error),
                });
            }
        }
        // Rows of lists removed later in the same update need no fetch.
        touched.rows.retain(|id, _| self.nodes.contains_key(id));
        Ok(touched)
    }

    fn undo(&mut self, log: Vec<Undo>) {
        for entry in log.into_iter().rev() {
            match entry {
                Undo::Props(id, props) => {
                    if let Some(node) = self.nodes.get_mut(&id) {
                        let replaced = std::mem::replace(&mut node.props, props);
                        self.canvases = self.canvases + usize::from(is_canvas(&node.props))
                            - usize::from(is_canvas(&replaced));
                        self.text_bytes =
                            self.text_bytes + node.props.text_len() - replaced.text_len();
                    }
                }
                Undo::Inserted(id) => {
                    if let Some(node) = self.nodes.remove(&id) {
                        self.canvases -= usize::from(is_canvas(&node.props));
                        self.text_bytes -= node.props.text_len();
                        self.siblings_mut(node.parent)
                            .retain(|&sibling| sibling != id);
                    }
                }
                Undo::Removed {
                    id,
                    parent,
                    index,
                    nodes,
                } => {
                    self.canvases += nodes
                        .iter()
                        .filter(|(_, node)| is_canvas(&node.props))
                        .count();
                    self.text_bytes += nodes
                        .iter()
                        .map(|(_, node)| node.props.text_len())
                        .sum::<usize>();
                    self.nodes.extend(nodes);
                    let siblings = self.siblings_mut(parent);
                    let index = index.min(siblings.len());
                    siblings.insert(index, id);
                }
            }
        }
    }

    /// The children of `parent`, or the roots. A missing parent gives the roots' list only in
    /// [`Tree::undo`], where every parent exists again by the time its children are restored.
    fn siblings_mut(&mut self, parent: Option<NodeId>) -> &mut Vec<NodeId> {
        match parent.and_then(|parent| self.nodes.get_mut(&parent)) {
            Some(parent) => &mut parent.children,
            None => &mut self.roots,
        }
    }

    fn check_update_size(&self, count: usize) -> Result<(), TreeError> {
        if count > self.limits.max_nodes_per_update {
            return Err(TreeError::UpdateTooLarge {
                count,
                max: self.limits.max_nodes_per_update,
            });
        }
        Ok(())
    }

    fn apply_patch(
        &mut self,
        patch: Patch,
        touched: &mut Touched,
        undo: &mut Vec<Undo>,
    ) -> Result<(), TreeError> {
        match patch {
            Patch::SetProps(id, props) => {
                let old = self.set_props(id, props)?;
                touched.nodes.insert(id);
                let new = &self.nodes[&id].props;
                if is_rows_node(new) && rows_changed(&old, new) {
                    touched.invalidate(id, RowInvalidation::All);
                }
                undo.push(Undo::Props(id, old));
            }
            Patch::Insert(op) => {
                let id = op.node.id;
                let rows = is_rows_node(&op.node.props);
                self.insert(op)?;
                touched.nodes.insert(id);
                if rows {
                    touched.invalidate(id, RowInvalidation::All);
                }
                undo.push(Undo::Inserted(id));
            }
            Patch::Remove(id) => {
                let (parent, index, nodes) = self.remove(id, &mut touched.nodes)?;
                undo.push(Undo::Removed {
                    id,
                    parent,
                    index,
                    nodes,
                });
            }
            Patch::InvalidateRows(InvalidateRowsOp { target, rows }) => {
                let node = self
                    .nodes
                    .get(&target)
                    .ok_or(TreeError::UnknownNode(target))?;
                if !is_rows_node(&node.props) {
                    return Err(TreeError::NotAList {
                        node: target,
                        kind: node.props.kind_name(),
                    });
                }
                let count = row_count(&node.props);
                let rows = match rows {
                    None => RowInvalidation::All,
                    Some(range) => {
                        let start = range.start.min(count);
                        let end = range.start.saturating_add(range.count).min(count);
                        RowInvalidation::Range(RowRange {
                            start,
                            count: end - start,
                        })
                    }
                };
                touched.nodes.insert(target);
                touched.invalidate(target, rows);
            }
        }
        Ok(())
    }

    /// Returns the props it replaced.
    fn set_props(&mut self, id: NodeId, props: Props) -> Result<Arc<Props>, TreeError> {
        let node = self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))?;
        if !node.children.is_empty() && !props.is_container() {
            return Err(TreeError::NotAContainer {
                parent: id,
                kind: props.kind_name(),
            });
        }
        let adds_canvas = is_canvas(&props) && !is_canvas(&node.props);
        if adds_canvas && self.canvases >= MAX_CANVASES_PER_SURFACE {
            return Err(TreeError::TooManyCanvases {
                max: MAX_CANVASES_PER_SURFACE,
            });
        }
        let (removed, added) = (node.props.text_len(), props.text_len());
        self.check_text(removed, added)?;
        let node = self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))?;
        let old = std::mem::replace(&mut node.props, Arc::new(props));
        self.canvases =
            self.canvases + usize::from(is_canvas(&node.props)) - usize::from(is_canvas(&old));
        self.text_bytes = self.text_bytes - removed + added;
        Ok(old)
    }

    /// Levels from `id` up to its root, inclusive (a root is 1). The walk stops at
    /// `max_depth + 1`, so it is cheap however the tree was built.
    fn depth_of(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut current = Some(id);
        while let Some(id) = current {
            let Some(node) = self.nodes.get(&id) else {
                break;
            };
            depth += 1;
            if depth > self.limits.max_depth {
                break;
            }
            current = node.parent;
        }
        depth
    }

    fn insert(&mut self, InsertOp { node, before }: InsertOp) -> Result<(), TreeError> {
        if self.nodes.contains_key(&node.id) {
            return Err(TreeError::DuplicateId(node.id));
        }
        if self.nodes.len() >= self.limits.max_nodes {
            return Err(TreeError::TreeTooLarge {
                max: self.limits.max_nodes,
            });
        }
        if is_canvas(&node.props) && self.canvases >= MAX_CANVASES_PER_SURFACE {
            return Err(TreeError::TooManyCanvases {
                max: MAX_CANVASES_PER_SURFACE,
            });
        }
        if let Some(parent_id) = node.parent
            && self.depth_of(parent_id) >= self.limits.max_depth
        {
            return Err(TreeError::TooDeep {
                node: node.id,
                max: self.limits.max_depth,
            });
        }
        let text = node.props.text_len();
        self.check_text(0, text)?;
        let siblings = match node.parent {
            Some(parent_id) => {
                let parent = self
                    .nodes
                    .get_mut(&parent_id)
                    .ok_or(TreeError::UnknownParent {
                        node: node.id,
                        parent: parent_id,
                    })?;
                if !parent.props.is_container() {
                    return Err(TreeError::NotAContainer {
                        parent: parent_id,
                        kind: parent.props.kind_name(),
                    });
                }
                &mut parent.children
            }
            None => &mut self.roots,
        };
        let index = match before {
            None => siblings.len(),
            Some(before) => siblings
                .iter()
                .position(|&sibling| sibling == before)
                .ok_or(TreeError::UnknownSibling {
                    node: node.id,
                    before,
                })?,
        };
        siblings.insert(index, node.id);
        self.canvases += usize::from(is_canvas(&node.props));
        self.text_bytes += text;
        self.nodes.insert(
            node.id,
            TreeNode {
                props: Arc::new(node.props),
                parent: node.parent,
                children: Vec::new(),
            },
        );
        Ok(())
    }

    /// Removes `id` and its subtree. Returns, for the undo log, its parent, its index among its
    /// siblings and every removed node.
    #[allow(clippy::type_complexity)]
    fn remove(
        &mut self,
        id: NodeId,
        removed: &mut BTreeSet<NodeId>,
    ) -> Result<(Option<NodeId>, usize, Vec<(NodeId, TreeNode)>), TreeError> {
        let node = self.nodes.remove(&id).ok_or(TreeError::UnknownNode(id))?;
        removed.insert(id);
        let parent = node.parent;
        let siblings = self.siblings_mut(parent);
        let index = siblings
            .iter()
            .position(|&sibling| sibling == id)
            .unwrap_or(siblings.len());
        siblings.retain(|&sibling| sibling != id);
        let mut pending = node.children.clone();
        self.canvases -= usize::from(is_canvas(&node.props));
        self.text_bytes -= node.props.text_len();
        let mut nodes = vec![(id, node)];
        while let Some(child) = pending.pop() {
            if let Some(node) = self.nodes.remove(&child) {
                removed.insert(child);
                self.canvases -= usize::from(is_canvas(&node.props));
                self.text_bytes -= node.props.text_len();
                pending.extend(node.children.iter().copied());
                nodes.push((child, node));
            }
        }
        Ok((parent, index, nodes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v0_2::{FlexProps, ListProps, TableProps, TextProps};

    fn node(id: NodeId, parent: Option<NodeId>, props: Props) -> Node {
        Node { id, parent, props }
    }

    fn flex(id: NodeId, parent: Option<NodeId>) -> Node {
        node(id, parent, Props::Flex(FlexProps::default()))
    }

    fn list(row_count: u32, row_height: u16, selected: Option<u32>) -> Props {
        Props::List(ListProps {
            row_count,
            row_height,
            selected,
        })
    }

    fn text(content: &str) -> Props {
        Props::Text(TextProps {
            content: content.into(),
            ..TextProps::default()
        })
    }

    fn sample() -> Tree {
        Tree::from_nodes(
            vec![
                flex(1, None),
                node(2, Some(1), list(10, 20, None)),
                node(3, Some(1), text("a")),
                node(
                    4,
                    Some(1),
                    Props::Table(TableProps {
                        columns: vec!["a".into()],
                        row_count: 5,
                    }),
                ),
            ],
            TreeLimits::default(),
        )
        .expect("valid tree")
    }

    fn invalidate(target: NodeId, rows: Option<(u32, u32)>) -> Patch {
        Patch::InvalidateRows(InvalidateRowsOp {
            target,
            rows: rows.map(|(start, count)| RowRange { start, count }),
        })
    }

    #[test]
    fn invalidate_rows_ranges_clamp_and_merge() {
        let mut tree = sample();
        let touched = tree
            .apply_patches(vec![
                invalidate(2, Some((2, 3))),
                invalidate(2, Some((8, 5))),
            ])
            .unwrap();
        assert_eq!(
            touched.rows[&2],
            RowInvalidation::Range(RowRange { start: 2, count: 8 }),
            "the hull of 2..5 and 8..10 (clamped from 8..13)"
        );
        assert!(touched.nodes.contains(&2));
        let touched = tree
            .apply_patches(vec![invalidate(2, Some((1, 1))), invalidate(2, None)])
            .unwrap();
        assert_eq!(touched.rows[&2], RowInvalidation::All);
        let touched = tree.apply_patches(vec![invalidate(4, None)]).unwrap();
        assert_eq!(touched.rows[&4], RowInvalidation::All, "tables too");
    }

    #[test]
    fn invalidate_rows_needs_a_list() {
        let mut tree = sample();
        let before = tree.clone();
        let error = tree
            .apply_patches(vec![invalidate(2, None), invalidate(3, None)])
            .unwrap_err();
        assert_eq!(
            error,
            TreeError::Patch {
                index: 1,
                error: Box::new(TreeError::NotAList {
                    node: 3,
                    kind: "text"
                })
            }
        );
        assert_eq!(tree, before, "the whole update is rejected");
        assert_eq!(
            tree.apply_patches(vec![invalidate(99, None)]).unwrap_err(),
            TreeError::Patch {
                index: 0,
                error: Box::new(TreeError::UnknownNode(99))
            }
        );
    }

    fn text_props(bytes: usize) -> Props {
        Props::Text(TextProps {
            content: "x".repeat(bytes),
            ..TextProps::default()
        })
    }

    /// triage-3 #7: a surface's tree holds at most `MAX_TREE_TEXT_BYTES` of text, however many
    /// updates build it; a refused update changes nothing, and removals and undo keep the count.
    #[test]
    fn retained_text_is_capped_per_surface() {
        let half = MAX_TREE_TEXT_BYTES / 2;
        let mut tree = Tree::from_nodes(vec![flex(1, None)], TreeLimits::default()).unwrap();
        tree.apply_patches(vec![Patch::Insert(InsertOp {
            node: node(2, Some(1), text_props(half)),
            before: None,
        })])
        .unwrap();
        assert_eq!(tree.text_bytes(), half);
        let before = tree.clone();
        // Each update is small enough on its own; the third would pass the cap.
        tree.apply_patches(vec![Patch::Insert(InsertOp {
            node: node(3, Some(1), text_props(half)),
            before: None,
        })])
        .unwrap();
        assert_eq!(tree.text_bytes(), MAX_TREE_TEXT_BYTES);
        let full = tree.clone();
        let error = tree
            .apply_patches(vec![
                Patch::SetProps(2, text_props(1)),
                Patch::Insert(InsertOp {
                    node: node(4, Some(1), text_props(half)),
                    before: None,
                }),
            ])
            .unwrap_err();
        assert!(
            matches!(&error, TreeError::Patch { index: 1, error } if matches!(**error, TreeError::TreeTextTooLarge { .. })),
            "{error:?}"
        );
        assert_eq!(tree, full, "the refused update undid its first patch");
        assert_eq!(tree.text_bytes(), MAX_TREE_TEXT_BYTES);
        // Growing a node past the cap is refused; shrinking one makes room.
        assert!(
            tree.apply_patches(vec![Patch::SetProps(2, text_props(half + 1))])
                .is_err()
        );
        tree.apply_patches(vec![Patch::SetProps(2, text_props(10))])
            .unwrap();
        assert_eq!(tree.text_bytes(), half + 10);
        // Removing a subtree frees its text; undoing the removal counts it again.
        tree.apply_patches(vec![Patch::Remove(1)]).unwrap();
        assert_eq!(tree.text_bytes(), 0);
        let mut tree = before;
        assert!(
            tree.apply_patches(vec![Patch::Remove(1), Patch::SetProps(9, text_props(1))])
                .is_err()
        );
        assert_eq!(tree.text_bytes(), half);
        // A replace counts its own nodes.
        assert!(matches!(
            tree.replace(vec![
                node(1, None, text_props(half)),
                node(2, None, text_props(half + 1)),
            ]),
            Err(TreeError::TreeTextTooLarge { .. })
        ));
        assert_eq!(
            tree.text_bytes(),
            half,
            "a refused replace keeps the old tree"
        );
    }

    /// perf-1 #6: patches apply in place, and a failing one undoes every earlier one, sibling
    /// order and removed subtrees included.
    #[test]
    fn a_failing_update_undoes_everything_in_place() {
        let mut tree = sample();
        tree.apply_patches(vec![
            Patch::Insert(InsertOp {
                node: flex(5, Some(1)),
                before: Some(3),
            }),
            Patch::Insert(InsertOp {
                node: node(6, Some(5), text("x")),
                before: None,
            }),
        ])
        .expect("valid");
        let before = tree.clone();
        let error = tree
            .apply_patches(vec![
                Patch::SetProps(3, text("b")),
                Patch::Remove(5),
                Patch::Insert(InsertOp {
                    node: node(7, Some(1), text("new")),
                    before: Some(2),
                }),
                Patch::Remove(4),
                Patch::Insert(InsertOp {
                    node: flex(8, None),
                    before: None,
                }),
                Patch::SetProps(2, list(1, 20, None)),
                Patch::Remove(99),
            ])
            .unwrap_err();
        assert!(matches!(error, TreeError::Patch { index: 6, .. }));
        assert_eq!(tree, before);
        assert_eq!(tree.get(1).expect("root").children(), [2, 5, 3, 4]);
        assert_eq!(tree.get(5).expect("restored").children(), [6]);
        assert_eq!(tree.roots(), [1]);
    }

    fn canvas() -> Props {
        Props::Canvas(crate::canvas::CanvasProps {
            width: 10,
            height: 10,
            description: "A chart".into(),
            ops: Vec::new(),
            clickable: false,
            fit: crate::canvas::CanvasFit::Clip,
        })
    }

    /// Design.md §13: at most 16 canvases per surface, however they arrive, and none in a row.
    #[test]
    fn canvases_per_surface_and_rows() {
        let max = MAX_CANVASES_PER_SURFACE as NodeId;
        let mut nodes = vec![flex(1, None)];
        nodes.extend((0..max).map(|i| node(10 + i, Some(1), canvas())));
        let mut tree = Tree::from_nodes(nodes.clone(), TreeLimits::default()).expect("16 fit");
        let too_many = TreeError::TooManyCanvases {
            max: MAX_CANVASES_PER_SURFACE,
        };
        let mut seventeen = nodes.clone();
        seventeen.push(node(99, Some(1), canvas()));
        assert_eq!(
            Tree::from_nodes(seventeen, TreeLimits::default()).unwrap_err(),
            too_many
        );
        let insert = |id| {
            Patch::Insert(InsertOp {
                node: node(id, Some(1), canvas()),
                before: None,
            })
        };
        let before = tree.clone();
        assert!(matches!(
            tree.apply_patches(vec![insert(99)]).unwrap_err(),
            TreeError::Patch { error, .. } if *error == too_many
        ));
        tree.apply_patches(vec![Patch::Insert(InsertOp {
            node: node(2, Some(1), text("a")),
            before: None,
        })])
        .expect("not a canvas");
        assert!(matches!(
            tree.apply_patches(vec![Patch::SetProps(2, canvas())]).unwrap_err(),
            TreeError::Patch { error, .. } if *error == too_many
        ));
        // Replacing a canvas by a canvas, or removing one first, frees no slot by accident and
        // uses a freed one.
        tree.apply_patches(vec![Patch::SetProps(10, canvas())])
            .expect("still 16");
        tree.apply_patches(vec![Patch::Remove(10), Patch::SetProps(2, canvas())])
            .expect("a freed slot");
        // A rejected update restores the count with the tree.
        tree.apply_patches(vec![
            Patch::Remove(11),
            Patch::Remove(12),
            insert(98),
            Patch::Remove(99),
        ])
        .unwrap_err();
        assert!(matches!(
            tree.apply_patches(vec![insert(97)]).unwrap_err(),
            TreeError::Patch { error, .. } if *error == too_many
        ));
        assert_ne!(tree, before);

        assert_eq!(
            Tree::from_row(
                vec![flex(1, None), node(2, Some(1), canvas())],
                TreeLimits::default()
            )
            .unwrap_err(),
            TreeError::CanvasInRow(2)
        );
    }

    /// perf-1 #12: a node's props `Arc` changes exactly when its props are replaced.
    #[test]
    fn props_are_shared_until_replaced() {
        let mut tree = sample();
        let text_props = tree.get(3).expect("text").props_arc().clone();
        let list_props = tree.get(2).expect("list").props_arc().clone();
        let copy = tree.clone();
        assert!(Arc::ptr_eq(
            copy.get(3).expect("text").props_arc(),
            &text_props
        ));
        tree.apply_patches(vec![Patch::SetProps(3, text("a"))])
            .expect("valid");
        assert!(!Arc::ptr_eq(
            tree.get(3).expect("text").props_arc(),
            &text_props
        ));
        assert_eq!(**tree.get(3).expect("text").props_arc(), *text_props);
        assert!(Arc::ptr_eq(
            tree.get(2).expect("list").props_arc(),
            &list_props
        ));
        // A rolled-back update gives back the very same props.
        tree.apply_patches(vec![
            Patch::SetProps(2, list(1, 20, None)),
            Patch::Remove(99),
        ])
        .unwrap_err();
        assert!(Arc::ptr_eq(
            tree.get(2).expect("list").props_arc(),
            &list_props
        ));
    }

    /// Design G's refetch rule: `selected` alone fetches nothing; `row-count`, `row-height`,
    /// `columns`, an insert and a replace fetch every row.
    #[test]
    fn refetch_rule() {
        let mut tree = sample();
        let touched = tree
            .apply_patches(vec![Patch::SetProps(2, list(10, 20, Some(3)))])
            .unwrap();
        assert!(touched.nodes.contains(&2));
        assert!(touched.rows.is_empty(), "selection only");
        for props in [list(11, 20, Some(3)), list(11, 24, Some(3))] {
            let touched = tree.apply_patches(vec![Patch::SetProps(2, props)]).unwrap();
            assert_eq!(touched.rows[&2], RowInvalidation::All);
        }
        let touched = tree
            .apply_patches(vec![Patch::SetProps(
                4,
                Props::Table(TableProps {
                    columns: vec!["a".into(), "b".into()],
                    row_count: 5,
                }),
            )])
            .unwrap();
        assert_eq!(touched.rows[&4], RowInvalidation::All);
        let touched = tree
            .apply_patches(vec![Patch::Insert(InsertOp {
                node: node(5, Some(1), list(3, 20, None)),
                before: None,
            })])
            .unwrap();
        assert_eq!(touched.rows[&5], RowInvalidation::All);
        let touched = tree.replace(sample().to_nodes()).unwrap();
        assert_eq!(touched.rows.keys().copied().collect::<Vec<_>>(), [2, 4]);
        // A list removed in the same update needs nothing.
        let touched = tree
            .apply_patches(vec![invalidate(2, None), Patch::Remove(2)])
            .unwrap();
        assert!(touched.rows.is_empty());
        assert!(touched.nodes.contains(&2));
    }

    #[test]
    fn caps_and_validation_match_0_1() {
        let limits = TreeLimits {
            max_nodes_per_update: 1,
            max_nodes: 3,
            max_depth: 2,
        };
        assert_eq!(
            Tree::from_nodes(
                vec![flex(1, None), flex(2, Some(1)), flex(3, Some(2))],
                limits
            )
            .unwrap_err(),
            TreeError::TooDeep { node: 3, max: 2 }
        );
        let mut tree = Tree::from_nodes(vec![flex(1, None), flex(2, Some(1))], limits).unwrap();
        assert!(matches!(
            tree.apply_patches(vec![Patch::Remove(2), Patch::Remove(1)]),
            Err(TreeError::UpdateTooLarge { count: 2, max: 1 })
        ));
        assert!(matches!(
            Tree::from_row(vec![flex(1, None), flex(2, None)], TreeLimits::default()),
            Err(TreeError::RowRootCount(2))
        ));
        assert_eq!(tree.to_nodes().len(), 2);
    }
}
