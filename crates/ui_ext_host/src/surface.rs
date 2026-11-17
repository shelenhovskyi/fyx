//! The retained state of one extension surface, shared by every view that shows it (a panel in
//! each workspace window, or a status item in each status bar).
//!
//! All extension calls start here and run on tasks; views only read this state while rendering.

use std::{collections::VecDeque, ops::Range, rc::Rc};

use collections::HashMap;
use gpui::{App, Context, SharedString, Task};
use ui_ext::{NodeId, Patch, Props, Surface, Tree, UiEvent, Update};

use crate::backend::{PushedUpdate, Reply, UiBackend};

/// Delivers a push that a call on one surface made for another surface of the same extension.
pub type PushRouter = Rc<dyn Fn(PushedUpdate, &mut App)>;

/// Cached rows above this count are pruned to the neighbourhood of the latest request.
const MAX_CACHED_ROWS: usize = 1024;
/// Rows fetched beyond the visible range, so short scrolls do not show placeholders.
pub(crate) const ROW_OVERSCAN: u32 = 8;

pub enum Phase {
    Mounting,
    Ready(Tree),
    Failed(SharedString),
}

/// The rows of one virtualized list or table node. Rows are subtrees with row-local node ids, so
/// they are kept apart from the surface tree and addressed by `(list node, index)`.
#[derive(Default)]
struct RowCache {
    /// Bumped whenever the list's props change; rows from older generations are still shown
    /// until their replacement arrives, which avoids flicker.
    generation: u64,
    rows: HashMap<u32, CachedRow>,
    in_flight: Option<Range<u32>>,
    /// The latest range asked for while a fetch was in flight.
    queued: Option<Range<u32>>,
    /// A fetch for this generation failed; do not retry until the list changes.
    failed_generation: Option<u64>,
    /// The last range answered in this generation. Rows the extension left out of it are not
    /// asked for again, so a short answer cannot cause a fetch on every frame.
    answered: Option<(u64, Range<u32>)>,
}

struct CachedRow {
    generation: u64,
    tree: Tree,
}

pub struct SurfaceState {
    backend: Rc<dyn UiBackend>,
    surface: Surface,
    route_push: Option<PushRouter>,
    phase: Phase,
    last_error: Option<SharedString>,
    lists: HashMap<NodeId, RowCache>,
    events: VecDeque<UiEvent>,
    event_worker_running: bool,
    /// How many times the extension has set each input's `value` (`set-props`, insert, replace).
    /// A view writes the value into its editor when this moves past what it last saw; otherwise
    /// the user's text stands (integrator decision 6). A counter, not a flag, so every window's
    /// view sees each set.
    input_sets: HashMap<NodeId, u64>,
    /// Bumped on every change to the tree, so views know when to reconcile their editors.
    revision: u64,
    resyncing: bool,
    _mount: Option<Task<()>>,
}

