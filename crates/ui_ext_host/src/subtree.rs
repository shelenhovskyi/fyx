//! Plan 7.7 (perf-1 finding 1): a large surface is drawn in cached chunks, so a frame caused by one
//! input's editor (a keystroke, a caret move) re-renders that input's chunk, not every node.
//!
//! A chunk is a run of children of a column whose width is definite (the panel body, or a column
//! stretched across it). GPUI lays a cached view out from its style alone, without measuring the
//! content (`gpui::view::request_layout_view`), so a chunk is drawn uncached until it has measured
//! its own height, and cached at that height after. A chunk only holds nodes whose size depends on
//! the tree and the width alone: flexes without `grow`, texts, buttons, inputs and still icons.
//! Lists, tables, markdown, diffs, images, canvases, terminals and spinners load, scroll or move on
//! their own and are drawn with the surface as before.

use std::sync::Arc;

use collections::HashMap;
use gpui::{
    AnyElement, AnyView, App, Context, Entity, IntoElement, ParentElement as _, Pixels, Render,
    StyleRefinement, Styled as _, WeakEntity, Window, canvas, px,
};
use ui::v_flex;
use ui_ext::v0_2::{Align, FlexDirection, NodeId, Props, tree::Tree};

use crate::view::SurfaceView;

/// The most nodes a chunk holds, and the smallest subtree whose children are chunked.
pub(crate) const CHUNK_NODES: usize = 256;
/// A shorter run is drawn with the surface: a chunk of a few nodes saves nothing.
const MIN_CHUNK_NODES: usize = 8;

/// One child of a chunked column, as it is drawn.
#[derive(Clone)]
pub(crate) enum Segment {
    Node(NodeId),
    Chunk(Entity<SubtreeView>),
}

/// A chunk as planned from the tree, before it has an entity.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PlannedSegment {
    Node(NodeId),
    Chunk(Vec<NodeId>),
}

/// The children of each chunked column (`None` is the panel body), with its gap and align.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ColumnPlan {
    pub(crate) column: Option<NodeId>,
    pub(crate) gap: u16,
    pub(crate) align: Align,
    pub(crate) segments: Vec<PlannedSegment>,
}

