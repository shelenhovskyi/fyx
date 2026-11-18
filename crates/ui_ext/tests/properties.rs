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
use ui_ext::{
    ApiError, ApiVersion, IdKind, IdLimits, LiveIds, MAX_DECORATION_PATH_BYTES, MAX_ID_BYTES,
    MAX_INDENT_LEVELS, MAX_TOOLTIP_BYTES, PathProblem, Placement, check_decoration_path, v0_2,
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

// fyx:ui 0.2: the model in `ui_ext::v0_2` and the version-gated rules in `ApiVersion` / `LiveIds`.

fn arb_icon() -> impl Strategy<Value = v0_2::Icon> {
    prop::sample::select(v0_2::Icon::ALL)
}

fn arb_icon_ref() -> impl Strategy<Value = v0_2::IconRef> {
    prop_oneof![
        arb_icon().prop_map(v0_2::IconRef::Frozen),
        // Includes spellings of frozen icons ("check"), which must stay upstream names.
        prop_oneof![
            arb_string(),
            arb_icon().prop_map(|icon| icon.name().to_owned())
        ]
        .prop_map(v0_2::IconRef::Upstream),
    ]
}

fn arb_button_style() -> impl Strategy<Value = ButtonStyle> {
    prop_oneof![
        Just(ButtonStyle::Default),
        Just(ButtonStyle::Filled),
        Just(ButtonStyle::Subtle),
        Just(ButtonStyle::Danger),
    ]
}

fn arb_v0_2_leaf_props() -> impl Strategy<Value = v0_2::Props> {
    let text = (
        arb_string(),
        prop::option::of(arb_color()),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(content, color, bold, monospace)| {
            v0_2::Props::Text(TextProps {
                content,
                color,
                bold,
                monospace,
            })
        });
    let button = (
        arb_string(),
        prop::option::of(arb_icon_ref()),
        arb_button_style(),
        any::<bool>(),
        prop::option::of(arb_string()),
    )
        .prop_map(|(label, icon, style, disabled, tooltip)| {
            v0_2::Props::Button(v0_2::ButtonProps {
                label,
                icon,
                style,
                disabled,
                tooltip,
            })
        });
    let input =
        (arb_string(), arb_string(), any::<bool>()).prop_map(|(value, placeholder, secret)| {
            v0_2::Props::Input(v0_2::InputProps {
                value,
                placeholder,
                secret,
            })
        });
    let list = (any::<u32>(), any::<u16>(), prop::option::of(any::<u32>())).prop_map(
        |(row_count, row_height, selected)| {
            v0_2::Props::List(v0_2::ListProps {
                row_count,
                row_height,
                selected,
            })
        },
    );
    let table = (prop::collection::vec(arb_string(), 0..4), any::<u32>())
        .prop_map(|(columns, row_count)| v0_2::Props::Table(TableProps { columns, row_count }));
    let icon = (arb_icon_ref(), prop::option::of(arb_color()))
        .prop_map(|(name, color)| v0_2::Props::Icon(v0_2::IconProps { name, color }));
    let diff_view = (arb_string(), arb_string(), prop::option::of(arb_string())).prop_map(
        |(old_text, new_text, language)| {
            v0_2::Props::DiffView(DiffViewProps {
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
        arb_string().prop_map(v0_2::Props::Markdown),
        diff_view,
    ]
}

fn arb_v0_2_props() -> impl Strategy<Value = v0_2::Props> {
    prop_oneof![
        3 => arb_flex_props().prop_map(v0_2::Props::Flex),
        4 => arb_v0_2_leaf_props(),
    ]
}

fn arb_v0_2_node() -> impl Strategy<Value = v0_2::Node> {
    (
        any::<NodeId>(),
        prop::option::of(any::<NodeId>()),
        arb_v0_2_props(),
    )
        .prop_map(|(id, parent, props)| v0_2::Node { id, parent, props })
}

fn arb_v0_2_patch() -> impl Strategy<Value = v0_2::Patch> {
    let rows = prop::option::of(
        (any::<u32>(), any::<u32>()).prop_map(|(start, count)| v0_2::RowRange { start, count }),
    );
    prop_oneof![
        (any::<NodeId>(), arb_v0_2_props())
            .prop_map(|(id, props)| v0_2::Patch::SetProps(id, props)),
        (arb_v0_2_node(), prop::option::of(any::<NodeId>()))
            .prop_map(|(node, before)| v0_2::Patch::Insert(v0_2::InsertOp { node, before })),
        any::<NodeId>().prop_map(v0_2::Patch::Remove),
        (any::<NodeId>(), rows).prop_map(|(target, rows)| {
            v0_2::Patch::InvalidateRows(v0_2::InvalidateRowsOp { target, rows })
        }),
    ]
}

fn arb_v0_2_update() -> impl Strategy<Value = v0_2::Update> {
    prop_oneof![
        Just(v0_2::Update::Unchanged),
        prop::collection::vec(arb_v0_2_node(), 0..6).prop_map(v0_2::Update::Replace),
        prop::collection::vec(arb_v0_2_patch(), 0..6).prop_map(v0_2::Update::Patches),
    ]
}

fn arb_dismiss_reason() -> impl Strategy<Value = v0_2::DismissReason> {
    prop_oneof![
        Just(v0_2::DismissReason::Escape),
        Just(v0_2::DismissReason::FocusLost),
        Just(v0_2::DismissReason::Replaced),
        Just(v0_2::DismissReason::WindowClosed),
    ]
}

fn arb_v0_2_ui_event() -> impl Strategy<Value = v0_2::UiEvent> {
    prop_oneof![
        arb_ui_event().prop_map(v0_2::UiEvent::from),
        (any::<NodeId>(), any::<u32>()).prop_map(|(id, row)| v0_2::UiEvent::RowSelected(id, row)),
        arb_dismiss_reason().prop_map(v0_2::UiEvent::Dismissed),
    ]
}

fn arb_dock_position() -> impl Strategy<Value = DockPosition> {
    prop_oneof![
        Just(DockPosition::Left),
        Just(DockPosition::Right),
        Just(DockPosition::Bottom),
    ]
}

fn arb_v0_2_surface_kind() -> impl Strategy<Value = v0_2::SurfaceKind> {
    let panel = (arb_dock_position(), arb_icon_ref(), any::<u32>()).prop_map(
        |(position, icon, default_size)| {
            v0_2::SurfaceKind::Panel(v0_2::PanelDescriptor {
                position,
                icon,
                default_size,
            })
        },
    );
    let status_item = (
        prop_oneof![Just(StatusBarSide::Left), Just(StatusBarSide::Right)],
        any::<i32>(),
    )
        .prop_map(|(side, priority)| {
            v0_2::SurfaceKind::StatusItem(StatusItemDescriptor { side, priority })
        });
    let modal = prop_oneof![
        Just(v0_2::ModalWidth::Small),
        Just(v0_2::ModalWidth::Medium),
        Just(v0_2::ModalWidth::Large),
    ]
    .prop_map(|width| v0_2::SurfaceKind::Modal(v0_2::ModalDescriptor { width }));
    prop_oneof![panel, status_item, modal]
}

fn arb_decoration_location() -> impl Strategy<Value = v0_2::DecorationLocation> {
    (
        arb_string(),
        any::<u32>(),
        prop_oneof![
            Just(v0_2::DecorationPlacement::Above),
            Just(v0_2::DecorationPlacement::Below),
        ],
        prop::option::of(any::<u16>()),
    )
        .prop_map(
            |(path, line, placement, height_lines)| v0_2::DecorationLocation {
                path,
                line,
                placement,
                height_lines,
            },
        )
}

fn arb_host_error() -> impl Strategy<Value = v0_2::HostError> {
    (
        prop_oneof![
            Just(v0_2::HostErrorCode::UnknownSurface),
            Just(v0_2::HostErrorCode::LimitExceeded),
            Just(v0_2::HostErrorCode::InvalidArgument),
            Just(v0_2::HostErrorCode::NotPermitted),
            Just(v0_2::HostErrorCode::Unavailable),
        ],
        arb_string(),
    )
        .prop_map(|(code, message)| v0_2::HostError { code, message })
}

/// The inverse of the 0.1 → 0.2 conversion, written out independently: `None` for anything 0.1
/// cannot express (a frozen icon, or a 0.2-only field that is not empty).
fn upstream_icon(icon: v0_2::IconRef) -> Option<String> {
    match icon {
        v0_2::IconRef::Upstream(name) => Some(name),
        v0_2::IconRef::Frozen(_) => None,
    }
}

fn props_to_v0_1(props: v0_2::Props) -> Option<Props> {
    Some(match props {
        v0_2::Props::Flex(props) => Props::Flex(props),
        v0_2::Props::Text(props) => Props::Text(props),
        v0_2::Props::Button(props) => {
            if props.tooltip.is_some() {
                return None;
            }
            Props::Button(ButtonProps {
                label: props.label,
                icon: match props.icon {
                    Some(icon) => Some(upstream_icon(icon)?),
                    None => None,
                },
                style: props.style,
                disabled: props.disabled,
            })
        }
        v0_2::Props::Input(props) => {
            if props.secret {
                return None;
            }
            Props::Input(InputProps {
                value: props.value,
                placeholder: props.placeholder,
            })
        }
        v0_2::Props::List(props) => {
            if props.selected.is_some() {
                return None;
            }
            Props::List(ListProps {
                row_count: props.row_count,
                row_height: props.row_height,
            })
        }
        v0_2::Props::Table(props) => Props::Table(props),
        v0_2::Props::Icon(props) => Props::Icon(IconProps {
            name: upstream_icon(props.name)?,
            color: props.color,
        }),
        v0_2::Props::Markdown(text) => Props::Markdown(text),
        v0_2::Props::DiffView(props) => Props::DiffView(props),
    })
}

fn node_to_v0_1(node: v0_2::Node) -> Option<Node> {
    Some(Node {
        id: node.id,
        parent: node.parent,
        props: props_to_v0_1(node.props)?,
    })
}

fn update_to_v0_1(update: v0_2::Update) -> Option<Update> {
    Some(match update {
        v0_2::Update::Unchanged => Update::Unchanged,
        v0_2::Update::Replace(nodes) => {
            Update::Replace(nodes.into_iter().map(node_to_v0_1).collect::<Option<_>>()?)
        }
        v0_2::Update::Patches(patches) => Update::Patches(
            patches
                .into_iter()
                .map(|patch| match patch {
                    v0_2::Patch::SetProps(id, props) => {
                        Some(Patch::SetProps(id, props_to_v0_1(props)?))
                    }
                    v0_2::Patch::Insert(op) => Some(insert(node_to_v0_1(op.node)?, op.before)),
                    v0_2::Patch::Remove(id) => Some(Patch::Remove(id)),
                    v0_2::Patch::InvalidateRows(_) => None,
                })
                .collect::<Option<_>>()?,
        ),
    })
}

fn surface_to_v0_1(surface: v0_2::Surface) -> Option<Surface> {
    let kind = match surface.kind {
        v0_2::SurfaceKind::Panel(panel) => SurfaceKind::Panel(PanelDescriptor {
            position: panel.position,
            icon: upstream_icon(panel.icon)?,
            default_size: panel.default_size,
        }),
        v0_2::SurfaceKind::StatusItem(item) => SurfaceKind::StatusItem(item),
        v0_2::SurfaceKind::Modal(_) => return None,
    };
    Some(Surface {
        id: surface.id,
        title: surface.title,
        kind,
    })
}

/// Written out here rather than calling `SurfaceKind::capability`, so the names are checked
/// against the manifest names the host uses too.
fn capability_of(kind: &v0_2::SurfaceKind) -> &'static str {
    match kind {
        v0_2::SurfaceKind::Panel(_) => "ui:panel",
        v0_2::SurfaceKind::StatusItem(_) => "ui:status-bar",
        v0_2::SurfaceKind::Modal(_) => "ui:modal",
    }
}

const CAPABILITIES: [&str; 4] = [
    "ui:panel",
    "ui:status-bar",
    "ui:modal",
    "ui:editor-decorations",
];

fn arb_capabilities() -> impl Strategy<Value = Vec<&'static str>> {
    prop_oneof![
        1 => Just(CAPABILITIES.to_vec()),
        2 => prop::sample::subsequence(CAPABILITIES.to_vec(), 0..=CAPABILITIES.len()),
    ]
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
}

/// Mostly allowed characters, so valid ids are common, with arbitrary characters mixed in.
fn arb_id() -> impl Strategy<Value = String> {
    let allowed: Vec<char> = ('a'..='z')
        .chain('0'..='9')
        .chain(['.', '_', '-'])
        .collect();
    prop::collection::vec(
        prop_oneof![
            16 => prop::sample::select(allowed),
            1 => any::<char>(),
        ],
        0..=MAX_ID_BYTES + 4,
    )
    .prop_map(String::from_iter)
}

fn arb_path_component() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 _.~-]{1,12}".prop_filter("a dot component", |component| {
        component != "." && component != ".."
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn v0_2_props_json_round_trips(props in arb_v0_2_props(), icon in arb_icon_ref()) {
        json_round_trip(&props)?;
        json_round_trip(&icon)?;
    }

    #[test]
    fn v0_2_update_json_round_trips(update in arb_v0_2_update()) {
        json_round_trip(&update)?;
        if let v0_2::Update::Patches(patches) = &update {
            for patch in patches {
                json_round_trip(patch)?;
            }
        }
    }

    #[test]
    fn v0_2_list_row_json_round_trips(
        index in any::<u32>(),
        indent in any::<u8>(),
        nodes in prop::collection::vec(arb_v0_2_node(), 0..6),
    ) {
        json_round_trip(&v0_2::ListRow { index, indent, nodes })?;
    }

    #[test]
    fn v0_2_host_side_types_json_round_trip(
        event in arb_v0_2_ui_event(),
        surface in (arb_string(), arb_string(), arb_v0_2_surface_kind())
            .prop_map(|(id, title, kind)| v0_2::Surface { id, title, kind }),
        location in arb_decoration_location(),
        error in arb_host_error(),
    ) {
        json_round_trip(&event)?;
        json_round_trip(&surface.kind)?;
        json_round_trip(&surface)?;
        json_round_trip(&location)?;
        json_round_trip(&error)?;
    }

    /// Every 0.1 value has a 0.2 counterpart with only upstream icons and empty 0.2 fields, and
    /// the conversion loses nothing.
    #[test]
    fn v0_1_values_convert_into_v0_2_losslessly(
        props in arb_props(),
        update in arb_update(),
    ) {
        prop_assert_eq!(props_to_v0_1(v0_2::Props::from(props.clone())), Some(props));
        prop_assert_eq!(update_to_v0_1(v0_2::Update::from(update.clone())), Some(update));
    }

    /// The rest of the 0.1 values, in a test of their own: with every strategy in one test, the
    /// generator overflowed the 1 MiB stack of a Windows debug test thread.
    #[test]
    fn v0_1_rows_events_surfaces_convert_into_v0_2_losslessly(
        row in (any::<u32>(), prop::collection::vec(arb_any_node(), 0..6))
            .prop_map(|(index, nodes)| ListRow { index, nodes }),
        event in arb_ui_event(),
        surface in arb_surface(),
    ) {
        let converted = v0_2::ListRow::from(row.clone());
        prop_assert_eq!(converted.indent, 0);
        prop_assert_eq!(converted.index, row.index);
        let nodes: Option<Vec<Node>> = converted.nodes.into_iter().map(node_to_v0_1).collect();
        prop_assert_eq!(nodes, Some(row.nodes));

        let converted = v0_2::UiEvent::from(event.clone());
        prop_assert_eq!(converted.target(), Some(event.target()));
        prop_assert_eq!(UiEvent::try_from(converted), Ok(event));

        prop_assert_eq!(surface_to_v0_1(v0_2::Surface::from(surface.clone())), Some(surface));
    }

    /// 0.1 static surfaces are checked exactly as the 0.1 host checks them: any id, duplicates
    /// included, no cap, only the capability of each kind.
    #[test]
    fn v0_1_static_surfaces_need_only_their_capabilities(
        surfaces in prop::collection::vec(arb_surface(), 0..8),
        capabilities in arb_capabilities(),
    ) {
        let converted: Vec<v0_2::Surface> = surfaces.into_iter().map(Into::into).collect();
        let permitted = converted
            .iter()
            .all(|surface| capabilities.contains(&capability_of(&surface.kind)));
        let result = LiveIds::new(ApiVersion::V0_1_0, &converted, &capabilities);
        prop_assert_eq!(result.is_ok(), permitted, "{:?}", result);
        if let Err(error) = result {
            let is_not_permitted = matches!(error, ApiError::NotPermitted { .. });
            prop_assert!(is_not_permitted);
        }
        for surface in &converted {
            prop_assert_eq!(ApiVersion::V0_1_0.check_surface_id(&surface.id), Ok(()));
        }
    }

    #[test]
    fn v0_2_surface_ids_follow_the_id_rule(id in arb_id()) {
        let expected = if is_valid_id(&id) {
            Ok(())
        } else {
            Err(ApiError::InvalidId { id: id.clone() })
        };
        prop_assert_eq!(ApiVersion::V0_2_0.check_surface_id(&id), expected);
        prop_assert_eq!(ApiVersion::V0_1_0.check_surface_id(&id), Ok(()));
    }

    #[test]
    fn decoration_paths(
        components in prop::collection::vec(arb_path_component(), 1..8),
        corruption in 0..6u8,
        position in any::<Index>(),
        forbidden in prop::sample::select(vec!['\\', ':', '\0']),
    ) {
        let path = components.join("/");
        prop_assert_eq!(check_decoration_path(&path), Ok(()), "{:?}", path);

        let (corrupted, problem) = match corruption {
            0 | 1 => {
                let mut components = components;
                let inserted = if corruption == 0 { ".." } else { "." };
                components.insert(position.index(components.len() + 1), inserted.to_owned());
                (components.join("/"), PathProblem::DotComponent)
            }
            2 => {
                let mut components = components;
                components.insert(position.index(components.len() + 1), String::new());
                (components.join("/"), PathProblem::EmptyComponent)
            }
            3 => (format!("/{path}"), PathProblem::EmptyComponent),
            4 => {
                // Every generated character is ASCII, so every byte index is a boundary.
                let mut corrupted = path.clone();
                corrupted.insert(position.index(path.len() + 1), forbidden);
                (corrupted, PathProblem::ForbiddenCharacter(forbidden))
            }
            _ => {
                let mut long = path.clone();
                while long.len() <= MAX_DECORATION_PATH_BYTES {
                    long.push('/');
                    long.push_str(&path);
                }
                (long, PathProblem::TooLong { max: MAX_DECORATION_PATH_BYTES })
            }
        };
        let result = check_decoration_path(&corrupted);
        prop_assert_eq!(result, Err(ApiError::InvalidPath { path: corrupted, problem }));
    }

    #[test]
    fn tooltips_are_truncated_to_the_longest_fitting_prefix(
        text in prop_oneof![
            prop::collection::vec(any::<char>(), 0..600).prop_map(String::from_iter),
            (0usize..1100, prop::collection::vec(any::<char>(), 0..4)).prop_map(|(len, tail)| {
                let mut text = "a".repeat(len);
                text.extend(tail);
                text
            }),
        ],
    ) {
        let shown = v0_2::truncate_tooltip(&text);
        prop_assert!(text.starts_with(shown));
        prop_assert!(shown.len() <= MAX_TOOLTIP_BYTES);
        if text.len() <= MAX_TOOLTIP_BYTES {
            prop_assert_eq!(shown, text.as_str());
        } else {
            let next = text[shown.len()..].chars().next().map_or(0, char::len_utf8);
            prop_assert!(shown.len() + next > MAX_TOOLTIP_BYTES);
        }
        let button = v0_2::ButtonProps { tooltip: Some(text.clone()), ..v0_2::ButtonProps::default() };
        prop_assert_eq!(button.tooltip_text(), Some(shown));
    }

    #[test]
    fn indent_is_clamped_to_the_drawn_maximum(index in any::<u32>(), indent in any::<u8>()) {
        let row = v0_2::ListRow { index, indent, nodes: Vec::new() };
        prop_assert_eq!(row.indent_levels(), indent.min(MAX_INDENT_LEVELS));
        prop_assert!(row.indent_levels() <= 32);
    }

    /// What `push-update` accepts at the call is exactly what the tree does not reject for size,
    /// so an accepted push can only fail later for fit, never for a cap.
    #[test]
    fn push_caps_agree_with_the_tree(
        limits in arb_limits(),
        existing in arb_nodes(8),
        update in prop_oneof![
            arb_update(),
            prop::collection::vec(arb_any_node(), 0..60).prop_map(Update::Replace),
            prop::collection::vec(arb_patch(), 0..16).prop_map(Update::Patches),
        ],
    ) {
        let mut existing = existing;
        existing.truncate(limits.max_nodes);
        let mut tree = Tree::from_nodes(existing, limits)?;
        let before = tree.clone();
        let caps = v0_2::Update::from(update.clone()).check_caps(limits);
        let result = tree.apply_update(update);
        match caps {
            Err(error) => {
                prop_assert_eq!(result, Err(error));
                prop_assert_eq!(&tree, &before);
            }
            Ok(()) => {
                let size_error = matches!(
                    result,
                    Err(TreeError::UpdateTooLarge { .. } | TreeError::TreeTooLarge { .. })
                );
                prop_assert!(!size_error, "{:?}", result);
            }
        }
    }
}

