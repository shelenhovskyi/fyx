use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestCaseError;
use serde::{Serialize, de::DeserializeOwned};
use ui_ext::{
    Align, ButtonProps, ButtonStyle, ColorToken, DiffViewProps, DockPosition, FlexDirection,
    FlexProps, IconProps, InputProps, InsertOp, ListProps, ListRow, LogLevel, Node, NodeId,
    PanelDescriptor, Patch, Props, StatusBarSide, StatusItemDescriptor, Surface, SurfaceKind,
    TableProps, TextProps, Tree, TreeError, TreeLimits, UiEvent, Update,
};

/// `arb_nodes` never generates this id and nothing inserts it, so a patch naming it always fails.
const UNKNOWN: NodeId = u32::MAX - 1;
/// Also never generated: the id of the node in an insert that must fail for another reason.
const UNUSED: NodeId = u32::MAX - 2;
/// `Driver` hands out fresh ids from here, above every id `arb_nodes` generates.
const FIRST_FRESH_ID: NodeId = 1_000_000;

fn arb_string() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..8).prop_map(String::from_iter)
}

fn arb_color() -> impl Strategy<Value = ColorToken> {
    prop_oneof![
        Just(ColorToken::Text),
        Just(ColorToken::TextMuted),
        Just(ColorToken::TextAccent),
        Just(ColorToken::Error),
        Just(ColorToken::Warning),
        Just(ColorToken::Success),
        Just(ColorToken::Border),
        Just(ColorToken::Surface),
        Just(ColorToken::ElevatedSurface),
    ]
}

fn arb_flex_props() -> impl Strategy<Value = FlexProps> {
    (
        prop_oneof![Just(FlexDirection::Row), Just(FlexDirection::Column)],
        any::<u16>(),
        any::<u16>(),
        any::<bool>(),
        prop_oneof![
            Just(Align::Start),
            Just(Align::Center),
            Just(Align::End),
            Just(Align::Stretch),
        ],
        prop::option::of(arb_color()),
    )
        .prop_map(
            |(direction, gap, padding, grow, align, background)| FlexProps {
                direction,
                gap,
                padding,
                grow,
                align,
                background,
            },
        )
}

fn arb_leaf_props() -> impl Strategy<Value = Props> {
    let text = (
        arb_string(),
        prop::option::of(arb_color()),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(content, color, bold, monospace)| {
            Props::Text(TextProps {
                content,
                color,
                bold,
                monospace,
            })
        });
    let button = (
        arb_string(),
        prop::option::of(arb_string()),
        prop_oneof![
            Just(ButtonStyle::Default),
            Just(ButtonStyle::Filled),
            Just(ButtonStyle::Subtle),
            Just(ButtonStyle::Danger),
        ],
        any::<bool>(),
    )
        .prop_map(|(label, icon, style, disabled)| {
            Props::Button(ButtonProps {
                label,
                icon,
                style,
                disabled,
            })
        });
    let input = (arb_string(), arb_string())
        .prop_map(|(value, placeholder)| Props::Input(InputProps { value, placeholder }));
    let list = (any::<u32>(), any::<u16>()).prop_map(|(row_count, row_height)| {
        Props::List(ListProps {
            row_count,
            row_height,
        })
    });
    let table = (prop::collection::vec(arb_string(), 0..4), any::<u32>())
        .prop_map(|(columns, row_count)| Props::Table(TableProps { columns, row_count }));
    let icon = (arb_string(), prop::option::of(arb_color()))
        .prop_map(|(name, color)| Props::Icon(IconProps { name, color }));
    let diff_view = (arb_string(), arb_string(), prop::option::of(arb_string())).prop_map(
        |(old_text, new_text, language)| {
            Props::DiffView(DiffViewProps {
                old_text,
                new_text,
                language,
            })
        },
    );
    prop_oneof![
        text,
        button,
        input,
        list,
        table,
        icon,
        arb_string().prop_map(Props::Markdown),
        diff_view,
    ]
}

/// Flex nodes are weighted up so that generated trees have some depth.
fn arb_props() -> impl Strategy<Value = Props> {
    prop_oneof![
        3 => arb_flex_props().prop_map(Props::Flex),
        4 => arb_leaf_props(),
    ]
}

/// A valid flat tree: distinct ids in random order, parents first, every parent a flex node.
fn arb_nodes(max_len: usize) -> impl Strategy<Value = Vec<Node>> {
    let candidates: Vec<NodeId> = (0..=300).chain([u32::MAX]).collect();
    (0..=max_len)
        .prop_flat_map(move |len| {
            (
                prop::sample::subsequence(candidates.clone(), len).prop_shuffle(),
                prop::collection::vec(
                    (
                        prop::bool::weighted(0.15),
                        prop::bool::weighted(0.4),
                        any::<Index>(),
                        arb_props(),
                    ),
                    len,
                ),
            )
        })
        .prop_map(|(ids, specs)| {
            let mut flex_ids = Vec::new();
            let mut nodes = Vec::with_capacity(ids.len());
            for (id, (root, under_latest, parent_pick, props)) in ids.into_iter().zip(specs) {
                let parent = if root {
                    None
                } else if under_latest {
                    flex_ids.last().copied()
                } else if flex_ids.is_empty() {
                    None
                } else {
                    Some(*parent_pick.get(&flex_ids))
                };
                if matches!(props, Props::Flex(_)) {
                    flex_ids.push(id);
                }
                nodes.push(Node { id, parent, props });
            }
            nodes
        })
}

fn arb_limits() -> impl Strategy<Value = TreeLimits> {
    prop_oneof![
        2 => Just(TreeLimits::default()),
        1 => (0usize..=12, 1usize..=48).prop_map(|(max_nodes_per_update, max_nodes)| TreeLimits {
            max_nodes_per_update,
            max_nodes,
        }),
    ]
}