impl SurfaceState {
    pub fn new(
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        route_push: Option<PushRouter>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            backend,
            surface,
            route_push,
            phase: Phase::Mounting,
            last_error: None,
            lists: HashMap::default(),
            events: VecDeque::new(),
            event_worker_running: false,
            input_sets: HashMap::default(),
            revision: 0,
            resyncing: false,
            _mount: None,
        };
        this.mount(cx);
        this
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn extension_id(&self) -> &str {
        self.backend.extension_id()
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn tree(&self) -> Option<&Tree> {
        match &self.phase {
            Phase::Ready(tree) => Some(tree),
            Phase::Mounting | Phase::Failed(_) => None,
        }
    }

    pub fn last_error(&self) -> Option<&SharedString> {
        self.last_error.as_ref()
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        if self.last_error.take().is_some() {
            cx.notify();
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Asks the extension for the whole tree again, discarding the retained one.
    pub fn mount(&mut self, cx: &mut Context<Self>) {
        let call = self.backend.mount(&self.surface.id);
        self._mount = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.resyncing = false;
                match result {
                    Ok(Reply { pushed, value }) => {
                        // Pushes for this surface made during `mount` are superseded by the tree
                        // it returns; pushes for other surfaces still apply.
                        let others = pushed
                            .into_iter()
                            .filter(|push| push.surface_id != this.surface.id)
                            .collect();
                        this.apply_pushes(others, cx);
                        this.set_tree(value);
                    }
                    Err(error) => {
                        log::error!(
                            "fyx:ui {}/{}: mount failed: {error:#}",
                            this.extension_id(),
                            this.surface.id
                        );
                        this.phase = Phase::Failed(format!("{error:#}").into());
                        this.revision += 1;
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn set_tree(&mut self, tree: Tree) {
        count_all_inputs(&tree, &mut self.input_sets);
        self.phase = Phase::Ready(tree);
        self.lists.clear();
        self.revision += 1;
    }

    /// Queues one input event. Events reach the extension one at a time, in order, and each makes
    /// exactly one `handle-event` call; a queued `input-changed` for the same node is replaced by
    /// the newer one instead of making a second call.
    pub fn dispatch(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        let Some(tree) = self.tree() else {
            return;
        };
        if !tree.contains(event.target()) {
            log::debug!("fyx:ui: dropping {event:?}, its node is gone");
            return;
        }
        if let UiEvent::InputChanged(id, value) | UiEvent::InputSubmitted(id, value) = &event {
            self.set_input_locally(*id, value);
        }
        if let UiEvent::InputChanged(id, _) = &event
            && let Some(UiEvent::InputChanged(queued_id, _)) = self.events.back()
            && queued_id == id
        {
            self.events.pop_back();
        }
        self.events.push_back(event);
        if !self.event_worker_running {
            self.event_worker_running = true;
            cx.spawn(async move |this, cx| {
                loop {
                    let next = this.update(cx, |this, _| {
                        let event = this.events.pop_front();
                        if event.is_none() {
                            this.event_worker_running = false;
                        }
                        event.map(|event| {
                            let call = this.backend.handle_event(&this.surface.id, event.clone());
                            (event, call)
                        })
                    });
                    let Ok(Some((event, call))) = next else {
                        break;
                    };
                    let result = call.await;
                    if this
                        .update(cx, |this, cx| this.apply_reply(&event, result, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
    }

    /// The host tree follows what the user typed (integrator decision 6), so it matches the text
    /// on screen. Not counted as a set by the extension: the editor already shows it.
    fn set_input_locally(&mut self, id: NodeId, value: &str) {
        let Phase::Ready(tree) = &mut self.phase else {
            return;
        };
        let Some(Props::Input(props)) = tree.get(id).map(|node| node.props()) else {
            return;
        };
        if props.value == value {
            return;
        }
        let mut props = props.clone();
        props.value = value.to_string();
        if let Err(error) = tree.apply_patches(vec![Patch::SetProps(id, Props::Input(props))]) {
            log::error!("fyx:ui: failed to record the text of input {id}: {error}");
        }
    }

    fn apply_reply(
        &mut self,
        event: &UiEvent,
        result: anyhow::Result<Reply<Update>>,
        cx: &mut Context<Self>,
    ) {
        match result {
            // Pushes made during the call first, the reply last.
            Ok(Reply { pushed, value }) => {
                if self.apply_pushes(pushed, cx) {
                    self.apply_update(value, cx);
                }
            }
            Err(error) => {
                log::error!(
                    "fyx:ui {}/{}: handle-event({event:?}) failed: {error:#}",
                    self.extension_id(),
                    self.surface.id
                );
                self.last_error = Some(format!("{error:#}").into());
                cx.notify();
            }
        }
    }

    /// Applies pushes in order: this surface's here, other surfaces' through the router. Returns
    /// false once one of this surface's fails, since the surface is then being mounted again and
    /// later updates no longer fit.
    fn apply_pushes(&mut self, pushed: Vec<PushedUpdate>, cx: &mut Context<Self>) -> bool {
        for push in pushed {
            if push.surface_id == self.surface.id {
                if !self.apply_update(push.update, cx) {
                    return false;
                }
            } else if let Some(route_push) = self.route_push.clone() {
                route_push(push, cx);
            } else {
                log::error!(
                    "fyx:ui {}: dropping a push for surface {:?}, nothing routes it",
                    self.backend.extension_id(),
                    push.surface_id
                );
            }
        }
        true
    }

    /// Applies an update from an event reply or a push. An update that does not fit the retained
    /// tree means host and extension disagree, so it is logged and the surface is mounted again
    /// (the SDK resets its mirror on mount). Returns whether the update applied.
    pub fn apply_update(&mut self, update: Update, cx: &mut Context<Self>) -> bool {
        let Phase::Ready(tree) = &mut self.phase else {
            log::debug!(
                "fyx:ui {}/{}: dropping an update that arrived before mount finished",
                self.backend.extension_id(),
                self.surface.id
            );
            return false;
        };
        match tree.apply_update(update) {
            Ok(touched) if touched.is_empty() => true,
            Ok(touched) => {
                let tree = &*tree;
                for id in touched {
                    match tree.get(id).map(|node| node.props()) {
                        // The WIT has no "rows changed" patch: a touched list (its row count,
                        // usually) has its rows fetched again, once, for the visible range.
                        Some(Props::List(_) | Props::Table(_)) => {
                            if let Some(cache) = self.lists.get_mut(&id) {
                                cache.invalidate();
                            }
                        }
                        // Set, inserted or replaced by the extension: the views write the value.
                        Some(Props::Input(_)) => *self.input_sets.entry(id).or_default() += 1,
                        _ => {}
                    }
                }
                self.lists.retain(|id, _| {
                    tree.get(*id).is_some_and(|node| {
                        matches!(node.props(), Props::List(_) | Props::Table(_))
                    })
                });
                self.input_sets.retain(|id, _| tree.contains(*id));
                self.revision += 1;
                cx.notify();
                true
            }
            Err(error) => {
                log::error!(
                    "fyx:ui {}/{}: invalid update, mounting again: {error}",
                    self.backend.extension_id(),
                    self.surface.id
                );
                self.last_error =
                    Some(format!("Invalid update from the extension: {error}").into());
                if !self.resyncing {
                    self.resyncing = true;
                    self.mount(cx);
                }
                cx.notify();
                false
            }
        }
    }

    /// How many times the extension has set this input's value; see `input_sets`.
    pub fn input_set_count(&self, id: NodeId) -> u64 {
        self.input_sets.get(&id).copied().unwrap_or_default()
    }

    /// A cached row, possibly from before the list last changed.
    pub fn row(&self, list: NodeId, index: u32) -> Option<&Tree> {
        self.lists.get(&list)?.rows.get(&index).map(|row| &row.tree)
    }

    /// Makes sure rows `range` of `list` are fetched, without waiting for them. Cheap when they
    /// are cached, so views call it for the visible range on every layout.
    pub fn request_rows(&mut self, list: NodeId, range: Range<u32>, cx: &mut Context<Self>) {
        if range.is_empty() || self.tree().is_none_or(|tree| !tree.contains(list)) {
            return;
        }
        let cache = self.lists.entry(list).or_default();
        if cache.failed_generation == Some(cache.generation) {
            return;
        }
        let generation = cache.generation;
        if cache
            .answered
            .as_ref()
            .is_some_and(|(answered_generation, answered)| {
                *answered_generation == generation
                    && answered.start <= range.start
                    && range.end <= answered.end
            })
        {
            return;
        }
        let missing = range.clone().any(|index| {
            cache
                .rows
                .get(&index)
                .is_none_or(|row| row.generation != generation)
        });
        if !missing {
            return;
        }
        if cache.in_flight.is_some() {
            cache.queued = Some(range);
            return;
        }
        cache.in_flight = Some(range.clone());
        let call = self
            .backend
            .list_rows(&self.surface.id, list, range.start, range.len() as u32);
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.receive_rows(list, generation, range, result, cx)
            })
            .ok();
        })
        .detach();
    }

    fn receive_rows(
        &mut self,
        list: NodeId,
        generation: u64,
        range: Range<u32>,
        result: anyhow::Result<Reply<Vec<(u32, Tree)>>>,
        cx: &mut Context<Self>,
    ) {
        let result = result.map(|Reply { pushed, value }| {
            // A push that touches this list bumps its generation, so these rows are stored as
            // stale and fetched again.
            self.apply_pushes(pushed, cx);
            value
        });
        let Some(cache) = self.lists.get_mut(&list) else {
            return;
        };
        cache.in_flight = None;
        cache.answered = Some((generation, range.clone()));
        match result {
            Ok(rows) => {
                for (index, tree) in rows {
                    cache.rows.insert(index, CachedRow { generation, tree });
                }
                if cache.rows.len() > MAX_CACHED_ROWS {
                    let keep = range.start.saturating_sub(MAX_CACHED_ROWS as u32 / 4)
                        ..range.end.saturating_add(MAX_CACHED_ROWS as u32 / 4);
                    cache.rows.retain(|index, _| keep.contains(index));
                }
            }
            Err(error) => {
                log::error!(
                    "fyx:ui {}/{}: list-rows({list}, {range:?}) failed: {error:#}",
                    self.backend.extension_id(),
                    self.surface.id
                );
                cache.failed_generation = Some(generation);
                self.last_error = Some(format!("{error:#}").into());
            }
        }
        if let Some(queued) = self
            .lists
            .get_mut(&list)
            .and_then(|cache| cache.queued.take())
        {
            self.request_rows(list, queued, cx);
        }
        cx.notify();
    }
}

impl RowCache {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.failed_generation = None;
    }
}

fn count_all_inputs(tree: &Tree, input_sets: &mut HashMap<NodeId, u64>) {
    for node in tree.to_nodes() {
        if let Props::Input(_) = node.props {
            *input_sets.entry(node.id).or_default() += 1;
        }
    }
}

/// `visible` widened by the overscan and clamped to the row count.
pub(crate) fn fetch_range(visible: Range<usize>, row_count: u32) -> Range<u32> {
    let start = (visible.start as u32).saturating_sub(ROW_OVERSCAN);
    let end = (visible.end as u32)
        .saturating_add(ROW_OVERSCAN)
        .min(row_count);
    start.min(end)..end
}
