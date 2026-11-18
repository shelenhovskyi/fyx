//! The retained state of one extension surface, shared by every view that shows it (a panel in
//! each workspace window, or a status item in each status bar).
//!
//! All extension calls start here and run on tasks; views only read this state while rendering.

use std::{collections::VecDeque, ops::Range, rc::Rc};

use collections::HashMap;
use gpui::{App, Context, SharedString, Task};
use ui_ext::v0_2::{NodeId, Patch, Props, Surface, UiEvent, Update, tree::Tree};

use crate::backend::{PushedUpdate, Reply, ReplyError, UiBackend};

/// How a surface reaches the other surfaces of its extension (the registry implements it).
pub trait SurfaceLink {
    /// Delivers a push that a call on one surface made for another surface of the extension.
    fn route_push(&self, push: PushedUpdate, cx: &mut App);
    /// The extension's instance is gone (trap, time budget, disabled): every other surface of
    /// it stops too.
    fn extension_failed(&self, from_surface: &str, message: SharedString, cx: &mut App);
    /// Loads the extension again from where it came from, on a fresh instance; its surfaces
    /// remount on it (plan 1.3, spec decision 8).
    fn restart_extension(&self, cx: &mut App);
}

/// Remounts in a row caused by updates that did not fit, with no user input between them, before
/// the surface gives up (judge review w3-deep-1, finding 4).
const MAX_REMOUNTS_WITHOUT_INPUT: u32 = 3;

/// Cached rows above this count are pruned to the neighbourhood of the latest request.
const MAX_CACHED_ROWS: usize = 1024;
/// Rows fetched beyond the visible range, so short scrolls do not show placeholders.
pub(crate) const ROW_OVERSCAN: u32 = 8;

pub enum Phase {
    Mounting,
    Ready(Tree),
    /// `retryable` is false when the extension's instance is gone (trap, time budget, disabled):
    /// mounting again would be refused, so only a reinstall or a restart brings it back.
    Failed {
        message: SharedString,
        retryable: bool,
    },
}

/// The rows of one virtualized list or table node. Rows are subtrees with row-local node ids, so
/// they are kept apart from the surface tree and addressed by `(list node, index)`.
#[derive(Default)]
struct RowCache {
    /// Changes whenever the list's props change. Taken from `SurfaceState::next_row_generation`,
    /// which never goes back, so a reply to a fetch made before a remount (which starts every
    /// cache again) can never pass for a current one (integration-3 finding 8). Rows from older
    /// generations are still shown until their replacement arrives, which avoids flicker.
    generation: u64,
    rows: HashMap<u32, CachedRow>,
    /// The generation of the fetch in flight, if any.
    in_flight: Option<u64>,
    /// Rows asked for and not yet fetched: the union of every request since the last fetch, so
    /// the measuring call and the visible range of one layout make one `list-rows` call.
    wanted: Option<Range<u32>>,
    /// A fetch of `wanted` is scheduled for after the current layout.
    scheduled: bool,
    /// A fetch for this generation failed; do not retry until the list changes.
    failed_generation: Option<u64>,
}

/// A union of wanted rows wider than this is replaced by the latest request instead (two
/// windows scrolled far apart, say).
const MAX_WANTED_ROWS: u32 = 256;

/// One index of a list. Every index of an answered range gets an entry, `tree: None` for a row the
/// extension left out, so a short answer is not asked for again in the same generation however
/// many ranges (measuring, visible, other windows) are requested.
struct CachedRow {
    generation: u64,
    tree: Option<Tree>,
}

/// An event waiting for the extension, with what it was aimed at when the user made it.
struct QueuedEvent {
    event: UiEvent,
    /// For `row-click`: the list's row generation the user saw. A reply that changes the list
    /// before this event is sent makes the index point at another row, so the event is dropped.
    row_generation: Option<u64>,
}

