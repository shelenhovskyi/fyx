//! The host-side retained tree of one surface: built from a flat `list<node>`, then changed by
//! patches. Every update is validated and applied atomically, so a bad update from an extension
//! leaves the previous tree intact.

use std::collections::{BTreeSet, HashMap};

use crate::types::{InsertOp, Node, NodeId, Patch, Props, Update};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeLimits {
    /// Patches in a `patches` update, or nodes in one list row.
    pub max_nodes_per_update: usize,
    /// Nodes in the whole tree, so also in a `mount` or `replace`: any tree that can be built up
    /// through patches can also be mounted at once.
    pub max_nodes: usize,
}

impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            max_nodes_per_update: 2048,
            max_nodes: 16384,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    #[error("update carries {count} nodes or patches, the cap is {max}")]
    UpdateTooLarge { count: usize, max: usize },
    #[error("tree would exceed {max} nodes")]
    TreeTooLarge { max: usize },
    #[error("duplicate node id {0}")]
    DuplicateId(NodeId),
    #[error("node {node} names parent {parent}, which is not an earlier node of the tree")]
    UnknownParent { node: NodeId, parent: NodeId },
    #[error("node {parent} is a {kind} node and cannot have children")]
    NotAContainer { parent: NodeId, kind: &'static str },
    #[error("unknown node id {0}")]
    UnknownNode(NodeId),
    #[error(
        "node {node} cannot be inserted before {before}, which is not a sibling under the same parent"
    )]
    UnknownSibling { node: NodeId, before: NodeId },
    #[error("a list row must have exactly one root node, found {0}")]
    RowRootCount(usize),
    #[error("patch {index}: {error}")]
    Patch { index: usize, error: Box<TreeError> },
}

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

    /// Applies an update atomically and returns the ids it touched: `set-props` targets, inserted
    /// ids and every removed id (whole subtrees); for `replace`, the old and the new ids. Empty means
    /// nothing changed. A touched id that is still a list node needs its rows requested again.
    pub fn apply_update(&mut self, update: Update) -> Result<BTreeSet<NodeId>, TreeError> {
        match update {
            Update::Unchanged => Ok(BTreeSet::new()),
            Update::Replace(nodes) => self.replace(nodes),
            Update::Patches(patches) => self.apply_patches(patches),
        }
    }

    /// Replaces the whole tree atomically. Returns the old and the new ids.
    pub fn replace(&mut self, nodes: Vec<Node>) -> Result<BTreeSet<NodeId>, TreeError> {
        if nodes.len() > self.limits.max_nodes {
            return Err(TreeError::TreeTooLarge {
                max: self.limits.max_nodes,
            });
        }
        let mut next = Self::new(self.limits);
        for node in nodes {
            next.insert(InsertOp { node, before: None })?;
        }
        let touched = self
            .nodes
            .keys()
            .chain(next.nodes.keys())
            .copied()
            .collect();
        *self = next;
        Ok(touched)
    }

    /// Applies patches in order, atomically: on error the tree is left as it was. Returns the
    /// touched ids, as [`Tree::apply_update`] does.
    pub fn apply_patches(&mut self, patches: Vec<Patch>) -> Result<BTreeSet<NodeId>, TreeError> {
        self.check_update_size(patches.len())?;
        let mut touched = BTreeSet::new();
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

    fn apply_patch(
        &mut self,
        patch: Patch,
        touched: &mut BTreeSet<NodeId>,
    ) -> Result<(), TreeError> {
        match patch {
            Patch::SetProps(id, props) => {
                self.set_props(id, props)?;
                touched.insert(id);
            }
            Patch::Insert(op) => {
                let id = op.node.id;
                self.insert(op)?;
                touched.insert(id);
            }
            Patch::Remove(id) => self.remove(id, touched)?,
        }
        Ok(())
    }

    fn set_props(&mut self, id: NodeId, props: Props) -> Result<(), TreeError> {
        let node = self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))?;
        if !node.children.is_empty() && !props.is_container() {
            return Err(TreeError::NotAContainer {
                parent: id,
                kind: props.kind_name(),
            });
        }
        node.props = props;
        Ok(())
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
    use crate::types::{ButtonProps, FlexProps, TextProps};

    fn flex(id: NodeId, parent: Option<NodeId>) -> Node {
        Node {
            id,
            parent,
            props: Props::Flex(FlexProps::default()),
        }
    }

    fn text(id: NodeId, parent: Option<NodeId>, content: &str) -> Node {
        Node {
            id,
            parent,
            props: text_props(content),
        }
    }

    fn text_props(content: &str) -> Props {
        Props::Text(TextProps {
            content: content.into(),
            ..TextProps::default()
        })
    }

    fn insert(node: Node, before: Option<NodeId>) -> Patch {
        Patch::Insert(InsertOp { node, before })
    }

    /// 1 (flex) -> [2 (text), 3 (flex) -> [4 (text)]]
    fn sample() -> Tree {
        Tree::from_nodes(
            vec![
                flex(1, None),
                text(2, Some(1), "a"),
                flex(3, Some(1)),
                text(4, Some(3), "b"),
            ],
            TreeLimits::default(),
        )
        .expect("valid tree")
    }

    #[test]
    fn builds_from_flat_list() {
        let tree = sample();
        assert_eq!(tree.len(), 4);
        assert_eq!(tree.roots(), &[1]);
        assert_eq!(tree.children(Some(1)), &[2, 3]);
        assert_eq!(tree.children(Some(3)), &[4]);
        assert_eq!(tree.get(4).map(TreeNode::parent), Some(Some(3)));
        assert_eq!(
            tree.to_nodes().iter().map(|n| n.id).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn round_trips_through_flat_list() {
        let tree = sample();
        let rebuilt = Tree::from_nodes(tree.to_nodes(), TreeLimits::default()).expect("valid");
        assert_eq!(rebuilt, tree);
    }

    #[test]
    fn allows_several_roots() {
        let tree = Tree::from_nodes(
            vec![text(1, None, "a"), text(2, None, "b")],
            TreeLimits::default(),
        )
        .expect("valid");
        assert_eq!(tree.roots(), &[1, 2]);
    }

    #[test]
    fn rejects_child_before_parent() {
        let error = Tree::from_nodes(
            vec![text(2, Some(1), "a"), flex(1, None)],
            TreeLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error, TreeError::UnknownParent { node: 2, parent: 1 });
    }

    #[test]
    fn rejects_self_parent() {
        let error = Tree::from_nodes(vec![flex(1, Some(1))], TreeLimits::default()).unwrap_err();
        assert_eq!(error, TreeError::UnknownParent { node: 1, parent: 1 });
    }

    #[test]
    fn rejects_duplicate_ids() {
        let error = Tree::from_nodes(
            vec![flex(1, None), text(1, Some(1), "a")],
            TreeLimits::default(),
        )
        .unwrap_err();
        assert_eq!(error, TreeError::DuplicateId(1));
    }

    #[test]
    fn rejects_children_of_leaf_nodes() {
        let error = Tree::from_nodes(
            vec![text(1, None, "a"), text(2, Some(1), "b")],
            TreeLimits::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            TreeError::NotAContainer {
                parent: 1,
                kind: "text"
            }
        );
    }

    #[test]
    fn caps_nodes_per_update() {
        let limits = TreeLimits {
            max_nodes_per_update: 2,
            max_nodes: 100,
        };
        let nodes = vec![flex(1, None), flex(2, Some(1)), flex(3, Some(1))];
        // A mount is capped by the tree size only; a row by the per-update cap.
        let mut tree = Tree::from_nodes(nodes.clone(), limits).expect("mount over the update cap");
        let error = Tree::from_row(nodes, limits).unwrap_err();
        assert_eq!(error, TreeError::UpdateTooLarge { count: 3, max: 2 });

        tree.replace(vec![flex(1, None)]).expect("valid");
        let error = tree
            .apply_patches(vec![
                insert(text(2, Some(1), "a"), None),
                insert(text(3, Some(1), "b"), None),
                insert(text(4, Some(1), "c"), None),
            ])
            .unwrap_err();
        assert_eq!(error, TreeError::UpdateTooLarge { count: 3, max: 2 });
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn caps_total_nodes() {
        let limits = TreeLimits {
            max_nodes_per_update: 10,
            max_nodes: 2,
        };
        let mut tree =
            Tree::from_nodes(vec![flex(1, None), flex(2, Some(1))], limits).expect("valid");
        let error = tree
            .apply_patches(vec![insert(text(3, Some(1), "a"), None)])
            .unwrap_err();
        assert!(matches!(
            error,
            TreeError::Patch { index: 0, ref error } if **error == TreeError::TreeTooLarge { max: 2 }
        ));
        assert_eq!(
            tree.apply_update(Update::Replace(vec![
                flex(1, None),
                flex(2, Some(1)),
                flex(3, Some(1)),
            ])),
            Err(TreeError::TreeTooLarge { max: 2 })
        );
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn set_props_replaces_props() {
        let mut tree = sample();
        tree.apply_patches(vec![Patch::SetProps(2, text_props("changed"))])
            .expect("valid");
        assert_eq!(
            tree.get(2).map(TreeNode::props),
            Some(&text_props("changed"))
        );
    }

    #[test]
    fn set_props_cannot_turn_a_parent_into_a_leaf() {
        let mut tree = sample();
        let error = tree
            .apply_patches(vec![Patch::SetProps(3, text_props("x"))])
            .unwrap_err();
        assert!(matches!(error, TreeError::Patch { index: 0, .. }));
    }

    #[test]
    fn set_props_on_unknown_node_fails() {
        let mut tree = sample();
        let error = tree
            .apply_patches(vec![Patch::SetProps(99, text_props("x"))])
            .unwrap_err();
        assert_eq!(
            error,
            TreeError::Patch {
                index: 0,
                error: Box::new(TreeError::UnknownNode(99))
            }
        );
    }

    #[test]
    fn insert_appends_or_goes_before_sibling() {
        let mut tree = sample();
        tree.apply_patches(vec![
            insert(text(5, Some(1), "end"), None),
            insert(text(6, Some(1), "first"), Some(2)),
            insert(text(7, Some(1), "middle"), Some(3)),
            insert(flex(8, None), None),
        ])
        .expect("valid");
        assert_eq!(tree.children(Some(1)), &[6, 2, 7, 3, 5]);
        assert_eq!(tree.roots(), &[1, 8]);
    }

    #[test]
    fn insert_errors() {
        let mut tree = sample();
        let cases = [
            (
                insert(text(2, Some(1), "dup"), None),
                TreeError::DuplicateId(2),
            ),
            (
                insert(text(9, Some(42), "orphan"), None),
                TreeError::UnknownParent {
                    node: 9,
                    parent: 42,
                },
            ),
            (
                // 4 is a child of 3, not a sibling under 1.
                insert(text(9, Some(1), "x"), Some(4)),
                TreeError::UnknownSibling { node: 9, before: 4 },
            ),
            (
                insert(text(9, Some(2), "x"), None),
                TreeError::NotAContainer {
                    parent: 2,
                    kind: "text",
                },
            ),
        ];
        for (patch, expected) in cases {
            let error = tree.apply_patches(vec![patch]).unwrap_err();
            assert_eq!(
                error,
                TreeError::Patch {
                    index: 0,
                    error: Box::new(expected)
                }
            );
        }
        assert_eq!(tree, sample());
    }

    #[test]
    fn remove_drops_subtree() {
        let mut tree = sample();
        tree.apply_patches(vec![Patch::Remove(3)]).expect("valid");
        assert_eq!(tree.len(), 2);
        assert!(!tree.contains(3));
        assert!(!tree.contains(4));
        assert_eq!(tree.children(Some(1)), &[2]);

        // Removed ids can be inserted again.
        tree.apply_patches(vec![insert(text(4, Some(1), "again"), None)])
            .expect("valid");
        assert_eq!(tree.children(Some(1)), &[2, 4]);
    }

    #[test]
    fn remove_root_empties_tree() {
        let mut tree = sample();
        tree.apply_patches(vec![Patch::Remove(1)]).expect("valid");
        assert!(tree.is_empty());
        assert!(tree.roots().is_empty());
    }

    #[test]
    fn remove_unknown_node_fails() {
        let mut tree = sample();
        let error = tree
            .apply_patches(vec![Patch::Remove(4), Patch::Remove(4)])
            .unwrap_err();
        assert_eq!(
            error,
            TreeError::Patch {
                index: 1,
                error: Box::new(TreeError::UnknownNode(4))
            }
        );
    }

    #[test]
    fn failed_patch_batch_is_rolled_back() {
        let mut tree = sample();
        let error = tree
            .apply_patches(vec![
                Patch::Remove(4),
                Patch::SetProps(2, text_props("changed")),
                insert(text(9, Some(77), "orphan"), None),
            ])
            .unwrap_err();
        assert!(matches!(error, TreeError::Patch { index: 2, .. }));
        assert_eq!(tree, sample());
    }

    #[test]
    fn apply_update_reports_touched_ids() {
        let mut tree = sample();
        let ids = |ids: &[NodeId]| ids.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(tree.apply_update(Update::Unchanged), Ok(ids(&[])));
        assert_eq!(tree.apply_update(Update::Patches(Vec::new())), Ok(ids(&[])));
        assert_eq!(
            tree.apply_update(Update::Patches(vec![
                Patch::SetProps(2, text_props("changed")),
                insert(text(5, Some(1), "new"), None),
            ])),
            Ok(ids(&[2, 5]))
        );
        // Removing 3 removes its child 4 too.
        assert_eq!(
            tree.apply_update(Update::Patches(vec![Patch::Remove(3)])),
            Ok(ids(&[3, 4]))
        );
        assert_eq!(
            tree.apply_update(Update::Replace(vec![text(10, None, "new")])),
            Ok(ids(&[1, 2, 5, 10]))
        );
        assert_eq!(tree.roots(), &[10]);
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn failed_replace_keeps_old_tree() {
        let mut tree = sample();
        let error = tree
            .apply_update(Update::Replace(vec![flex(1, None), flex(1, None)]))
            .unwrap_err();
        assert_eq!(error, TreeError::DuplicateId(1));
        assert_eq!(tree, sample());
    }

    #[test]
    fn row_needs_single_root() {
        let button = Node {
            id: 2,
            parent: Some(1),
            props: Props::Button(ButtonProps::default()),
        };
        Tree::from_row(vec![flex(1, None), button], TreeLimits::default()).expect("valid row");
        assert_eq!(
            Tree::from_row(
                vec![text(1, None, "a"), text(2, None, "b")],
                TreeLimits::default()
            )
            .unwrap_err(),
            TreeError::RowRootCount(2)
        );
        assert_eq!(
            Tree::from_row(Vec::new(), TreeLimits::default()).unwrap_err(),
            TreeError::RowRootCount(0)
        );
    }

    #[test]
    fn json_form_matches_wit_names() {
        let update = Update::Patches(vec![
            Patch::SetProps(
                3,
                Props::List(crate::types::ListProps {
                    row_count: 10,
                    row_height: 22,
                }),
            ),
            Patch::Remove(4),
        ]);
        let json = serde_json::to_value(&update).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "patches": [
                    { "set-props": [3, { "list": { "row-count": 10, "row-height": 22 } }] },
                    { "remove": 4 },
                ]
            })
        );
        let back: Update = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, update);
    }
}
