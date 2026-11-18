//! The retained tree of one surface over the 0.2 model (design.md, G): the same rules as the 0.1
//! [`crate::Tree`], which the SDK keeps using until it moves to 0.2 (platform plan 2.4), plus
//! `invalidate-rows` and a [`Touched`] result that tells a props change from a row
//! invalidation.
//!
//! Every update is validated and applied atomically: a bad update leaves the previous tree as it
//! was.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{InsertOp, InvalidateRowsOp, Node, NodeId, Patch, Props, RowRange, Update};
use crate::tree::{TreeError, TreeLimits};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    props: Props,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

impl TreeNode {
    pub fn props(&self) -> &Props {
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

/// A forest: nodes with `parent: None` are roots, kept in insertion order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    nodes: HashMap<NodeId, TreeNode>,
    roots: Vec<NodeId>,
    limits: TreeLimits,
}

impl Tree {
    pub fn new(limits: TreeLimits) -> Self {
        Self {
            nodes: HashMap::default(),
            roots: Vec::new(),
            limits,
        }
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
                props: node.props.clone(),
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

    /// Applies patches in order, atomically: on error the tree is left as it was.
    pub fn apply_patches(&mut self, patches: Vec<Patch>) -> Result<Touched, TreeError> {
        self.check_update_size(patches.len())?;
        let mut touched = Touched::default();
        if patches.is_empty() {
            return Ok(touched);
        }
        // Patching a copy keeps rollback trivial; updates are per input event, not per frame.
        let mut next = self.clone();
        for (index, patch) in patches.into_iter().enumerate() {
            next.apply_patch(patch, &mut touched)
                .map_err(|error| TreeError::Patch {
                    index,
                    error: Box::new(error),
                })?;
        }
        // Rows of lists removed later in the same update need no fetch.
        touched.rows.retain(|id, _| next.nodes.contains_key(id));
        *self = next;
        Ok(touched)
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

    fn apply_patch(&mut self, patch: Patch, touched: &mut Touched) -> Result<(), TreeError> {
        match patch {
            Patch::SetProps(id, props) => {
                let old = self.set_props(id, props)?;
                touched.nodes.insert(id);
                let new = &self.nodes[&id].props;
                if is_rows_node(new) && rows_changed(&old, new) {
                    touched.invalidate(id, RowInvalidation::All);
                }
            }
            Patch::Insert(op) => {
                let id = op.node.id;
                let rows = is_rows_node(&op.node.props);
                self.insert(op)?;
                touched.nodes.insert(id);
                if rows {
                    touched.invalidate(id, RowInvalidation::All);
                }
            }
            Patch::Remove(id) => self.remove(id, &mut touched.nodes)?,
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
    fn set_props(&mut self, id: NodeId, props: Props) -> Result<Props, TreeError> {
        let node = self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))?;
        if !node.children.is_empty() && !props.is_container() {
            return Err(TreeError::NotAContainer {
                parent: id,
                kind: props.kind_name(),
            });
        }
        Ok(std::mem::replace(&mut node.props, props))
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
        if let Some(parent_id) = node.parent
            && self.depth_of(parent_id) >= self.limits.max_depth
        {
            return Err(TreeError::TooDeep {
                node: node.id,
                max: self.limits.max_depth,
            });
        }
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
        self.nodes.insert(
            node.id,
            TreeNode {
                props: node.props,
                parent: node.parent,
                children: Vec::new(),
            },
        );
        Ok(())
    }

    fn remove(&mut self, id: NodeId, removed: &mut BTreeSet<NodeId>) -> Result<(), TreeError> {
        let node = self.nodes.remove(&id).ok_or(TreeError::UnknownNode(id))?;
        removed.insert(id);
        let siblings = match node.parent {
            Some(parent_id) => self
                .nodes
                .get_mut(&parent_id)
                .map(|parent| &mut parent.children),
            None => Some(&mut self.roots),
        };
        if let Some(siblings) = siblings {
            siblings.retain(|&sibling| sibling != id);
        }
        let mut pending = node.children;
        while let Some(child) = pending.pop() {
            if let Some(node) = self.nodes.remove(&child) {
                removed.insert(child);
                pending.extend(node.children);
            }
        }
        Ok(())
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
            ..TreeLimits::default()
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