#[cfg(test)]
thread_local! {
    /// Benchmarks compare a frame with and without chunks.
    pub(crate) static DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Which columns of `tree` are chunked, and how. Empty for a tree of at most [`CHUNK_NODES`].
pub(crate) fn plan(tree: &Tree) -> Vec<ColumnPlan> {
    let mut plans = Vec::new();
    #[cfg(test)]
    if DISABLED.get() {
        return plans;
    }
    if tree.len() <= CHUNK_NODES {
        return plans;
    }
    let sizes = subtree_sizes(tree);
    plan_column(
        tree,
        &sizes,
        None,
        0,
        Align::Stretch,
        tree.roots(),
        &mut plans,
    );
    plans
}

/// A node's subtree, as planning sees it.
#[derive(Clone, Copy)]
struct Subtree {
    nodes: usize,
    /// Every node in it may be cached.
    cacheable: bool,
    /// It holds an input, whose every keystroke renders the chunk it is in.
    has_input: bool,
}

/// Per node: its subtree's size and contents.
fn subtree_sizes(tree: &Tree) -> HashMap<NodeId, Subtree> {
    let mut sizes: HashMap<NodeId, Subtree> = HashMap::default();
    // Post-order without recursion: a node is summed once all its children are.
    let mut stack: Vec<(NodeId, bool)> = tree.roots().iter().map(|&id| (id, false)).collect();
    while let Some((id, children_done)) = stack.pop() {
        let Some(node) = tree.get(id) else {
            continue;
        };
        if !children_done {
            stack.push((id, true));
            stack.extend(node.children().iter().map(|&child| (child, false)));
            continue;
        }
        let own = Subtree {
            nodes: 1,
            cacheable: cacheable_kind(node.props()),
            has_input: matches!(node.props(), Props::Input(_)),
        };
        let subtree = node
            .children()
            .iter()
            .filter_map(|child| sizes.get(child))
            .fold(own, |subtree, child| Subtree {
                nodes: subtree.nodes + child.nodes,
                cacheable: subtree.cacheable && child.cacheable,
                has_input: subtree.has_input || child.has_input,
            });
        sizes.insert(id, subtree);
    }
    sizes
}

/// Nodes whose size and look follow from the tree and the width alone.
fn cacheable_kind(props: &Props) -> bool {
    match props {
        Props::Flex(_) | Props::Text(_) | Props::Button(_) | Props::Input(_) => true,
        Props::Icon(icon) => !matches!(
            crate::icons::resolve_icon(&icon.name),
            crate::icons::ResolvedIcon::Spinner
        ),
        Props::List(_)
        | Props::Table(_)
        | Props::Markdown(_)
        | Props::DiffView(_)
        | Props::Image(_)
        | Props::Canvas(_)
        | Props::Terminal(_) => false,
    }
}

fn plan_column(
    tree: &Tree,
    sizes: &HashMap<NodeId, Subtree>,
    column: Option<NodeId>,
    gap: u16,
    align: Align,
    children: &[NodeId],
    plans: &mut Vec<ColumnPlan>,
) {
    let size = |id: &NodeId| {
        sizes.get(id).copied().unwrap_or(Subtree {
            nodes: 0,
            cacheable: false,
            has_input: false,
        })
    };
    let total: usize = children.iter().map(|id| size(id).nodes).sum();
    if total <= CHUNK_NODES {
        return;
    }
    let mut segments = Vec::new();
    let mut run: Vec<NodeId> = Vec::new();
    let mut run_nodes = 0;
    let flush = |run: &mut Vec<NodeId>, run_nodes: &mut usize, segments: &mut Vec<_>| {
        if *run_nodes >= MIN_CHUNK_NODES {
            segments.push(PlannedSegment::Chunk(std::mem::take(run)));
        } else {
            segments.extend(run.drain(..).map(PlannedSegment::Node));
        }
        *run_nodes = 0;
    };
    for &child in children {
        let Some(node) = tree.get(child) else {
            continue;
        };
        let subtree = size(&child);
        let count = subtree.nodes;
        let grows = matches!(node.props(), Props::Flex(flex) if flex.grow);
        if subtree.cacheable && !grows && count <= CHUNK_NODES && subtree.has_input {
            // Typing renders the chunk an input is in: it gets one of its own.
            flush(&mut run, &mut run_nodes, &mut segments);
            segments.push(PlannedSegment::Chunk(vec![child]));
            continue;
        }
        if subtree.cacheable && !grows && count <= CHUNK_NODES {
            if run_nodes + count > CHUNK_NODES {
                flush(&mut run, &mut run_nodes, &mut segments);
            }
            run.push(child);
            run_nodes += count;
            continue;
        }
        flush(&mut run, &mut run_nodes, &mut segments);
        segments.push(PlannedSegment::Node(child));
        // A column stretched across this one has a definite width too; one aligned to a side is
        // as wide as its content, which a cached chunk inside it could not give.
        if let Props::Flex(flex) = node.props()
            && flex.direction == FlexDirection::Column
            && align == Align::Stretch
        {
            plan_column(
                tree,
                sizes,
                Some(child),
                flex.gap,
                flex.align,
                node.children(),
                plans,
            );
        }
    }
    flush(&mut run, &mut run_nodes, &mut segments);
    if segments
        .iter()
        .any(|segment| matches!(segment, PlannedSegment::Chunk(_)))
    {
        plans.push(ColumnPlan {
            column,
            gap,
            align,
            segments,
        });
    }
}

/// What a chunk drew from: every node of its subtrees in pre-order, with its props and number of
/// children. Holding the props keeps their pointers unique while compared.
#[derive(Default)]
pub(crate) struct Snapshot(Vec<(NodeId, Arc<Props>, usize)>);

impl Snapshot {
    pub(crate) fn of(tree: &Tree, members: &[NodeId]) -> Self {
        let mut nodes = Vec::new();
        let mut stack: Vec<NodeId> = members.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = tree.get(id) else {
                continue;
            };
            nodes.push((id, node.props_arc().clone(), node.children().len()));
            stack.extend(node.children().iter().rev());
        }
        Self(nodes)
    }

    fn same_as(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(
                |((id, props, children), (other_id, other_props, other_children))| {
                    id == other_id && children == other_children && Arc::ptr_eq(props, other_props)
                },
            )
    }
}