fn insert(node: Node, before: Option<NodeId>) -> Patch {
    Patch::Insert(InsertOp { node, before })
}

fn leaf(id: NodeId, parent: Option<NodeId>) -> Node {
    Node {
        id,
        parent,
        props: Props::Markdown(String::new()),
    }
}

fn patch_error(index: usize, error: TreeError) -> TreeError {
    TreeError::Patch {
        index,
        error: Box::new(error),
    }
}

/// Written out here rather than calling `Props::kind_name`, so the names are checked against the
/// WIT case names too.
fn kind_of(props: &Props) -> &'static str {
    match props {
        Props::Flex(_) => "flex",
        Props::Text(_) => "text",
        Props::Button(_) => "button",
        Props::Input(_) => "input",
        Props::List(_) => "list",
        Props::Table(_) => "table",
        Props::Icon(_) => "icon",
        Props::Markdown(_) => "markdown",
        Props::DiffView(_) => "diff-view",
    }
}

fn subtree_ids(tree: &Tree, id: NodeId) -> BTreeSet<NodeId> {
    let mut ids = BTreeSet::new();
    let mut pending = vec![id];
    while let Some(id) = pending.pop() {
        if ids.insert(id) {
            pending.extend_from_slice(tree.children(Some(id)));
        }
    }
    ids
}

/// Every node is reachable from the roots exactly once (so there are no orphans), names the
/// parent it is listed under, and only flex nodes have children.
fn check_invariants(tree: &Tree) -> Result<(), TestCaseError> {
    let mut seen = BTreeSet::new();
    let mut pending: Vec<(Option<NodeId>, NodeId)> =
        tree.roots().iter().map(|&id| (None, id)).collect();
    while let Some((parent, id)) = pending.pop() {
        prop_assert!(seen.insert(id), "node {} is reachable twice", id);
        let node = tree
            .get(id)
            .ok_or_else(|| TestCaseError::fail(format!("listed node {id} does not exist")))?;
        prop_assert_eq!(node.parent(), parent);
        if !node.children().is_empty() {
            prop_assert!(
                matches!(node.props(), Props::Flex(_)),
                "{} node {} has children",
                kind_of(node.props()),
                id
            );
        }
        pending.extend(node.children().iter().map(|&child| (Some(id), child)));
    }
    prop_assert_eq!(seen.len(), tree.len(), "some nodes are unreachable");
    Ok(())
}

fn check_parents_first(nodes: &[Node]) -> Result<(), TestCaseError> {
    let mut earlier = BTreeSet::new();
    for node in nodes {
        if let Some(parent) = node.parent {
            prop_assert!(
                earlier.contains(&parent),
                "node {} precedes its parent {}",
                node.id,
                parent
            );
        }
        prop_assert!(earlier.insert(node.id), "node {} is listed twice", node.id);
    }
    Ok(())
}

/// Reference model: the tree as one pre-order list, so a subtree is a contiguous run. This is a
/// different representation from `Tree`'s, which is the point of comparing the two.
#[derive(Clone, Debug)]
struct Model {
    nodes: Vec<Node>,
    limits: TreeLimits,
}

#[derive(Debug)]
enum ModelError {
    UpdateTooLarge,
    Patch(usize),
    Replace,
}

impl Model {
    fn from_nodes(nodes: &[Node], limits: TreeLimits) -> Option<Self> {
        if nodes.len() > limits.max_nodes {
            return None;
        }
        let mut model = Self {
            nodes: Vec::new(),
            limits,
        };
        for node in nodes {
            model
                .apply(&insert(node.clone(), None), &mut BTreeSet::new())
                .ok()?;
        }
        Some(model)
    }

    fn ids(&self) -> BTreeSet<NodeId> {
        self.nodes.iter().map(|node| node.id).collect()
    }

