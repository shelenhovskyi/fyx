//! The retained state of one extension surface, shared by every view that shows it (a panel in
//! each workspace window, or a status item in each status bar).
//!
//! All extension calls start here and run on tasks; views only read this state while rendering.

use std::{collections::VecDeque, ops::Range, rc::Rc};

use collections::{HashMap, HashSet};
use gpui::{Context, SharedString, Task};
use ui_ext::{NodeId, Patch, Props, Surface, Tree, UiEvent, Update};

use crate::backend::UiBackend;

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
    phase: Phase,
    last_error: Option<SharedString>,
    lists: HashMap<NodeId, RowCache>,
    events: VecDeque<UiEvent>,
    event_worker_running: bool,
    /// Inputs whose next `value` from the extension wins over the user's text even while
    /// focused, because it answers the user's own submit (for example, clearing the field).
    input_overrides: HashSet<NodeId>,
    /// Bumped on every change to the tree, so views know when to reconcile their editors.
    revision: u64,
    resyncing: bool,
    _mount: Option<Task<()>>,
}

impl SurfaceState {
    pub fn new(backend: Rc<dyn UiBackend>, surface: Surface, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            backend,
            surface,
            phase: Phase::Mounting,
            last_error: None,
            lists: HashMap::default(),
            events: VecDeque::new(),
            event_worker_running: false,
            input_overrides: HashSet::default(),
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
                    Ok(tree) => this.set_tree(tree),
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

    fn apply_reply(
        &mut self,
        event: &UiEvent,
        result: anyhow::Result<Update>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(update) => {
                if let UiEvent::InputSubmitted(id, _) = event
                    && sets_input(&update, *id)
                {
                    self.input_overrides.insert(*id);
                }
                self.apply_update(update, cx);
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

    /// Applies an update from an event reply or a push. An update that does not fit the retained
    /// tree means host and extension disagree, so the surface is mounted again.
    pub fn apply_update(&mut self, update: Update, cx: &mut Context<Self>) {
        let Phase::Ready(tree) = &mut self.phase else {
            log::debug!(
                "fyx:ui {}/{}: dropping an update that arrived before mount finished",
                self.backend.extension_id(),
                self.surface.id
            );
            return;
        };
        let invalidated = invalidated_lists(&update);
        match tree.apply_update(update) {
            Ok(false) => {}
            Ok(true) => {
                match invalidated {
                    None => self.lists.clear(),
                    Some(ids) => {
                        for id in ids {
                            if let Some(cache) = self.lists.get_mut(&id) {
                                cache.invalidate();
                            }
                        }
                    }
                }
                let tree = &*tree;
                self.lists.retain(|id, _| {
                    tree.get(*id).is_some_and(|node| {
                        matches!(node.props(), Props::List(_) | Props::Table(_))
                    })
                });
                self.input_overrides.retain(|id| tree.contains(*id));
                self.revision += 1;
                cx.notify();
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
            }
        }
    }

    pub fn take_input_override(&mut self, id: NodeId) -> bool {
        self.input_overrides.remove(&id)
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
        result: anyhow::Result<Vec<(u32, Tree)>>,
        cx: &mut Context<Self>,
    ) {
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

/// The list and table nodes whose rows an update makes stale, or `None` for "all of them".
///
/// The WIT has no "rows changed" patch, so a change to a list's props (its row count, usually)
/// is the signal to fetch its visible rows again.
fn invalidated_lists(update: &Update) -> Option<Vec<NodeId>> {
    match update {
        Update::Unchanged => Some(Vec::new()),
        Update::Replace(_) => None,
        Update::Patches(patches) => Some(
            patches
                .iter()
                .filter_map(|patch| match patch {
                    Patch::SetProps(id, Props::List(_) | Props::Table(_)) => Some(*id),
                    Patch::SetProps(..) | Patch::Insert(_) | Patch::Remove(_) => None,
                })
                .collect(),
        ),
    }
}

fn sets_input(update: &Update, input: NodeId) -> bool {
    match update {
        Update::Unchanged => false,
        Update::Replace(_) => true,
        Update::Patches(patches) => patches
            .iter()
            .any(|patch| matches!(patch, Patch::SetProps(id, Props::Input(_)) if *id == input)),
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