/// A cached run of a column's children. It renders through its surface's view, with the same
/// renderer, so listeners, focus and element ids stay the surface's.
pub(crate) struct SubtreeView {
    surface: WeakEntity<SurfaceView>,
    members: Vec<NodeId>,
    gap: u16,
    align: Align,
    snapshot: Snapshot,
    /// The content's height at the width it was last drawn in; `None` until drawn uncached once.
    measured: Option<Pixels>,
    #[cfg(test)]
    pub(crate) renders: usize,
}

impl SubtreeView {
    pub(crate) fn new(
        surface: WeakEntity<SurfaceView>,
        members: Vec<NodeId>,
        gap: u16,
        align: Align,
        snapshot: Snapshot,
    ) -> Self {
        Self {
            surface,
            members,
            gap,
            align,
            snapshot,
            measured: None,
            #[cfg(test)]
            renders: 0,
        }
    }

    /// Takes the chunk's new place in the tree. A change of anything it drew is drawn uncached
    /// and measured again.
    pub(crate) fn update_plan(
        &mut self,
        members: Vec<NodeId>,
        gap: u16,
        align: Align,
        snapshot: Snapshot,
        cx: &mut Context<Self>,
    ) {
        if self.members == members
            && self.gap == gap
            && self.align == align
            && self.snapshot.same_as(&snapshot)
        {
            return;
        }
        self.members = members;
        self.gap = gap;
        self.align = align;
        self.snapshot = snapshot;
        self.measured = None;
        cx.notify();
    }

    /// The chunk as a child of its column: cached at its measured height once it has one.
    pub(crate) fn element(entity: &Entity<Self>, cx: &App) -> AnyElement {
        let view = AnyView::from(entity.clone());
        match entity.read(cx).measured {
            Some(height) => view
                .cached(StyleRefinement::default().w_full().h(height).flex_none())
                .into_any_element(),
            None => view.into_any_element(),
        }
    }

    #[cfg(test)]
    pub(crate) fn measured(&self) -> Option<Pixels> {
        self.measured
    }
}

impl Render for SubtreeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let started = std::time::Instant::now();
        let members = self.members.clone();
        let content = self
            .surface
            .update(cx, |surface, cx| {
                let content = surface.render_members(&members, window, cx);
                surface
                    .state()
                    .read(cx)
                    .add_to_last_render(started.elapsed());
                content
            })
            .unwrap_or_default();
        let column = v_flex().w_full().relative().gap(px(self.gap.into()));
        let column = match self.align {
            Align::Start => column.items_start(),
            Align::Center => column.items_center(),
            Align::End => column.items_end(),
            Align::Stretch => column.items_stretch(),
        };
        let chunk = cx.entity().downgrade();
        column.children(content).child(
            // Measures the content: a cached chunk is laid out from its style alone.
            canvas(
                move |bounds, _, cx| {
                    let height = bounds.size.height;
                    let remeasured = chunk
                        .update(cx, |chunk, _| {
                            let previous = chunk.measured.replace(height);
                            previous.is_some_and(|previous| previous != height)
                        })
                        .unwrap_or(false);
                    // A cached chunk drawn at a new width whose height changed: its column lays
                    // out again with the new height on the next frame. Notifying inside a frame
                    // would be dropped.
                    if remeasured {
                        cx.defer(move |cx| {
                            chunk.update(cx, |_, cx| cx.notify()).ok();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
    }
}