/// An `add-surface` / `remove-surface` / `place-decoration` / `remove-decoration` sequence picked
/// from a small id pool, so ids collide, run out and come back.
#[derive(Clone, Debug)]
enum IdsOp {
    AddSurface(Index, v0_2::SurfaceKind),
    RemoveSurface(Index),
    PlaceDecoration {
        id: Index,
        valid_path: bool,
        node_count: usize,
    },
    RemoveDecoration(Index),
}

const ID_POOL: [&str; 8] = ["a", "b", "c", "d", "e", "f", "Bad Id", ""];

fn arb_ids_op() -> impl Strategy<Value = IdsOp> {
    prop_oneof![
        3 => (any::<Index>(), arb_v0_2_surface_kind())
            .prop_map(|(id, kind)| IdsOp::AddSurface(id, kind)),
        1 => any::<Index>().prop_map(IdsOp::RemoveSurface),
        3 => (
            any::<Index>(),
            prop::bool::weighted(0.9),
            prop_oneof![9 => 0usize..=16384, 1 => Just(16385)],
        )
            .prop_map(|(id, valid_path, node_count)| IdsOp::PlaceDecoration {
                id,
                valid_path,
                node_count,
            }),
        1 => any::<Index>().prop_map(IdsOp::RemoveDecoration),
    ]
}

