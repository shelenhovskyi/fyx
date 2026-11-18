//! The retained state of one extension surface, shared by every view that shows it (a panel in
//! each workspace window, or a status item in each status bar).
//!
//! All extension calls start here and run on tasks; views only read this state while rendering.

use std::{
    collections::VecDeque,
    ops::Range,
    rc::Rc,
    time::{Duration, Instant},
};

use collections::{HashMap, HashSet};
use gpui::{App, Context, EntityId, SharedString, Task};
use ui_ext::v0_2::{NodeId, Patch, Props, Surface, SurfaceKind, UiEvent, Update, tree::Tree};
use ui_ext_wasm::ticks::{PendingTick, TickConditions, TickErrors};

use crate::backend::{HostEffect, Reply, ReplyError, Row, UiBackend};

/// How a surface reaches the other surfaces of its extension (the registry implements it).
pub trait SurfaceLink {
    /// Delivers a push that a call on one surface made for another surface of the extension.
    fn route_effect(&self, effect: HostEffect, cx: &mut App);
    /// The extension's instance is gone (trap, time budget, disabled): every other surface of
    /// it stops too.
    fn extension_failed(&self, from_surface: &str, message: SharedString, cx: &mut App);
    /// Loads the extension again from where it came from, on a fresh instance; its surfaces
    /// remount on it (plan 1.3, spec decision 8).
    fn restart_extension(&self, cx: &mut App);
    /// The extension's tick budget (design.md, B), shared by its surfaces: when a tick due at
    /// `due` may run. Without a registry, at `due`.
    fn reserve_tick(&self, due: Instant, _cx: &mut App) -> Instant {
        due
    }
    /// The extension's least tick delay now (raised while it is throttled).
    fn min_tick_delay(&self, _cx: &App) -> Duration {
        ui_ext_wasm::ticks::MIN_TICK_DELAY
    }
    /// A tick call finished after `busy`, for the extension's CPU share.
    fn record_tick(&self, _finished_at: Instant, _busy: Duration, _cx: &mut App) {}
}

/// An item of a surface's queue: a user event, or a due tick, which waits behind the events
/// queued before it and never overtakes one (design.md, B).
enum Queued {
    Event(QueuedEvent),
    Tick,
}

/// What the event worker sends next.
enum Next {
    Event(UiEvent),
    Tick,
}

/// Events a surface queues for its extension before it drops new ones.
pub const MAX_QUEUED_EVENTS: usize = 64;

/// Remounts in a row caused by updates that did not fit, with no user input between them, before
/// the surface gives up (judge review w3-deep-1, finding 4).
const MAX_REMOUNTS_WITHOUT_INPUT: u32 = 3;

/// Cached rows above this count are pruned to the neighbourhood of the latest request.
const MAX_CACHED_ROWS: usize = 1024;
/// Rows fetched beyond the visible range, so short scrolls do not show placeholders.
pub(crate) const ROW_OVERSCAN: u32 = 8;

pub enum Phase {
    /// A modal that is not shown: no tree, nothing mounted (design.md, C).
    Closed,
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
    /// Levels to indent the row by (0.2's `list-row.indent`, clamped).
    indent: u8,
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
    events: VecDeque<Queued>,
    event_worker_running: bool,
    /// The one pending `request-tick`, if any; a new request replaces it.
    tick: Option<PendingTick>,
    /// The timer that queues the pending tick when it is due.
    _tick_timer: Option<Task<()>>,
    tick_errors: TickErrors,
    /// A due tick met a surface that was mounting: it goes out after the mount.
    tick_waits_for_mount: bool,
    /// Shown somewhere: a status item always; a panel while it is the active panel of an open
    /// dock in some window (reported by its containers). Ticks wait while hidden.
    visible_in: HashSet<EntityId>,
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
    /// The raw error of the failure that stopped the extension, behind "Copy Error Details".
    failure_detail: Option<SharedString>,
    /// Bumped when the surface moves to a new instance (`replace_backend`): a result of a call
    /// made on an older instance is dropped, so an old instance can neither patch the new tree
    /// nor fail a healthy extension (triage-3 #4).
    backend_epoch: u64,
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
    /// A surface that mounts itself from the extension.
    pub fn new(
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        link: Option<Rc<dyn SurfaceLink>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let modal = matches!(surface.kind, SurfaceKind::Modal(_));
        let mut this = Self::unmounted(backend, surface, link);
        if modal {
            // A modal mounts each time it is shown.
            this.phase = Phase::Closed;
        } else {
            this.mount(cx);
        }
        this
    }