pub struct SurfaceState {
    backend: Rc<dyn UiBackend>,
    surface: Surface,
    link: Option<Rc<dyn SurfaceLink>>,
    phase: Phase,
    last_error: Option<SharedString>,
    /// `last_error` is a failed reload: the old instance still runs, and the footer offers Retry.
    reload_failed: bool,
    lists: HashMap<NodeId, RowCache>,
    events: VecDeque<QueuedEvent>,
    event_worker_running: bool,
    /// How many times the extension has set each input's `value` (`set-props`, insert, replace).
    /// A view writes the value into its editor when this moves past what it last saw; otherwise
    /// the user's text stands (integrator decision 6). A counter, not a flag, so every window's
    /// view sees each set.
    input_sets: HashMap<NodeId, u64>,
    /// Bumped on every change to the tree, so views know when to reconcile their editors.
    revision: u64,
    /// Bumped when a mounted tree is applied; see `mount_count`.
    mounts: u64,
    resyncing: bool,
    /// Remounts caused by bad updates since the last update that applied (a reply, a push, a row
    /// answer).
    remounts_without_input: u32,
    /// The source of row generations; see `RowCache::generation`.
    next_row_generation: u64,
    /// Recent events and updates, for the Extension Inspector.
    history: VecDeque<HistoryEntry>,
    /// What differs by the component's `fyx:ui` version.
    rules: ApiRules,
    _mount: Option<Task<()>>,
}

/// The renderer's behaviour by the component's `fyx:ui` version (migration.md §3), decided once
/// when the surface is created, so no version comparisons are scattered through the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiRules {
    /// 0.2: rows are fetched again only as design G says; 0.1: every touched list refetches.
    pub precise_row_refetch: bool,
}

impl ApiRules {
    pub fn for_version(version: &semver::Version) -> Self {
        let v0_2 = *version >= semver::Version::new(0, 2, 0);
        Self {
            precise_row_refetch: v0_2,
        }
    }
}

/// Entries of a surface's history the Extension Inspector keeps.
pub const HISTORY_CAPACITY: usize = 200;

/// One event sent to the extension, or one tree change, for the Extension Inspector.
#[derive(Clone, Debug, PartialEq)]
pub enum HistoryEntry {
    Event {
        at: std::time::SystemTime,
        event: UiEvent,
    },
    /// A mounted tree, or an update (a reply, a push) and whether it applied.
    Update {
        at: std::time::SystemTime,
        summary: String,
        applied: bool,
    },
}

/// `replace 12 nodes`, `patches: set-props 3, insert 10, remove 4`, `unchanged`.
pub fn summarize_update(update: &Update) -> String {
    match update {
        Update::Unchanged => "unchanged".into(),
        Update::Replace(nodes) => format!("replace {} nodes", nodes.len()),
        Update::Patches(patches) => {
            let parts: Vec<String> = patches
                .iter()
                .map(|patch| match patch {
                    Patch::SetProps(id, props) => {
                        format!("set-props {id} ({})", props.kind_name())
                    }
                    Patch::Insert(insert) => format!(
                        "insert {} ({})",
                        insert.node.id,
                        insert.node.props.kind_name()
                    ),
                    Patch::Remove(id) => format!("remove {id}"),
                    Patch::InvalidateRows(op) => match op.rows {
                        Some(range) => format!(
                            "invalidate-rows {} {}..{}",
                            op.target,
                            range.start,
                            range.start + range.count
                        ),
                        None => format!("invalidate-rows {}", op.target),
                    },
                })
                .collect();
            format!("patches: {}", parts.join(", "))
        }
    }
}

impl SurfaceState {
    pub fn new(
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        link: Option<Rc<dyn SurfaceLink>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let backend_version = backend.api_version();
        let mut this = Self {
            backend,
            surface,
            link,
            phase: Phase::Mounting,
            last_error: None,
            reload_failed: false,
            lists: HashMap::default(),
            events: VecDeque::new(),
            event_worker_running: false,
            input_sets: HashMap::default(),
            revision: 0,
            mounts: 0,
            resyncing: false,
            remounts_without_input: 0,
            next_row_generation: 0,
            history: VecDeque::new(),
            rules: ApiRules::for_version(&backend_version),
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
            Phase::Mounting | Phase::Failed { .. } => None,
        }
    }