/// Reference model of the 0.2 rules, from design.md E and L, in the order the design lists them.
#[derive(Debug)]
struct IdsModel {
    limits: IdLimits,
    surfaces: BTreeMap<String, v0_2::SurfaceKind>,
    decorations: BTreeSet<String>,
}

impl IdsModel {
    fn in_use(&self, id: &str) -> Option<IdKind> {
        if self.surfaces.contains_key(id) {
            Some(IdKind::Surface)
        } else if self.decorations.contains(id) {
            Some(IdKind::Decoration)
        } else {
            None
        }
    }

    fn add_surface(
        &mut self,
        surface: &v0_2::Surface,
        capabilities: &[&str],
    ) -> Result<(), ApiError> {
        let id = surface.id.clone();
        if !is_valid_id(&id) {
            return Err(ApiError::InvalidId { id });
        }
        if let Some(kind) = self.in_use(&id) {
            return Err(ApiError::IdInUse { id, kind });
        }
        let capability = capability_of(&surface.kind);
        if !capabilities.contains(&capability) {
            return Err(ApiError::NotPermitted {
                what: format!("surface {id:?}"),
                capability,
            });
        }
        if self.surfaces.len() >= self.limits.max_surfaces {
            return Err(ApiError::TooManySurfaces {
                max: self.limits.max_surfaces,
            });
        }
        self.surfaces.insert(id, surface.kind.clone());
        Ok(())
    }