    fn position(&self, id: NodeId) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }

    fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    fn children(&self, parent: Option<NodeId>) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|node| node.parent == parent)
            .map(|node| node.id)
            .collect()
    }

    /// The exclusive end of the subtree that starts at `start`.
    fn subtree_end(&self, start: usize) -> usize {
        let mut members = BTreeSet::new();
        if let Some(node) = self.nodes.get(start) {
            members.insert(node.id);
        }
        let mut end = start + 1;
        while let Some(node) = self.nodes.get(end) {
            match node.parent {
                Some(parent) if members.contains(&parent) => {
                    members.insert(node.id);
                    end += 1;
                }
                _ => break,
            }
        }
        end
    }

    /// Leaves the model unchanged on error.
    fn apply(&mut self, patch: &Patch, touched: &mut BTreeSet<NodeId>) -> Result<(), ()> {
        match patch {
            Patch::SetProps(id, props) => {
                let has_children = self.nodes.iter().any(|node| node.parent == Some(*id));
                if has_children && !matches!(props, Props::Flex(_)) {
                    return Err(());
                }
                let node = self
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == *id)
                    .ok_or(())?;
                node.props = props.clone();
                touched.insert(*id);
            }
            Patch::Insert(InsertOp { node, before }) => {
                if self.get(node.id).is_some() || self.nodes.len() >= self.limits.max_nodes {
                    return Err(());
                }
                if let Some(parent) = node.parent {
                    if !matches!(self.get(parent).ok_or(())?.props, Props::Flex(_)) {
                        return Err(());
                    }
                }
                let index = match (before, node.parent) {
                    (Some(before), _) => {
                        let index = self.position(*before).ok_or(())?;
                        if self.nodes.get(index).map(|sibling| sibling.parent) != Some(node.parent)
                        {
                            return Err(());
                        }
                        index
                    }
                    (None, Some(parent)) => self.subtree_end(self.position(parent).ok_or(())?),
                    (None, None) => self.nodes.len(),
                };
                self.nodes.insert(index, node.clone());
                touched.insert(node.id);
            }
            Patch::Remove(id) => {
                let start = self.position(*id).ok_or(())?;
                let end = self.subtree_end(start);
                touched.extend(self.nodes.drain(start..end).map(|node| node.id));
            }
        }
        Ok(())
    }

    fn apply_update(&mut self, update: &Update) -> Result<BTreeSet<NodeId>, ModelError> {
        match update {
            Update::Unchanged => Ok(BTreeSet::new()),
            Update::Replace(nodes) => {
                let next = Self::from_nodes(nodes, self.limits).ok_or(ModelError::Replace)?;
                let touched = self.ids().union(&next.ids()).copied().collect();
                *self = next;
                Ok(touched)
            }
            Update::Patches(patches) => {
                if patches.len() > self.limits.max_nodes_per_update {
                    return Err(ModelError::UpdateTooLarge);
                }
                let mut next = self.clone();
                let mut touched = BTreeSet::new();
                for (index, patch) in patches.iter().enumerate() {
                    next.apply(patch, &mut touched)
                        .map_err(|()| ModelError::Patch(index))?;
                }
                *self = next;
                Ok(touched)
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Target {
    Live(Index),
    Unknown,
}

#[derive(Clone, Debug)]
enum ParentChoice {
    Root,
    Flex(Index),
    /// Often a leaf, which the host must reject.
    Any(Index),
    Unknown,
}

#[derive(Clone, Debug)]
enum BeforeChoice {
    Append,
    Sibling(Index),
    /// Usually not a sibling, which the host must reject.
    Any(Index),
    Unknown,
}

#[derive(Clone, Debug)]
enum IdChoice {
    Fresh,
    /// An id that was in the tree once and is not any more.
    Removed(Index),
    /// A live id, which the host must reject as a duplicate.
    Live(Index),
}

/// A patch whose ids are picked from the tree state it is applied to, so a random sequence stays
/// mostly valid while still producing every kind of invalid patch.
#[derive(Clone, Debug)]
enum Op {
    SetProps {
        target: Target,
        keep_kind: bool,
        flex: FlexProps,
        other: Props,
    },
    Insert {
        id: IdChoice,
        parent: ParentChoice,
        before: BeforeChoice,
        props: Props,
    },
    Remove(Target),
}

fn arb_target() -> impl Strategy<Value = Target> {
    prop_oneof![
        24 => any::<Index>().prop_map(Target::Live),
        1 => Just(Target::Unknown),
    ]
}

fn arb_op() -> impl Strategy<Value = Op> {
    let set_props = (
        arb_target(),
        prop::bool::weighted(0.9),
        arb_flex_props(),
        arb_props(),
    )
        .prop_map(|(target, keep_kind, flex, other)| Op::SetProps {
            target,
            keep_kind,
            flex,
            other,
        });
    let id = prop_oneof![
        16 => Just(IdChoice::Fresh),
        4 => any::<Index>().prop_map(IdChoice::Removed),
        1 => any::<Index>().prop_map(IdChoice::Live),
    ];
    let parent = prop_oneof![
        3 => Just(ParentChoice::Root),
        16 => any::<Index>().prop_map(ParentChoice::Flex),
        1 => any::<Index>().prop_map(ParentChoice::Any),
        1 => Just(ParentChoice::Unknown),
    ];
    let before = prop_oneof![
        10 => Just(BeforeChoice::Append),
        8 => any::<Index>().prop_map(BeforeChoice::Sibling),
        1 => any::<Index>().prop_map(BeforeChoice::Any),
        1 => Just(BeforeChoice::Unknown),
    ];
    let insert =
        (id, parent, before, arb_props()).prop_map(|(id, parent, before, props)| Op::Insert {
            id,
            parent,
            before,
            props,
        });
    prop_oneof![
        3 => set_props,
        5 => insert,
        2 => arb_target().prop_map(Op::Remove),
    ]
}

/// Turns `Op`s into concrete patches and keeps the id bookkeeping across batches.
struct Driver {
    next_fresh: NodeId,
    ever_seen: BTreeSet<NodeId>,
}

impl Driver {
    fn new(nodes: &[Node]) -> Self {
        Self {
            next_fresh: FIRST_FRESH_ID,
            ever_seen: nodes.iter().map(|node| node.id).collect(),
        }
    }

    fn fresh(&mut self) -> NodeId {
        let id = self.next_fresh;
        self.next_fresh += 1;
        id
    }

    /// With `keep_failing` false, patches that would fail are dropped, so the batch is valid.
    fn resolve(&mut self, model: &Model, ops: &[Op], keep_failing: bool) -> Vec<Patch> {
        let mut scratch = model.clone();
        let mut patches = Vec::new();
        for op in ops {
            let patch = self.concrete(&scratch, op);
            if let Patch::Insert(op) = &patch {
                self.ever_seen.insert(op.node.id);
            }
            let succeeded = scratch.apply(&patch, &mut BTreeSet::new()).is_ok();
            if succeeded || keep_failing {
                patches.push(patch);
            }
        }
        patches
    }

    fn concrete(&mut self, scratch: &Model, op: &Op) -> Patch {
        let pick = |index: &Index, ids: &[NodeId]| -> Option<NodeId> {
            (!ids.is_empty()).then(|| *index.get(ids))
        };
        let live: Vec<NodeId> = scratch.nodes.iter().map(|node| node.id).collect();
        let target = |target: &Target| match target {
            Target::Live(index) => pick(index, &live).unwrap_or(UNKNOWN),
            Target::Unknown => UNKNOWN,
        };
        match op {
            Op::SetProps {
                target: choice,
                keep_kind,
                flex,
                other,
            } => {
                let id = target(choice);
                let is_flex = scratch
                    .get(id)
                    .is_some_and(|node| matches!(node.props, Props::Flex(_)));
                let props = if *keep_kind && is_flex {
                    Props::Flex(flex.clone())
                } else {
                    other.clone()
                };
                Patch::SetProps(id, props)
            }
            Op::Remove(choice) => Patch::Remove(target(choice)),
            Op::Insert {
                id,
                parent,
                before,
                props,
            } => {
                let id = match id {
                    IdChoice::Fresh => self.fresh(),
                    IdChoice::Removed(index) => {
                        let removed: Vec<NodeId> = self
                            .ever_seen
                            .iter()
                            .copied()
                            .filter(|id| scratch.get(*id).is_none())
                            .collect();
                        match pick(index, &removed) {
                            Some(id) => id,
                            None => self.fresh(),
                        }
                    }
                    IdChoice::Live(index) => match pick(index, &live) {
                        Some(id) => id,
                        None => self.fresh(),
                    },
                };
                let parent = match parent {
                    ParentChoice::Root => None,
                    ParentChoice::Flex(index) => {
                        let flex: Vec<NodeId> = scratch
                            .nodes
                            .iter()
                            .filter(|node| matches!(node.props, Props::Flex(_)))
                            .map(|node| node.id)
                            .collect();
                        pick(index, &flex)
                    }
                    ParentChoice::Any(index) => pick(index, &live),
                    ParentChoice::Unknown => Some(UNKNOWN),
                };
                let before = match before {
                    BeforeChoice::Append => None,
                    BeforeChoice::Sibling(index) => pick(index, &scratch.children(parent)),
                    BeforeChoice::Any(index) => pick(index, &live),
                    BeforeChoice::Unknown => Some(UNKNOWN),
                };
                insert(
                    Node {
                        id,
                        parent,
                        props: props.clone(),
                    },
                    before,
                )
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Step {
    Unchanged,
    Replace(Vec<Node>),
    Patches { ops: Vec<Op>, keep_failing: bool },
}

fn arb_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        1 => Just(Step::Unchanged),
        1 => arb_nodes(24).prop_map(Step::Replace),
        12 => (prop::collection::vec(arb_op(), 0..14), prop::bool::weighted(0.4))
            .prop_map(|(ops, keep_failing)| Step::Patches { ops, keep_failing }),
    ]
}

fn same_outcome(
    actual: &Result<BTreeSet<NodeId>, TreeError>,
    expected: &Result<BTreeSet<NodeId>, ModelError>,
) -> bool {
    match (actual, expected) {
        (Ok(actual), Ok(expected)) => actual == expected,
        (Err(TreeError::UpdateTooLarge { .. }), Err(ModelError::UpdateTooLarge)) => true,
        (Err(TreeError::Patch { index, .. }), Err(ModelError::Patch(expected))) => {
            index == expected
        }
        (Err(_), Err(ModelError::Replace)) => true,
        _ => false,
    }
}

fn check_matches_model(tree: &Tree, model: &Model) -> Result<(), TestCaseError> {
    let nodes = tree.to_nodes();
    prop_assert_eq!(&nodes, &model.nodes);
    prop_assert_eq!(tree.len(), model.nodes.len());
    prop_assert!(tree.len() <= tree.limits().max_nodes);
    check_invariants(tree)?;
    check_parents_first(&nodes)?;
    let rebuilt = Tree::from_nodes(nodes, tree.limits())?;
    prop_assert_eq!(&rebuilt, tree);
    Ok(())
}

/// Every id that appeared, disappeared or changed parent or props must be reported as touched.
fn check_touched_covers_changes(
    before: &Tree,
    after: &Tree,
    touched: &BTreeSet<NodeId>,
) -> Result<(), TestCaseError> {
    let entries = |tree: &Tree| -> BTreeMap<NodeId, (Option<NodeId>, Props)> {
        tree.to_nodes()
            .into_iter()
            .map(|node| (node.id, (node.parent, node.props)))
            .collect()
    };
    let before = entries(before);
    let after = entries(after);
    for id in before.keys().chain(after.keys()) {
        if before.get(id) != after.get(id) {
            prop_assert!(
                touched.contains(id),
                "node {} changed but is not touched",
                id
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Random valid and mutated updates against the reference model: same outcome, same touched
    /// ids, same tree, atomic on error.
    #[test]
    fn tree_matches_reference_model(
        limits in arb_limits(),
        initial in arb_nodes(32),
        steps in prop::collection::vec(arb_step(), 1..8),
    ) {
        let mut initial = initial;
        // A prefix of a parents-first list is still a valid tree.
        initial.truncate(limits.max_nodes);
        let mut tree = Tree::from_nodes(initial.clone(), limits)?;
        let mut model = Model::from_nodes(&initial, limits)
            .ok_or_else(|| TestCaseError::fail("model rejected a valid tree"))?;
        let mut driver = Driver::new(&initial);
        check_matches_model(&tree, &model)?;

        for step in steps {
            let update = match step {
                Step::Unchanged => Update::Unchanged,
                Step::Replace(nodes) => {
                    driver.ever_seen.extend(nodes.iter().map(|node| node.id));
                    Update::Replace(nodes)
                }
                Step::Patches { ops, keep_failing } => {
                    Update::Patches(driver.resolve(&model, &ops, keep_failing))
                }
            };
            let before = tree.clone();
            let actual = tree.apply_update(update.clone());
            let expected = model.apply_update(&update);
            prop_assert!(
                same_outcome(&actual, &expected),
                "update {:?}: tree returned {:?}, model {:?}",
                update,
                actual,
                expected
            );
            match &actual {
                Err(_) => prop_assert_eq!(&tree, &before, "a failed update changed the tree"),
                Ok(touched) => {
                    if touched.is_empty() {
                        prop_assert_eq!(&tree, &before, "no touched ids but the tree changed");
                    }
                    check_touched_covers_changes(&before, &tree, touched)?;
                }
            }
            check_matches_model(&tree, &model)?;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn flat_list_round_trips(nodes in arb_nodes(48)) {
        let tree = Tree::from_nodes(nodes.clone(), TreeLimits::default())?;
        prop_assert_eq!(tree.len(), nodes.len());
        check_invariants(&tree)?;

        let flat = tree.to_nodes();
        check_parents_first(&flat)?;
        // Pre-order, with siblings in the order they were listed.
        let model = Model::from_nodes(&nodes, TreeLimits::default())
            .ok_or_else(|| TestCaseError::fail("model rejected a valid tree"))?;
        prop_assert_eq!(&flat, &model.nodes);

        let rebuilt = Tree::from_nodes(flat.clone(), TreeLimits::default())?;
        prop_assert_eq!(&rebuilt, &tree);
        prop_assert_eq!(rebuilt.to_nodes(), flat);
    }

    #[test]
    fn invalid_node_lists_are_rejected(
        nodes in arb_nodes(40),
        corruption in 0..5u8,
        first in any::<Index>(),
        second in any::<Index>(),
        existing in arb_nodes(8),
    ) {
        prop_assume!(nodes.len() >= 2);
        let mut nodes = nodes;
        let len = nodes.len();
        let expected = match corruption {
            0 => {
                let (a, b) = (first.index(len), second.index(len));
                prop_assume!(a != b);
                let (earlier, later) = (a.min(b), a.max(b));
                let id = nodes[earlier].id;
                nodes[later].id = id;
                TreeError::DuplicateId(id)
            }
            1 => {
                let node = &mut nodes[first.index(len)];
                node.parent = Some(UNKNOWN);
                TreeError::UnknownParent { node: node.id, parent: UNKNOWN }
            }
            2 => {
                let node = &mut nodes[first.index(len)];
                node.parent = Some(node.id);
                TreeError::UnknownParent { node: node.id, parent: node.id }
            }
            3 => {
                let leaves: Vec<usize> = (0..len - 1)
                    .filter(|&index| !matches!(nodes[index].props, Props::Flex(_)))
                    .collect();
                prop_assume!(!leaves.is_empty());
                let leaf_index = *first.get(&leaves);
                let child_index = leaf_index + 1 + second.index(len - leaf_index - 1);
                let leaf = &nodes[leaf_index];
                let error = TreeError::NotAContainer {
                    parent: leaf.id,
                    kind: kind_of(&leaf.props),
                };
                nodes[child_index].parent = Some(leaf.id);
                error
            }
            _ => {
                let children: Vec<usize> =
                    (0..len).filter(|&index| nodes[index].parent.is_some()).collect();
                prop_assume!(!children.is_empty());
                let child_index = *first.get(&children);
                let child = nodes.remove(child_index);
                let parent = child.parent.unwrap_or(UNKNOWN);
                let parent_index = nodes
                    .iter()
                    .position(|node| node.id == parent)
                    .ok_or_else(|| TestCaseError::fail("generated child without parent"))?;
                let error = TreeError::UnknownParent { node: child.id, parent };
                nodes.insert(parent_index, child);
                error
            }
        };
        prop_assert_eq!(
            Tree::from_nodes(nodes.clone(), TreeLimits::default()),
            Err(expected.clone())
        );

        let mut tree = Tree::from_nodes(existing, TreeLimits::default())?;
        let before = tree.clone();
        prop_assert_eq!(tree.apply_update(Update::Replace(nodes)), Err(expected));
        prop_assert_eq!(&tree, &before);
    }

    #[test]
    fn failed_batch_leaves_tree_unchanged(
        nodes in arb_nodes(32),
        ops in prop::collection::vec(arb_op(), 0..16),
        position in any::<Index>(),
        failure in 0..4u8,
        props in arb_props(),
    ) {
        let mut tree = Tree::from_nodes(nodes.clone(), TreeLimits::default())?;
        let model = Model::from_nodes(&nodes, TreeLimits::default())
            .ok_or_else(|| TestCaseError::fail("model rejected a valid tree"))?;
        let mut patches = Driver::new(&nodes).resolve(&model, &ops, false);
        let (patch, error) = match failure {
            0 => (Patch::SetProps(UNKNOWN, props), TreeError::UnknownNode(UNKNOWN)),
            1 => (Patch::Remove(UNKNOWN), TreeError::UnknownNode(UNKNOWN)),
            2 => (
                insert(Node { id: UNUSED, parent: Some(UNKNOWN), props }, None),
                TreeError::UnknownParent { node: UNUSED, parent: UNKNOWN },
            ),
            _ => (
                insert(Node { id: UNUSED, parent: None, props }, Some(UNKNOWN)),
                TreeError::UnknownSibling { node: UNUSED, before: UNKNOWN },
            ),
        };
        let index = position.index(patches.len() + 1);
        patches.insert(index, patch);

        let before = tree.clone();
        prop_assert_eq!(tree.apply_patches(patches), Err(patch_error(index, error)));
        prop_assert_eq!(&tree, &before);
        prop_assert_eq!(tree.to_nodes(), before.to_nodes());
    }

    #[test]
    fn remove_drops_whole_subtree_and_frees_its_ids(
        nodes in arb_nodes(40),
        pick in any::<Index>(),
    ) {
        prop_assume!(!nodes.is_empty());
        let mut tree = Tree::from_nodes(nodes.clone(), TreeLimits::default())?;
        let removed_id = pick.get(&nodes).id;
        let subtree = subtree_ids(&tree, removed_id);
        let parent = tree.get(removed_id).and_then(|node| node.parent());
        let before = tree.clone();

        let touched = tree.apply_patches(vec![Patch::Remove(removed_id)])?;
        prop_assert_eq!(&touched, &subtree);
        prop_assert_eq!(tree.len(), nodes.len() - subtree.len());
        check_invariants(&tree)?;
        prop_assert!(!tree.children(parent).contains(&removed_id));
        for node in &nodes {
            if subtree.contains(&node.id) {
                prop_assert!(!tree.contains(node.id), "node {} survived", node.id);
                continue;
            }
            let (Some(old), Some(new)) = (before.get(node.id), tree.get(node.id)) else {
                return Err(TestCaseError::fail(format!("node {} vanished", node.id)));
            };
            prop_assert_eq!(new.props(), old.props());
            prop_assert_eq!(new.parent(), old.parent());
            let expected_children: Vec<NodeId> = old
                .children()
                .iter()
                .copied()
                .filter(|&child| child != removed_id)
                .collect();
            prop_assert_eq!(new.children(), expected_children.as_slice());
        }

        // The host frees an id as soon as its node is removed; not reusing it is the extension's
        // job (WIT `node-id` doc).
        let mut reinserted = tree.clone();
        let reinsert = subtree.iter().map(|&id| insert(leaf(id, None), None)).collect();
        prop_assert_eq!(&reinserted.apply_patches(reinsert)?, &subtree);
        check_invariants(&reinserted)?;

        for node in &nodes {
            if subtree.contains(&node.id) {
                continue;
            }
            let result = tree.apply_patches(vec![insert(leaf(node.id, None), None)]);
            prop_assert_eq!(result, Err(patch_error(0, TreeError::DuplicateId(node.id))));
        }

        // Within one batch: removing an id frees it for a later patch, removing it twice fails.
        let mut same_batch = before.clone();
        let touched = same_batch.apply_patches(vec![
            Patch::Remove(removed_id),
            insert(leaf(removed_id, None), None),
        ])?;
        prop_assert_eq!(&touched, &subtree);
        prop_assert_eq!(same_batch.roots().last(), Some(&removed_id));
        let mut twice = before.clone();
        prop_assert_eq!(
            twice.apply_patches(vec![Patch::Remove(removed_id), Patch::Remove(removed_id)]),
            Err(patch_error(1, TreeError::UnknownNode(removed_id)))
        );
        prop_assert_eq!(&twice, &before);
    }

    #[test]
    fn only_flex_nodes_have_children(
        nodes in arb_nodes(40),
        pick in any::<Index>(),
        child_props in arb_props(),
        leaf_props in arb_leaf_props(),
        flex_props in arb_flex_props(),
    ) {
        prop_assume!(!nodes.is_empty());
        let tree = Tree::from_nodes(nodes.clone(), TreeLimits::default())?;
        let node = pick.get(&nodes);
        let is_flex = matches!(node.props, Props::Flex(_));
        let has_children = !tree.children(Some(node.id)).is_empty();
        prop_assert!(is_flex || !has_children);

        let mut with_child = tree.clone();
        let child = Node { id: FIRST_FRESH_ID, parent: Some(node.id), props: child_props };
        let result = with_child.apply_patches(vec![insert(child, None)]);
        if is_flex {
            prop_assert_eq!(result, Ok(BTreeSet::from([FIRST_FRESH_ID])));
            prop_assert_eq!(with_child.children(Some(node.id)).last(), Some(&FIRST_FRESH_ID));
        } else {
            prop_assert_eq!(
                result,
                Err(patch_error(0, TreeError::NotAContainer {
                    parent: node.id,
                    kind: kind_of(&node.props),
                }))
            );
            prop_assert_eq!(&with_child, &tree);
        }

        let mut to_leaf = tree.clone();
        let result = to_leaf.apply_patches(vec![Patch::SetProps(node.id, leaf_props.clone())]);
        if has_children {
            prop_assert_eq!(
                result,
                Err(patch_error(0, TreeError::NotAContainer {
                    parent: node.id,
                    kind: kind_of(&leaf_props),
                }))
            );
            prop_assert_eq!(&to_leaf, &tree);
        } else {
            prop_assert_eq!(result, Ok(BTreeSet::from([node.id])));
            prop_assert_eq!(to_leaf.get(node.id).map(|node| node.props()), Some(&leaf_props));
        }

        let mut to_flex = tree;
        let flex = Props::Flex(flex_props);
        prop_assert_eq!(
            to_flex.apply_patches(vec![Patch::SetProps(node.id, flex.clone())]),
            Ok(BTreeSet::from([node.id]))
        );
        prop_assert_eq!(to_flex.get(node.id).map(|node| node.props()), Some(&flex));
        check_invariants(&to_flex)?;
    }

    #[test]
    fn before_must_be_a_sibling_under_the_same_parent(
        nodes in arb_nodes(40),
        parent_pick in prop::option::of(any::<Index>()),
        before_pick in any::<Index>(),
        unknown_before in prop::bool::weighted(0.1),
        props in arb_props(),
    ) {
        prop_assume!(!nodes.is_empty());
        let mut tree = Tree::from_nodes(nodes.clone(), TreeLimits::default())?;
        let flex_ids: Vec<NodeId> = nodes
            .iter()
            .filter(|node| matches!(node.props, Props::Flex(_)))
            .map(|node| node.id)
            .collect();
        let parent = parent_pick
            .filter(|_| !flex_ids.is_empty())
            .map(|pick| *pick.get(&flex_ids));
        let before = if unknown_before { UNKNOWN } else { before_pick.get(&nodes).id };
        let is_sibling = tree.get(before).is_some_and(|node| node.parent() == parent);
        let siblings = tree.children(parent).to_vec();
        let original = tree.clone();

        let new_node = Node { id: FIRST_FRESH_ID, parent, props };
        let result = tree.apply_patches(vec![insert(new_node, Some(before))]);
        if is_sibling {
            prop_assert_eq!(result, Ok(BTreeSet::from([FIRST_FRESH_ID])));
            let mut expected = siblings;
            let index = expected
                .iter()
                .position(|&sibling| sibling == before)
                .ok_or_else(|| TestCaseError::fail("sibling missing"))?;
            expected.insert(index, FIRST_FRESH_ID);
            prop_assert_eq!(tree.children(parent), expected.as_slice());
            check_invariants(&tree)?;
        } else {
            prop_assert_eq!(
                result,
                Err(patch_error(0, TreeError::UnknownSibling { node: FIRST_FRESH_ID, before }))
            );
            prop_assert_eq!(&tree, &original);
        }
    }

    #[test]
    fn batches_above_the_patch_cap_are_rejected(
        nodes in arb_nodes(16),
        per_update in 0usize..8,
        extra in 1usize..4,
        targets in prop::collection::vec(any::<NodeId>(), 12),
    ) {
        let limits = TreeLimits { max_nodes_per_update: per_update, max_nodes: 64 };
        let mut tree = Tree::from_nodes(nodes, limits)?;
        let before = tree.clone();

        let count = per_update + extra;
        let oversized: Vec<Patch> = targets.iter().take(count).map(|&id| Patch::Remove(id)).collect();
        prop_assert_eq!(oversized.len(), count);
        prop_assert_eq!(
            tree.apply_update(Update::Patches(oversized)),
            Err(TreeError::UpdateTooLarge { count, max: per_update })
        );
        prop_assert_eq!(&tree, &before);

        let ids: BTreeSet<NodeId> = (0..per_update as NodeId).map(|k| FIRST_FRESH_ID + k).collect();
        let at_cap = ids.iter().map(|&id| insert(leaf(id, None), None)).collect();
        prop_assert_eq!(tree.apply_update(Update::Patches(at_cap)), Ok(ids));
    }

    #[test]
    fn node_cap_holds_across_batches(
        max_nodes in 1usize..40,
        per_update in 1usize..10,
        batches in prop::collection::vec(
            (
                any::<bool>(),
                any::<Index>(),
                prop::collection::vec(prop::option::of(any::<Index>()), 0..10),
            ),
            1..30,
        ),
    ) {
        let limits = TreeLimits { max_nodes_per_update: per_update, max_nodes };
        let mut tree = Tree::new(limits);
        let mut next_id: NodeId = 0;
        for (remove_first, remove_pick, parent_picks) in batches {
            let mut live: Vec<NodeId> = tree.to_nodes().iter().map(|node| node.id).collect();
            let mut patches = Vec::new();
            if remove_first && !live.is_empty() {
                let id = *remove_pick.get(&live);
                let removed = subtree_ids(&tree, id);
                live.retain(|id| !removed.contains(id));
                patches.push(Patch::Remove(id));
            }
            let mut first_failure = None;
            let room = per_update.saturating_sub(patches.len());
            for parent_pick in parent_picks.into_iter().take(room) {
                if live.len() >= max_nodes && first_failure.is_none() {
                    first_failure = Some(patches.len());
                }
                let parent = parent_pick
                    .filter(|_| !live.is_empty())
                    .map(|pick| *pick.get(&live));
                let id = next_id;
                next_id += 1;
                let node = Node { id, parent, props: Props::Flex(FlexProps::default()) };
                patches.push(insert(node, None));
                live.push(id);
            }

            let before = tree.clone();
            let result = tree.apply_patches(patches);
            match first_failure {
                Some(index) => {
                    prop_assert_eq!(
                        result,
                        Err(patch_error(index, TreeError::TreeTooLarge { max: max_nodes }))
                    );
                    prop_assert_eq!(&tree, &before);
                }
                None => {
                    prop_assert!(result.is_ok(), "{:?}", result);
                    prop_assert_eq!(tree.len(), live.len());
                }
            }
            prop_assert!(tree.len() <= max_nodes);
            check_invariants(&tree)?;
        }
    }

    /// Mounts and replaces are capped by the tree size, not the per-update cap (integration-1
    /// finding 11); list rows by both.
    #[test]
    fn mount_replace_and_row_limits(
        nodes in arb_nodes(40),
        per_update in 1usize..48,
        max_nodes in 1usize..48,
        existing in arb_nodes(8),
    ) {
        let limits = TreeLimits { max_nodes_per_update: per_update, max_nodes };
        let fits = nodes.len() <= max_nodes;

        let mounted = Tree::from_nodes(nodes.clone(), limits);
        if fits {
            prop_assert_eq!(mounted.map(|tree| tree.len()), Ok(nodes.len()));
        } else {
            prop_assert_eq!(mounted, Err(TreeError::TreeTooLarge { max: max_nodes }));
        }

        let mut existing = existing;
        existing.truncate(max_nodes);
        let mut tree = Tree::from_nodes(existing, limits)?;
        let before = tree.clone();
        let result = tree.apply_update(Update::Replace(nodes.clone()));
        if fits {
            let expected: BTreeSet<NodeId> = before
                .to_nodes()
                .iter()
                .chain(&nodes)
                .map(|node| node.id)
                .collect();
            prop_assert_eq!(result, Ok(expected));
            prop_assert_eq!(tree.len(), nodes.len());
        } else {
            prop_assert_eq!(result, Err(TreeError::TreeTooLarge { max: max_nodes }));
            prop_assert_eq!(&tree, &before);
        }

        let root_count = nodes.iter().filter(|node| node.parent.is_none()).count();
        let row = Tree::from_row(nodes.clone(), limits);
        if nodes.len() > per_update {
            prop_assert_eq!(row, Err(TreeError::UpdateTooLarge { count: nodes.len(), max: per_update }));
        } else if !fits {
            prop_assert_eq!(row, Err(TreeError::TreeTooLarge { max: max_nodes }));
        } else if root_count != 1 {
            prop_assert_eq!(row, Err(TreeError::RowRootCount(root_count)));
        } else {
            prop_assert_eq!(row.map(|row| row.roots().len()), Ok(1));
        }
    }
}

fn json_round_trip<T>(value: &T) -> Result<(), TestCaseError>
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let text = serde_json::to_string(value)?;
    let from_text: T = serde_json::from_str(&text)?;
    prop_assert_eq!(&from_text, value, "via {}", text);
    let json = serde_json::to_value(value)?;
    let from_value: T = serde_json::from_value(json)?;
    prop_assert_eq!(&from_value, value);
    Ok(())
}

fn arb_any_node() -> impl Strategy<Value = Node> {
    (
        any::<NodeId>(),
        prop::option::of(any::<NodeId>()),
        arb_props(),
    )
        .prop_map(|(id, parent, props)| Node { id, parent, props })
}

fn arb_patch() -> impl Strategy<Value = Patch> {
    prop_oneof![
        (any::<NodeId>(), arb_props()).prop_map(|(id, props)| Patch::SetProps(id, props)),
        (arb_any_node(), prop::option::of(any::<NodeId>()))
            .prop_map(|(node, before)| insert(node, before)),
        any::<NodeId>().prop_map(Patch::Remove),
    ]
}

fn arb_update() -> impl Strategy<Value = Update> {
    prop_oneof![
        Just(Update::Unchanged),
        prop::collection::vec(arb_any_node(), 0..6).prop_map(Update::Replace),
        prop::collection::vec(arb_patch(), 0..6).prop_map(Update::Patches),
    ]
}

fn arb_ui_event() -> impl Strategy<Value = UiEvent> {
    prop_oneof![
        any::<NodeId>().prop_map(UiEvent::Click),
        (any::<NodeId>(), arb_string()).prop_map(|(id, text)| UiEvent::InputChanged(id, text)),
        (any::<NodeId>(), arb_string()).prop_map(|(id, text)| UiEvent::InputSubmitted(id, text)),
        (any::<NodeId>(), any::<u32>()).prop_map(|(id, row)| UiEvent::RowClick(id, row)),
    ]
}

fn arb_surface() -> impl Strategy<Value = Surface> {
    let panel = (
        prop_oneof![
            Just(DockPosition::Left),
            Just(DockPosition::Right),
            Just(DockPosition::Bottom),
        ],
        arb_string(),
        any::<u32>(),
    )
        .prop_map(|(position, icon, default_size)| {
            SurfaceKind::Panel(PanelDescriptor {
                position,
                icon,
                default_size,
            })
        });
    let status_item = (
        prop_oneof![Just(StatusBarSide::Left), Just(StatusBarSide::Right)],
        any::<i32>(),
    )
        .prop_map(|(side, priority)| {
            SurfaceKind::StatusItem(StatusItemDescriptor { side, priority })
        });
    (arb_string(), arb_string(), prop_oneof![panel, status_item])
        .prop_map(|(id, title, kind)| Surface { id, title, kind })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn props_json_round_trips(props in arb_props()) {
        json_round_trip(&props)?;
        match &props {
            Props::Flex(inner) => json_round_trip(inner)?,
            Props::Text(inner) => json_round_trip(inner)?,
            Props::Button(inner) => {
                json_round_trip(&inner.style)?;
                json_round_trip(inner)?;
            }
            Props::Input(inner) => json_round_trip(inner)?,
            Props::List(inner) => json_round_trip(inner)?,
            Props::Table(inner) => json_round_trip(inner)?,
            Props::Icon(inner) => json_round_trip(inner)?,
            Props::Markdown(inner) => json_round_trip(inner)?,
            Props::DiffView(inner) => json_round_trip(inner)?,
        }
    }

    #[test]
    fn update_json_round_trips(update in arb_update()) {
        json_round_trip(&update)?;
        if let Update::Patches(patches) = &update {
            for patch in patches {
                json_round_trip(patch)?;
                if let Patch::Insert(op) = patch {
                    json_round_trip(op)?;
                    json_round_trip(&op.node)?;
                }
            }
        }
    }

    #[test]
    fn list_row_json_round_trips(
        index in any::<u32>(),
        nodes in prop::collection::vec(arb_any_node(), 0..6),
    ) {
        json_round_trip(&ListRow { index, nodes })?;
    }

    #[test]
    fn ui_event_json_round_trips(event in arb_ui_event()) {
        json_round_trip(&event)?;
    }

    #[test]
    fn surface_json_round_trips(surface in arb_surface()) {
        match &surface.kind {
            SurfaceKind::Panel(panel) => {
                json_round_trip(&panel.position)?;
                json_round_trip(panel)?;
            }
            SurfaceKind::StatusItem(item) => {
                json_round_trip(&item.side)?;
                json_round_trip(item)?;
            }
        }
        json_round_trip(&surface.kind)?;
        json_round_trip(&surface)?;
    }

    #[test]
    fn leaf_types_json_round_trip(
        color in arb_color(),
        flex in arb_flex_props(),
        level in prop_oneof![
            Just(LogLevel::Trace),
            Just(LogLevel::Debug),
            Just(LogLevel::Info),
            Just(LogLevel::Warn),
            Just(LogLevel::Error),
        ],
    ) {
        json_round_trip(&color)?;
        json_round_trip(&flex.direction)?;
        json_round_trip(&flex.align)?;
        json_round_trip(&flex)?;
        json_round_trip(&level)?;
    }
}