    pub fn last_error(&self) -> Option<&SharedString> {
        self.last_error.as_ref()
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.reload_failed = false;
        if self.last_error.take().is_some() {
            cx.notify();
        }
    }

    /// A reload of the extension failed: the surface keeps its instance and shows the error in
    /// its footer with Retry (spec decision 7).
    pub fn show_reload_error(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.last_error = Some(message);
        self.reload_failed = true;
        cx.notify();
    }

    pub fn reload_failed(&self) -> bool {
        self.reload_failed
    }

    /// Whether `restart_extension` can do anything: the surface belongs to a registry.
    pub fn can_restart(&self) -> bool {
        self.link.is_some()
    }

    /// Loads the extension again on a fresh instance (Restart extension, and Retry after a
    /// failed reload). Deferred: the registry updates this surface while it restarts.
    pub fn restart_extension(&mut self, cx: &mut Context<Self>) {
        if let Some(link) = self.link.clone() {
            cx.defer(move |cx| link.restart_extension(cx));
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
                        if let Some(pushed) = reply_error_pushes(&error) {
                            // This surface has no tree to apply its own pushes to.
                            let others = pushed
                                .into_iter()
                                .filter(|push| push.surface_id != this.surface.id)
                                .collect();
                            this.apply_pushes(others, cx);
                            this.fail(format!("{error:#}").into(), true, cx);
                        } else {
                            this.fail_extension(&error, cx);
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Swaps in a new instance of the extension (a reinstall) and mounts the surface on it.
    pub fn replace_backend(
        &mut self,
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        cx: &mut Context<Self>,
    ) {
        self.backend = backend;
        self.surface = surface;
        self.events.clear();
        self.retry(cx);
    }

    /// Mounts again after a failure, at the user's request.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.remounts_without_input = 0;
        self.last_error = None;
        self.reload_failed = false;
        self.phase = Phase::Mounting;
        self.revision += 1;
        self.mount(cx);
        cx.notify();
    }

    /// Stops the surface: no events, no rows, the message in place of the tree. `retry` mounts
    /// again when `retryable`.
    pub fn fail(&mut self, message: SharedString, retryable: bool, cx: &mut Context<Self>) {
        self.phase = Phase::Failed { message, retryable };
        self.events.clear();
        self.lists.clear();
        self.resyncing = false;
        self.revision += 1;
        cx.notify();
    }

    /// An error without a `ReplyError` means the instance is gone (trap, time budget, or disabled
    /// by an earlier failure): this surface and every other surface of the extension stop.
    fn fail_extension(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message: SharedString =
            format!("{} stopped: {error:#}", self.backend.extension_id()).into();
        if let Some(link) = self.link.clone() {
            link.extension_failed(&self.surface.id, message.clone(), cx);
        }
        self.fail(message, false, cx);
    }

    /// Applies the tree a `mount` returned: every mount is one the host started (the first one, a
    /// resync, Retry, a reload), never the extension's own rebuild.
    fn record(&mut self, entry: HistoryEntry) {
        if self.history.len() == HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(entry);
    }

    /// Recent events and updates, oldest first.
    pub fn history(&self) -> &VecDeque<HistoryEntry> {
        &self.history
    }

    /// The instance's recent calls, `log` output and capability checks, if the backend keeps them.
    pub fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        self.backend.diagnostics()
    }

    fn set_tree(&mut self, tree: Tree) {
        self.record(HistoryEntry::Update {
            at: std::time::SystemTime::now(),
            summary: format!("mount: {} nodes", tree.len()),
            applied: true,
        });
        count_all_inputs(&tree, &mut self.input_sets);
        self.phase = Phase::Ready(tree);
        self.lists.clear();
        self.mounts += 1;
        self.revision += 1;
    }

    /// How many mounted trees have been applied. A view carries unsent typing to renumbered inputs
    /// only when this moved, so the extension's own `replace` or rebuild clears a form as it means
    /// to (integration-4 finding 8).
    pub fn mount_count(&self) -> u64 {
        self.mounts
    }

    /// Queues one input event. Events reach the extension one at a time, in order, and each makes
    /// exactly one `handle-event` call; a queued `input-changed` for the same node is replaced by
    /// the newer one instead of making a second call.
    pub fn dispatch(&mut self, event: UiEvent, cx: &mut Context<Self>) {
        let Some(tree) = self.tree() else {
            return;
        };
        if event.target().is_some_and(|target| !tree.contains(target)) {
            log::debug!("fyx:ui: dropping {event:?}, its node is gone");
            return;
        }
        self.record(HistoryEntry::Event {
            at: std::time::SystemTime::now(),
            event: event.clone(),
        });
        if let UiEvent::InputChanged(id, value) | UiEvent::InputSubmitted(id, value) = &event {
            self.set_input_locally(*id, value);
        }
        if let UiEvent::InputChanged(id, _) = &event
            && let Some(QueuedEvent {
                event: UiEvent::InputChanged(queued_id, _),
                ..
            }) = self.events.back()
            && queued_id == id
        {
            self.events.pop_back();
        }
        let row_generation = match &event {
            // The generation of the row the user saw, which may be older than the list's
            // current one while its replacement is on the way (integration-3 finding 9).
            UiEvent::RowClick(list, index) => Some(self.drawn_row_generation(*list, *index)),
            _ => None,
        };
        self.events.push_back(QueuedEvent {
            event,
            row_generation,
        });
        if !self.event_worker_running {
            self.event_worker_running = true;
            cx.spawn(async move |this, cx| {
                loop {
                    let next = this.update(cx, |this, _| {
                        let event = this.next_event();
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

    /// The next queued event that still means what it meant when the user made it. Replies to
    /// earlier events may have removed its node or changed the list under a row click.
    fn next_event(&mut self) -> Option<UiEvent> {
        while let Some(QueuedEvent {
            event,
            row_generation,
        }) = self.events.pop_front()
        {
            let still_there = self
                .tree()
                .is_some_and(|tree| event.target().is_none_or(|target| tree.contains(target)));
            let same_rows = match (&event, row_generation) {
                (UiEvent::RowClick(list, _), Some(generation)) => {
                    self.row_generation(*list) == generation
                }
                _ => true,
            };
            if still_there && same_rows {
                return Some(event);
            }
            log::debug!("fyx:ui: dropping {event:?}, an earlier reply changed its target");
        }
        None
    }

    fn drawn_row_generation(&self, list: NodeId, index: u32) -> u64 {
        self.lists
            .get(&list)
            .and_then(|cache| cache.rows.get(&index))
            .map_or_else(|| self.row_generation(list), |row| row.generation)
    }

    /// Applies an update pushed outside a reply to this surface's own events (by another
    /// surface's call, or after instantiation). One that applies counts as progress for the
    /// remount bound, like an applied reply.
    pub fn apply_pushed_update(&mut self, update: Update, cx: &mut Context<Self>) {
        if self.apply_update(update, cx) {
            self.remounts_without_input = 0;
        }
    }

    #[cfg(test)]
    pub(crate) fn current_row_generation(&self, list: NodeId) -> u64 {
        self.row_generation(list)
    }

    #[cfg(test)]
    pub(crate) fn queued_events(&self) -> Vec<UiEvent> {
        self.events
            .iter()
            .map(|queued| queued.event.clone())
            .collect()
    }

    fn row_generation(&self, list: NodeId) -> u64 {
        self.lists.get(&list).map_or(0, |cache| cache.generation)
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
                if self.apply_pushes(pushed, cx) && self.apply_update(value, cx) {
                    self.remounts_without_input = 0;
                }
            }
            Err(error) => {
                log::error!(
                    "fyx:ui {}/{}: handle-event({event:?}) failed: {error:#}",
                    self.extension_id(),
                    self.surface.id
                );
                match reply_error_pushes(&error) {
                    Some(pushed) => {
                        self.apply_pushes(pushed, cx);
                        self.last_error = Some(format!("{error:#}").into());
                        cx.notify();
                    }
                    None => self.fail_extension(&error, cx),
                }
            }
        }
    }

    /// Applies pushes in order: this surface's here, other surfaces' through the link. Returns
    /// false if one of this surface's failed: the surface is then being mounted again, so its
    /// later pushes and the reply no longer fit and are skipped. Pushes for other surfaces are
    /// still routed, because their mirrors in the extension already count them as applied.
    fn apply_pushes(&mut self, pushed: Vec<PushedUpdate>, cx: &mut Context<Self>) -> bool {
        let mut own_ok = true;
        for push in pushed {
            if push.surface_id == self.surface.id {
                if own_ok && !self.apply_update(push.update, cx) {
                    own_ok = false;
                }
            } else if let Some(link) = self.link.clone() {
                link.route_push(push, cx);
            } else {
                log::error!(
                    "fyx:ui {}: dropping a push for surface {:?}, nothing routes it",
                    self.backend.extension_id(),
                    push.surface_id
                );
            }
        }
        own_ok
    }

    /// Applies an update from an event reply or a push. An update that does not fit the retained
    /// tree means host and extension disagree, so it is logged and the surface is mounted again
    /// (the SDK resets its mirror on mount). Returns whether the update applied.
    pub fn apply_update(&mut self, update: Update, cx: &mut Context<Self>) -> bool {
        let summary = summarize_update(&update);
        let applied = self.apply_update_inner(update, cx);
        if summary != "unchanged" || !applied {
            self.record(HistoryEntry::Update {
                at: std::time::SystemTime::now(),
                summary,
                applied,
            });
        }
        applied
    }

    fn apply_update_inner(&mut self, update: Update, cx: &mut Context<Self>) -> bool {
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
                // Which lists fetch their rows again (migration.md §3): a 0.1 component has no
                // `invalidate-rows`, so every touched list refetches; a 0.2 component refetches
                // only what design G names (`Touched::rows`). Either way only the visible range
                // is fetched, once.
                let refetch: Vec<NodeId> = if self.rules.precise_row_refetch {
                    touched.rows.keys().copied().collect()
                } else {
                    touched
                        .nodes
                        .iter()
                        .copied()
                        .filter(|id| {
                            matches!(
                                tree.get(*id).map(|node| node.props()),
                                Some(Props::List(_) | Props::Table(_))
                            )
                        })
                        .collect()
                };
                for id in refetch {
                    if let Some(cache) = self.lists.get_mut(&id) {
                        self.next_row_generation += 1;
                        cache.invalidate(self.next_row_generation);
                    }
                }
                for id in &touched.nodes {
                    // Set, inserted or replaced by the extension: the views write the value.
                    if let Some(Props::Input(_)) = tree.get(*id).map(|node| node.props()) {
                        *self.input_sets.entry(*id).or_default() += 1;
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
                    if self.remounts_without_input >= MAX_REMOUNTS_WITHOUT_INPUT {
                        self.fail(
                            format!(
                                "{} keeps sending updates that do not fit its tree: {error}",
                                self.backend.extension_id()
                            )
                            .into(),
                            true,
                            cx,
                        );
                        return false;
                    }
                    self.remounts_without_input += 1;
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
        self.lists.get(&list)?.rows.get(&index)?.tree.as_ref()
    }

    /// Makes sure rows `range` of `list` are fetched, without waiting for them. Cheap when they
    /// are cached, so views call it for the visible range on every layout. Requests made during
    /// one layout are merged into one `list-rows` call, made right after it.
    pub fn request_rows(&mut self, list: NodeId, range: Range<u32>, cx: &mut Context<Self>) {
        if range.is_empty() || self.tree().is_none_or(|tree| !tree.contains(list)) {
            return;
        }
        if !self.lists.contains_key(&list) {
            self.next_row_generation += 1;
            let cache = RowCache {
                generation: self.next_row_generation,
                ..RowCache::default()
            };
            self.lists.insert(list, cache);
        }
        let Some(cache) = self.lists.get_mut(&list) else {
            return;
        };
        if cache.failed_generation == Some(cache.generation) || !cache.missing(&range) {
            return;
        }
        cache.wanted = Some(match cache.wanted.take() {
            Some(wanted) => {
                let union = wanted.start.min(range.start)..wanted.end.max(range.end);
                if union.len() as u32 > MAX_WANTED_ROWS {
                    range
                } else {
                    union
                }
            }
            None => range,
        });
        self.schedule_fetch(list, cx);
    }

    fn schedule_fetch(&mut self, list: NodeId, cx: &mut Context<Self>) {
        let Some(cache) = self.lists.get_mut(&list) else {
            return;
        };
        if cache.scheduled || cache.in_flight.is_some() || cache.wanted.is_none() {
            return;
        }
        cache.scheduled = true;
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| this.fetch_rows(list, cx)).ok();
        })
        .detach();
    }

    fn fetch_rows(&mut self, list: NodeId, cx: &mut Context<Self>) {
        if self.tree().is_none_or(|tree| !tree.contains(list)) {
            return;
        }
        let Some(cache) = self.lists.get_mut(&list) else {
            return;
        };
        cache.scheduled = false;
        if cache.in_flight.is_some() {
            return;
        }
        let Some(range) = cache.wanted.take() else {
            return;
        };
        // Answered in the meantime, or the list changed so that nothing is missing.
        if cache.failed_generation == Some(cache.generation) || !cache.missing(&range) {
            return;
        }
        let generation = cache.generation;
        cache.in_flight = Some(generation);
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

    pub(crate) fn receive_rows(
        &mut self,
        list: NodeId,
        generation: u64,
        range: Range<u32>,
        result: anyhow::Result<Reply<Vec<(u32, Tree)>>>,
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Ok(Reply { pushed, value }) => {
                // A push that touches this list gives it a new generation, so these rows no
                // longer count and are fetched again.
                if self.apply_pushes(pushed, cx) {
                    self.remounts_without_input = 0;
                }
                Ok(value)
            }
            Err(error) => match reply_error_pushes(&error) {
                Some(pushed) => {
                    self.apply_pushes(pushed, cx);
                    Err(error)
                }
                None => {
                    self.fail_extension(&error, cx);
                    return;
                }
            },
        };
        let Some(cache) = self.lists.get_mut(&list) else {
            return;
        };
        if cache.in_flight == Some(generation) {
            cache.in_flight = None;
        }
        // An answer for an older generation (the list changed, or the surface was mounted again
        // while it was in flight) is dropped: storing it would mark rows the current generation
        // still needs.
        if generation == cache.generation {
            match result {
                Ok(rows) => {
                    let mut answered: HashMap<u32, Tree> = rows.into_iter().collect();
                    for index in range.clone() {
                        let tree = answered.remove(&index);
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
        }
        self.schedule_fetch(list, cx);
        cx.notify();
    }
}

impl RowCache {
    fn missing(&self, range: &Range<u32>) -> bool {
        range.clone().any(|index| {
            self.rows
                .get(&index)
                .is_none_or(|row| row.generation != self.generation)
        })
    }

    fn invalidate(&mut self, generation: u64) {
        self.generation = generation;
        self.failed_generation = None;
    }
}

/// The pushes a rejected call carries, or `None` when the error is not a rejection: then the
/// instance itself failed (trap, time budget, disabled) and carries none.
fn reply_error_pushes(error: &anyhow::Error) -> Option<Vec<PushedUpdate>> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ReplyError>())
        .map(|reply_error| reply_error.pushed.clone())
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