    fn decorations_permitted(capabilities: &[&str]) -> Result<(), ApiError> {
        if capabilities.contains(&"ui:editor-decorations") {
            Ok(())
        } else {
            Err(ApiError::NotPermitted {
                what: "editor decorations".to_owned(),
                capability: "ui:editor-decorations",
            })
        }
    }

    fn place(
        &mut self,
        id: &str,
        path: &str,
        node_count: usize,
        capabilities: &[&str],
    ) -> Result<Placement, ApiError> {
        Self::decorations_permitted(capabilities)?;
        if !is_valid_id(id) {
            return Err(ApiError::InvalidId { id: id.to_owned() });
        }
        if self.surfaces.contains_key(id) {
            return Err(ApiError::IdInUse {
                id: id.to_owned(),
                kind: IdKind::Surface,
            });
        }
        check_decoration_path(path)?;
        if node_count > 16384 {
            return Err(ApiError::TooManyNodes {
                count: node_count,
                max: 16384,
            });
        }
        if self.decorations.contains(id) {
            return Ok(Placement::Moved);
        }
        if self.decorations.len() >= self.limits.max_decorations {
            return Err(ApiError::TooManyDecorations {
                max: self.limits.max_decorations,
            });
        }
        self.decorations.insert(id.to_owned());
        Ok(Placement::New)
    }
}