    pub fn is_modal(&self) -> bool {
        matches!(self.surface.kind, SurfaceKind::Modal(_))
    }

    /// Whether a modal is shown (anything but `Closed`).
    pub fn is_open(&self) -> bool {
        !matches!(self.phase, Phase::Closed)
    }

    /// `show-surface` of a modal: a fresh mount, since modals keep no tree between showings.
    /// Showing one that is already shown does nothing.
    pub fn show(&mut self, cx: &mut Context<Self>) {
        if self.is_open() {
            return;
        }
        self.phase = Phase::Mounting;
        self.revision += 1;
        self.mount(cx);
        cx.notify();
    }

    /// Closes a modal: the tree, queued events and the pending tick go. With a `reason` the user
    /// closed it, and `dismissed(reason)` goes to the extension (the only item left in the
    /// queue, so it never waits on events for a tree that is gone); without one the extension
    /// asked (`hide-surface`, `remove-surface`), and nothing is sent.
    pub fn close(&mut self, reason: Option<ui_ext::v0_2::DismissReason>, cx: &mut Context<Self>) {
        if !self.is_open() {
            return;
        }
        self.phase = Phase::Closed;
        self.events.clear();
        self.lists.clear();
        self.tick = None;
        self._tick_timer = None;
        self.tick_waits_for_mount = false;
        self._mount = None;
        self.resyncing = false;
        self.last_error = None;
        self.revision += 1;
        if let Some(reason) = reason {
            let event = UiEvent::Dismissed(reason);
            self.record(HistoryEntry::Event {
                at: std::time::SystemTime::now(),
                event: event.clone(),
            });
            self.events.push_back(Queued::Event(QueuedEvent {
                event,
                row_generation: None,
            }));
            self.run_queue(cx);
        }
        cx.notify();
    }

    /// Keyboard selection in a list (design.md, C): the host moves `selected` in its own tree
    /// first, like typed text, then sends `row-selected`. Consecutive selections of one list
    /// are coalesced. Returns the new index.
    pub fn select_row(&mut self, list: NodeId, index: u32, cx: &mut Context<Self>) -> Option<u32> {
        let Phase::Ready(tree) = &mut self.phase else {
            return None;
        };
        let Some(Props::List(props)) = tree.get(list).map(|node| node.props()) else {
            return None;
        };
        if props.row_count == 0 {
            return None;
        }
        let index = index.min(props.row_count - 1);
        if props.selected == Some(index) {
            return Some(index);
        }
        let mut props = props.clone();
        props.selected = Some(index);
        if let Err(error) = tree.apply_patches(vec![Patch::SetProps(list, Props::List(props))]) {
            log::error!("fyx:ui: failed to record the selection of list {list}: {error}");
            return None;
        }
        self.revision += 1;
        if let Some(Queued::Event(QueuedEvent {
            event: UiEvent::RowSelected(queued, _),
            ..
        })) = self.events.back()
            && *queued == list
        {
            self.events.pop_back();
        }
        self.dispatch(UiEvent::RowSelected(list, index), cx);
        cx.notify();
        Some(index)
    }