fn check_ids_match_model(ids: &LiveIds, model: &IdsModel) -> Result<(), TestCaseError> {
    let surfaces: BTreeMap<String, v0_2::SurfaceKind> = ids
        .surfaces()
        .map(|(id, kind)| (id.to_owned(), kind.clone()))
        .collect();
    prop_assert_eq!(&surfaces, &model.surfaces);
    let decorations: BTreeSet<String> = ids.decorations().map(str::to_owned).collect();
    prop_assert_eq!(&decorations, &model.decorations);
    prop_assert!(surfaces.len() <= ids.limits().max_surfaces);
    prop_assert!(decorations.len() <= ids.limits().max_decorations);
    for id in surfaces.keys() {
        prop_assert!(
            !decorations.contains(id),
            "{:?} is a surface and a decoration",
            id
        );
        prop_assert!(is_valid_id(id), "live id {:?} breaks the id rule", id);
    }
    for id in &decorations {
        prop_assert!(is_valid_id(id), "live id {:?} breaks the id rule", id);
        prop_assert!(ids.is_decoration(id));
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Random 0.2 id operations against the reference model: same results, same live sets,
    /// surfaces and decorations never share an id, and neither cap is ever passed.
    #[test]
    fn live_ids_match_reference_model(
        max_surfaces in 0usize..5,
        max_decorations in 0usize..5,
        static_surfaces in prop::collection::vec((any::<Index>(), arb_v0_2_surface_kind()), 0..5),
        capabilities in arb_capabilities(),
        ops in prop::collection::vec(arb_ids_op(), 0..40),
    ) {
        let limits = IdLimits { max_surfaces, max_decorations };
        let pick = |index: &Index| *index.get(&ID_POOL);
        let surface = |id: &Index, kind: &v0_2::SurfaceKind| v0_2::Surface {
            id: pick(id).to_owned(),
            title: String::new(),
            kind: kind.clone(),
        };
        let static_surfaces: Vec<v0_2::Surface> =
            static_surfaces.iter().map(|(id, kind)| surface(id, kind)).collect();

        let mut model = IdsModel {
            limits,
            surfaces: BTreeMap::new(),
            decorations: BTreeSet::new(),
        };
        let expected = static_surfaces
            .iter()
            .try_for_each(|surface| model.add_surface(surface, &capabilities));
        let created =
            LiveIds::with_limits(ApiVersion::V0_2_0, limits, &static_surfaces, &capabilities);
        let mut ids = match (created, expected) {
            (Ok(ids), Ok(())) => ids,
            (Err(actual), Err(expected)) => {
                prop_assert_eq!(actual, expected);
                return Ok(());
            }
            (created, expected) => {
                return Err(TestCaseError::fail(format!(
                    "static set: LiveIds returned {created:?}, model {expected:?}"
                )));
            }
        };
        check_ids_match_model(&ids, &model)?;

        for op in &ops {
            match op {
                IdsOp::AddSurface(id, kind) => {
                    let surface = surface(id, kind);
                    let expected = model.add_surface(&surface, &capabilities);
                    prop_assert_eq!(ids.add_surface(&surface, &capabilities), expected);
                }
                IdsOp::RemoveSurface(id) => {
                    let id = pick(id);
                    let expected = if model.decorations.contains(id) {
                        Err(ApiError::NotASurface { id: id.to_owned() })
                    } else {
                        model
                            .surfaces
                            .remove(id)
                            .ok_or_else(|| ApiError::UnknownSurface { id: id.to_owned() })
                    };
                    prop_assert_eq!(ids.remove_surface(id), expected);
                }
                IdsOp::PlaceDecoration { id, valid_path, node_count } => {
                    let id = pick(id);
                    let path = if *valid_path { "src/lib.rs" } else { "../outside.rs" };
                    let location = v0_2::DecorationLocation {
                        path: path.to_owned(),
                        line: 0,
                        placement: v0_2::DecorationPlacement::Below,
                        height_lines: None,
                    };
                    let expected = model.place(id, path, *node_count, &capabilities);
                    prop_assert_eq!(
                        ids.place_decoration(id, &location, *node_count, &capabilities),
                        expected
                    );
                }
                IdsOp::RemoveDecoration(id) => {
                    let id = pick(id);
                    let expected = IdsModel::decorations_permitted(&capabilities).and_then(|()| {
                        if model.decorations.remove(id) {
                            Ok(())
                        } else {
                            Err(ApiError::UnknownDecoration { id: id.to_owned() })
                        }
                    });
                    prop_assert_eq!(ids.remove_decoration(id, &capabilities), expected);
                }
            }
            check_ids_match_model(&ids, &model)?;
        }
    }
}