    /// The first list node of the tree in pre-order (a modal's keyboard selection moves it).
    pub fn first_list(&self) -> Option<(NodeId, Option<u32>, u32)> {
        let tree = self.tree()?;
        let mut stack: Vec<NodeId> = tree.roots().iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let node = tree.get(id)?;
            if let Props::List(list) = node.props() {
                return Some((id, list.selected, list.row_count));
            }
            stack.extend(node.children().iter().rev());
        }
        None
    }

    /// A surface whose first tree is already known (a decoration's `place-decoration` nodes):
    /// no `mount` call; a later resync or Retry mounts from the extension as usual.
    pub fn with_tree(
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        link: Option<Rc<dyn SurfaceLink>>,
        tree: Tree,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::unmounted(backend, surface, link);
        this.set_tree(tree);
        cx.notify();
        this
    }

    fn unmounted(
        backend: Rc<dyn UiBackend>,
        surface: Surface,
        link: Option<Rc<dyn SurfaceLink>>,
    ) -> Self {
        let backend_version = backend.api_version();
        Self {
            backend,
            surface,
            link,
            phase: Phase::Mounting,
            last_error: None,
            reload_failed: false,
            lists: HashMap::default(),
            events: VecDeque::new(),
            event_worker_running: false,
            tick: None,
            _tick_timer: None,
            tick_errors: TickErrors::default(),
            tick_waits_for_mount: false,
            visible_in: HashSet::default(),
            input_sets: HashMap::default(),
            revision: 0,
            mounts: 0,
            resyncing: false,
            remounts_without_input: 0,
            next_row_generation: 0,
            history: VecDeque::new(),
            rules: ApiRules::for_version(&backend_version),
            backend_epoch: 0,
            failure_detail: None,
            _mount: None,
        }
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
            Phase::Closed | Phase::Mounting | Phase::Failed { .. } => None,
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
        let epoch = self.backend_epoch;
        self._mount = Some(cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.backend_epoch != epoch {
                    return;
                }
                this.resyncing = false;
                // A surface that stopped (failed) while its mount was in flight stays stopped.
                if matches!(this.phase, Phase::Failed { .. }) {
                    return;
                }
                if !this.is_open() {
                    // A modal closed while its mount was in flight.
                    return;
                }
                match result {
                    Ok(Reply { effects, value }) => {
                        // Pushes for this surface made during `mount` are superseded by the tree
                        // it returns; every other effect still applies.
                        let others = this.without_own_pushes(effects);
                        this.apply_effects(others, cx);
                        this.set_tree(value);
                    }
                    Err(error) => {
                        log::error!(
                            "fyx:ui {}/{}: mount failed: {}",
                            this.extension_id(),
                            this.surface.id,
                            ui_ext::sanitize_for_log(&format!("{error:#}"))
                        );
                        if let Some(effects) = reply_error_effects(&error) {
                            // This surface has no tree to apply its own pushes to.
                            let others = this.without_own_pushes(effects);
                            this.apply_effects(others, cx);
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
        self.backend_epoch += 1;
        self.rules = ApiRules::for_version(&self.backend.api_version());
        self.events.clear();
        self.retry(cx);
    }

    /// Mounts again after a failure, at the user's request.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.failure_detail = None;
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
        if !retryable {
            // The instance will not be called again: let it go (a restart or reinstall brings a
            // new one through `replace_backend`).
            self.backend = Rc::new(crate::backend::DeadBackend::of(&*self.backend));
        }
        self.phase = Phase::Failed { message, retryable };
        self.events.clear();
        // A failed surface drops its tick (design.md, B) and its mount in flight, so it does not
        // revive when that mount answers (triage-3 #4).
        self._mount = None;
        self.tick = None;
        self._tick_timer = None;
        self.tick_waits_for_mount = false;
        self.lists.clear();
        self.resyncing = false;
        self.revision += 1;
        cx.notify();
    }

    /// An error without a `ReplyError` means the instance is gone (trap, time budget, or disabled
    /// by an earlier failure): this surface and every other surface of the extension stop.
    fn fail_extension(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message: SharedString = stopped_message(self.backend.extension_id(), error).into();
        if let Some(link) = self.link.clone() {
            link.extension_failed(&self.surface.id, message.clone(), cx);
        }
        self.fail(message, false, cx);
        // The raw error, for "Copy Error Details" (triage-3 #7).
        self.failure_detail = Some(format!("{error:#}").into());
    }

    /// The raw error behind a stopped surface, if this surface saw it.
    pub fn failure_detail(&self) -> Option<&SharedString> {
        self.failure_detail.as_ref()
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
        if std::mem::take(&mut self.tick_waits_for_mount) {
            self.events.push_back(Queued::Tick);
        }
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
            log::debug!(
                "fyx:ui: dropping {:?}, its node is gone",
                self.redacted(&event)
            );
            return;
        }
        let shown = self.redacted(&event);
        self.record(HistoryEntry::Event {
            at: std::time::SystemTime::now(),
            event: shown,
        });
        if let UiEvent::InputChanged(id, value) | UiEvent::InputSubmitted(id, value) = &event {
            self.set_input_locally(*id, value);
        }
        if let UiEvent::InputChanged(id, _) = &event
            && let Some(Queued::Event(QueuedEvent {
                event: UiEvent::InputChanged(queued_id, _),
                ..
            })) = self.events.back()
            && queued_id == id
        {
            self.events.pop_back();
        }
        // A user event lets ticks resume after errors stopped them.
        self.tick_errors.on_user_event();
        // A guest that never answers must not let the queue grow without end (triage-3, ASYNC 10).
        if self.events.len() >= MAX_QUEUED_EVENTS {
            log::warn!(
                "fyx:ui {}/{}: {} events are waiting; dropping {:?}",
                self.extension_id(),
                self.surface.id,
                self.events.len(),
                self.redacted(&event)
            );
            return;
        }
        let row_generation = match &event {
            // The generation of the row the user saw, which may be older than the list's
            // current one while its replacement is on the way (integration-3 finding 9).
            UiEvent::RowClick(list, index) => Some(self.drawn_row_generation(*list, *index)),
            _ => None,
        };
        self.events.push_back(Queued::Event(QueuedEvent {
            event,
            row_generation,
        }));
        self.run_queue(cx);
    }

    /// Starts the worker that sends queued items one at a time, unless it runs already.
    fn run_queue(&mut self, cx: &mut Context<Self>) {
        if self.event_worker_running {
            return;
        }
        self.event_worker_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                let next = this.update(cx, |this, cx| {
                    let this_now = cx.background_executor().now();
                    let next = this.next_item();
                    if next.is_none() {
                        this.event_worker_running = false;
                    }
                    next.map(|next| {
                        let call = match &next {
                            Next::Event(event) => {
                                this.backend.handle_event(&this.surface.id, event.clone())
                            }
                            Next::Tick => this.backend.tick(&this.surface.id),
                        };
                        (next, call, this_now, this.backend_epoch)
                    })
                });
                let Ok(Some((next, call, started, epoch))) = next else {
                    break;
                };
                let result = call.await;
                let applied = this.update(cx, |this, cx| {
                    // A reply from the instance this surface had before a reload.
                    if this.backend_epoch != epoch {
                        return;
                    }
                    match next {
                        Next::Event(event) => this.apply_reply(&event, result, cx),
                        Next::Tick => this.apply_tick_reply(result, started, cx),
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The next queued event that still means what it meant when the user made it. Replies to
    /// earlier events may have removed its node or changed the list under a row click.
    fn next_item(&mut self) -> Option<Next> {
        while let Some(item) = self.events.pop_front() {
            let QueuedEvent {
                event,
                row_generation,
            } = match item {
                // A tick goes out only to a mounted surface; otherwise it waits for the mount.
                Queued::Tick if self.tree().is_some() => return Some(Next::Tick),
                Queued::Tick => {
                    self.tick_waits_for_mount = true;
                    continue;
                }
                Queued::Event(queued) => queued,
            };
            let still_there = match event.target() {
                // `dismissed` belongs to the whole surface, which has no tree any more.
                None => true,
                Some(target) => self.tree().is_some_and(|tree| tree.contains(target)),
            };
            let same_rows = match (&event, row_generation) {
                (UiEvent::RowClick(list, _), Some(generation)) => {
                    self.row_generation(*list) == generation
                }
                _ => true,
            };
            if still_there && same_rows {
                return Some(Next::Event(event));
            }
            log::debug!(
                "fyx:ui: dropping {:?}, an earlier reply changed its target",
                self.redacted(&event)
            );
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
            .filter_map(|queued| match queued {
                Queued::Event(queued) => Some(queued.event.clone()),
                Queued::Tick => None,
            })
            .collect()
    }

    /// `request-tick` for this surface: one pending tick, `delay` from now (when the requesting
    /// call's effects apply), replacing any earlier one. Ignored while errors stopped its ticks.
    pub fn request_tick(&mut self, delay: Duration, cx: &mut Context<Self>) {
        if self.tick_errors.stopped() {
            log::debug!(
                "fyx:ui {}/{}: request-tick ignored, ticks stopped after errors",
                self.backend.extension_id(),
                self.surface.id
            );
            return;
        }
        self.tick = Some(PendingTick::new(delay, cx.background_executor().now()));
        self.schedule_tick(cx);
    }

    pub fn cancel_tick(&mut self) {
        self.tick = None;
        self._tick_timer = None;
    }

    pub fn has_pending_tick(&self) -> bool {
        self.tick.is_some()
    }

    /// A container of this surface became visible or hidden (`id` names the container).
    pub fn set_visible(&mut self, container: EntityId, visible: bool, cx: &mut Context<Self>) {
        let changed = if visible {
            self.visible_in.insert(container)
        } else {
            self.visible_in.remove(&container)
        };
        if changed {
            self.schedule_tick(cx);
        }
    }

    pub(crate) fn is_visible(&self) -> bool {
        matches!(self.surface.kind, SurfaceKind::StatusItem(_)) || !self.visible_in.is_empty()
    }

    /// Arms the timer for the pending tick under design B's rules, or disarms it while hidden.
    fn schedule_tick(&mut self, cx: &mut Context<Self>) {
        let Some(tick) = self.tick else {
            self._tick_timer = None;
            return;
        };
        let min_delay = self
            .link
            .as_ref()
            .map_or(ui_ext_wasm::ticks::MIN_TICK_DELAY, |link| {
                link.min_tick_delay(cx)
            });
        let conditions = TickConditions {
            visible: self.is_visible(),
            app_active: cx.active_window().is_some(),
            extension_min_delay: min_delay,
        };
        let Some(due) = tick.due(conditions) else {
            self._tick_timer = None;
            return;
        };
        let run_at = match self.link.clone() {
            Some(link) => link.reserve_tick(due, cx),
            None => due,
        };
        let wait = run_at.saturating_duration_since(cx.background_executor().now());
        let timer = cx.background_executor().timer(wait);
        self._tick_timer = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.tick_due(cx)).ok();
        }));
    }

    /// The pending tick is due: it joins the queue behind the events already in it.
    fn tick_due(&mut self, cx: &mut Context<Self>) {
        if self.tick.take().is_none() {
            return;
        }
        self._tick_timer = None;
        self.events.push_back(Queued::Tick);
        self.run_queue(cx);
    }

    /// A tick's reply applies like an event's, but does not count as user progress for the
    /// remount bound, so a tick loop cannot keep a broken surface remounting (design.md, B).
    fn apply_tick_reply(
        &mut self,
        result: anyhow::Result<Reply<Update>>,
        started: Instant,
        cx: &mut Context<Self>,
    ) {
        let finished = cx.background_executor().now();
        if let Some(link) = self.link.clone() {
            link.record_tick(finished, finished - started, cx);
        }
        match result {
            Ok(Reply { effects, value }) => {
                self.tick_errors.on_success();
                if self.apply_effects(effects, cx) {
                    self.apply_update(value, cx);
                }
            }
            Err(error) => {
                log::error!(
                    "fyx:ui {}/{}: tick failed: {}",
                    self.extension_id(),
                    self.surface.id,
                    ui_ext::sanitize_for_log(&format!("{error:#}"))
                );
                match reply_error_effects(&error) {
                    Some(effects) => {
                        self.apply_effects(effects, cx);
                        if self.tick_errors.on_error() {
                            log::warn!(
                                "fyx:ui {}/{}: ticks stopped after {} errors in a row, until the \
                                 next user event",
                                self.extension_id(),
                                self.surface.id,
                                ui_ext_wasm::ticks::MAX_CONSECUTIVE_TICK_ERRORS
                            );
                            self.cancel_tick();
                        }
                        self.last_error = Some(format!("{error:#}").into());
                        cx.notify();
                    }
                    None => self.fail_extension(&error, cx),
                }
            }
        }
    }

    /// The event with a secret input's text replaced (design K): what logs and the Extension
    /// Inspector may show.
    pub fn redacted(&self, event: &UiEvent) -> UiEvent {
        let secret = |id: NodeId| {
            matches!(
                self.tree().and_then(|tree| tree.get(id)).map(|node| node.props()),
                Some(Props::Input(input)) if input.secret
            )
        };
        match event {
            UiEvent::InputChanged(id, _) if secret(*id) => {
                UiEvent::InputChanged(*id, "<secret>".into())
            }
            UiEvent::InputSubmitted(id, _) if secret(*id) => {
                UiEvent::InputSubmitted(*id, "<secret>".into())
            }
            event => event.clone(),
        }
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
            Ok(Reply { effects, value }) => {
                if self.apply_effects(effects, cx) && self.apply_update(value, cx) {
                    self.remounts_without_input = 0;
                }
            }
            Err(error) => {
                log::error!(
                    "fyx:ui {}/{}: handle-event({:?}) failed: {}",
                    self.extension_id(),
                    self.surface.id,
                    self.redacted(event),
                    ui_ext::sanitize_for_log(&format!("{error:#}"))
                );
                match reply_error_effects(&error) {
                    Some(effects) => {
                        self.apply_effects(effects, cx);
                        self.last_error = Some(format!("{error:#}").into());
                        cx.notify();
                    }
                    None => self.fail_extension(&error, cx),
                }
            }
        }
    }

    fn without_own_pushes(&self, effects: Vec<HostEffect>) -> Vec<HostEffect> {
        effects
            .into_iter()
            .filter(|effect| {
                !matches!(effect, HostEffect::Push { surface_id, .. } if *surface_id == self.surface.id)
            })
            .collect()
    }

    /// Applies a call's effects in order: this surface's pushes here, everything else (pushes to
    /// other surfaces and decorations, ticks, surfaces added, removed, shown or hidden) through the
    /// link. Returns false if one of this surface's pushes failed: the surface is then being
    /// mounted again, so its later pushes and the reply no longer fit and are skipped. Other
    /// effects still apply, because the extension already counts them as done.
    fn apply_effects(&mut self, effects: Vec<HostEffect>, cx: &mut Context<Self>) -> bool {
        let mut own_ok = true;
        for effect in effects {
            match effect {
                HostEffect::Push { surface_id, update } if surface_id == self.surface.id => {
                    if own_ok && !self.apply_update(update, cx) {
                        own_ok = false;
                    }
                }
                HostEffect::RequestTick { surface_id, delay } if surface_id == self.surface.id => {
                    self.request_tick(delay, cx)
                }
                HostEffect::CancelTick { surface_id } if surface_id == self.surface.id => {
                    self.cancel_tick()
                }
                effect => match self.link.clone() {
                    Some(link) => link.route_effect(effect, cx),
                    None => log::error!(
                        "fyx:ui {}: dropping {effect:?}, nothing routes it",
                        self.backend.extension_id(),
                    ),
                },
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
                    "fyx:ui {}/{}: invalid update, mounting again: {}",
                    self.backend.extension_id(),
                    self.surface.id,
                    ui_ext::sanitize_for_log(&format!("{error:#}"))
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

    /// The indent level of a cached row (0 when unknown).
    pub fn row_indent(&self, list: NodeId, index: u32) -> u8 {
        self.lists
            .get(&list)
            .and_then(|cache| cache.rows.get(&index))
            .map_or(0, |row| row.indent)
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
        let epoch = self.backend_epoch;
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                if this.backend_epoch == epoch {
                    this.receive_rows(list, generation, range, result, cx)
                }
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
        result: anyhow::Result<Reply<Vec<Row>>>,
        cx: &mut Context<Self>,
    ) {
        let result = match result {
            Ok(Reply { effects, value }) => {
                // A push that touches this list gives it a new generation, so these rows no
                // longer count and are fetched again.
                if self.apply_effects(effects, cx) {
                    self.remounts_without_input = 0;
                }
                Ok(value)
            }
            Err(error) => match reply_error_effects(&error) {
                Some(effects) => {
                    self.apply_effects(effects, cx);
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
                    let mut answered: HashMap<u32, Row> =
                        rows.into_iter().map(|row| (row.index, row)).collect();
                    for index in range.clone() {
                        let row = answered.remove(&index);
                        let indent = row.as_ref().map_or(0, |row| row.indent);
                        let tree = row.map(|row| row.tree);
                        cache.rows.insert(
                            index,
                            CachedRow {
                                generation,
                                tree,
                                indent,
                            },
                        );
                    }
                    if cache.rows.len() > MAX_CACHED_ROWS {
                        let keep = range.start.saturating_sub(MAX_CACHED_ROWS as u32 / 4)
                            ..range.end.saturating_add(MAX_CACHED_ROWS as u32 / 4);
                        cache.rows.retain(|index, _| keep.contains(index));
                    }
                }
                Err(error) => {
                    log::error!(
                        "fyx:ui {}/{}: list-rows({list}, {range:?}) failed: {}",
                        self.backend.extension_id(),
                        self.surface.id,
                        ui_ext::sanitize_for_log(&format!("{error:#}"))
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

/// What a stopped extension shows, in plain words: what happened and what to do. The raw
/// error is the detail.
fn stopped_message(extension_id: &str, error: &anyhow::Error) -> String {
    let raw = format!("{error:#}").to_lowercase();
    let what = if raw.contains("time budget") || raw.contains("interrupt") {
        "took too long to answer and was stopped"
    } else if raw.contains("memory") {
        "ran out of memory and was stopped"
    } else if raw.contains("disabled after an earlier failure") {
        "is stopped after an earlier failure"
    } else if raw.contains("shut down") {
        "is no longer running"
    } else {
        "crashed"
    };
    format!("{extension_id} {what}. Restart the extension to try again.")
}

/// The pushes a rejected call carries, or `None` when the error is not a rejection: then the
/// instance itself failed (trap, time budget, disabled) and carries none.
fn reply_error_effects(error: &anyhow::Error) -> Option<Vec<HostEffect>> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ReplyError>())
        .map(|reply_error| reply_error.effects.clone())
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
