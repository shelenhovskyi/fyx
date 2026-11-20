//! The 0.2 host imports' checks and effect list (`docs/fyx/wit-0.2/design.md`, A): every effectful
//! import is checked when it is called, against the live ids as of that point in the call, and
//! its effect is recorded to be applied after the call returns, in call order, before the call's
//! own value. It mirrors the host runtime (`ui_ext_wasm`); the id rules, caps and capability
//! checks are `ui_ext::LiveIds`, which both use.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ui_ext::image::{self, ImageFormat};

use ui_ext::edit::{EditCounters, WorkspaceEdit};
use ui_ext::network::{self as net, DomainList, FetchCounters, FetchRequest, PermissionState};
use ui_ext::process::{self as proc, ProcessCounters, ProcessPolicy, SpawnCheck, SpawnRequest};
use ui_ext::v0_2 as model;
use ui_ext::v0_2::notifications::{self as notify, Notification, ProgressOptions, ProgressValue};
use ui_ext::{ApiError, ApiVersion, LiveIds, MAX_DECORATION_EFFECTS_PER_CALL, Placement};

use crate::MAX_PUSHES_PER_CALL;

/// Patches plus nodes that one call's 0.2 pushes may carry together, past which `push-update`
/// returns `limit-exceeded`: two whole trees, so a guest can always resend one surface after a
/// failed push.
pub const MAX_PUSHED_ITEMS_PER_CALL_V0_2: usize = 2 * 16384;
/// Shorter tick delays are clamped up to this (design.md, B).
pub const MIN_TICK_DELAY: Duration = Duration::from_millis(100);
/// Longer tick delays are refused with `invalid-argument`.
pub const MAX_TICK_DELAY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after a click or Enter an extension may show a modal (design.md, C).
pub const ACTIVATION_WINDOW: Duration = Duration::from_secs(5);

/// Something a host import asked for, checked at the call and applied after it, in call order.
/// A 0.1 component only produces [`HostEffect::Push`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEffect {
    /// `push-update`, to a surface or a decoration.
    Push {
        surface_id: String,
        update: model::Update,
    },
    /// `request-tick`, with the delay already clamped up to [`MIN_TICK_DELAY`]. It replaces any
    /// pending tick of the surface.
    RequestTick { surface_id: String, delay: Duration },
    /// `cancel-tick` of a live surface.
    CancelTick { surface_id: String },
    /// `add-surface`.
    AddSurface(model::Surface),
    /// `remove-surface`: its tree, pending tick and queued events go. No event.
    RemoveSurface { surface_id: String },
    /// `show-surface`: a modal opens and mounts (the gesture check passed); a panel's dock is
    /// revealed.
    ShowSurface { surface_id: String },
    /// `hide-surface`: a modal closes without a `dismissed` event; a panel's dock is hidden.
    HideSurface { surface_id: String },
    /// `place-decoration`: a live id moves and its tree is replaced.
    PlaceDecoration {
        id: String,
        location: model::DecorationLocation,
        nodes: Vec<model::Node>,
    },
    /// `remove-decoration`. No event.
    RemoveDecoration { id: String },
    /// `images.add-image` (0.3): the bytes passed the size cap and the header checks of
    /// `ui_ext::image::check_image_bytes`. `handle` never repeats within an instance.
    AddImage {
        handle: u32,
        format: ImageFormat,
        bytes: Arc<[u8]>,
    },
    /// `images.remove-image` of a live handle: nodes that show it fall back to their alt text.
    RemoveImage { handle: u32 },
    /// `notifications.notify` (design.md M.1): shown in the active window; its events go to
    /// `notification.owner`.
    Notify { id: u64, notification: Notification },
    /// `dismiss-notification` of an open notification: closed, with no event.
    DismissNotification { id: u64 },
    /// `progress-start` (M.2), indeterminate until the first update.
    ProgressStart { id: u64, options: ProgressOptions },
    /// `progress-update` of a running progress; `None` keeps the previous message.
    ProgressUpdate {
        id: u64,
        value: ProgressValue,
        message: Option<String>,
    },
    /// `progress-end` of a running progress, with no event.
    ProgressEnd { id: u64 },
    /// 0.4 `settings.open` (design.md §D.4), from an event call: show the extension's Settings
    /// tab, scrolled to `key` (a declared one) when given.
    OpenSettings { key: Option<String> },
}

/// A transient activation (design.md, C): granted by `handle-event` for a click or Enter, it lasts
/// until [`ACTIVATION_WINDOW`] after that call returns, or until one modal is shown with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Activation {
    None,
    DuringCall,
    Until(Instant),
}

fn host_error(code: model::HostErrorCode, message: impl Into<String>) -> model::HostError {
    model::HostError {
        code,
        message: message.into(),
    }
}

/// `settings.get` or `open` of a key `[[fyx_ui.settings]]` does not declare.
pub(crate) fn undeclared_setting(key: &str) -> model::HostError {
    host_error(
        model::HostErrorCode::InvalidArgument,
        format!("no setting {key:?} is declared in [[fyx_ui.settings]]"),
    )
}

fn api_error(error: ApiError) -> model::HostError {
    model::HostError::from(&error)
}

/// One extension's live ids, capabilities, shown modals and activation, and the running call's
/// effects and counters.
pub(crate) struct ApiState {
    version: ApiVersion,
    capabilities: Vec<String>,
    live: LiveIds,
    /// Shown as far as the host knows: by `show-surface`, until `hide-surface`, `remove-surface`
    /// or a `dismissed` event.
    shown_modals: BTreeSet<String>,
    activation: Activation,
    effects: Vec<HostEffect>,
    pushes: usize,
    pushed_items: usize,
    decoration_effects: usize,
    /// Live generated images (0.3), at most `ui_ext::image::MAX_GENERATED_IMAGES`.
    images: BTreeSet<u32>,
    next_image: u32,
    /// Open notifications and running progresses by id, with their owners (§M), and the
    /// accepted `notify` and `progress-start` calls of the rolling window.
    open_notifications: BTreeMap<u64, String>,
    running_progresses: BTreeMap<u64, String>,
    next_notification: u64,
    next_progress: u64,
    notify_times: VecDeque<Instant>,
    progress_start_times: VecDeque<Instant>,
    /// `notify` effects and `progress-update` calls of the running call.
    notification_effects: usize,
    progress_updates: usize,
    /// `network:fetch` (0.3): the manifest's domains, the host's counters, accepted requests not
    /// delivered yet, the ones the running call made, and the broker's state at the import.
    network_domains: DomainList,
    fetch_counters: FetchCounters,
    fetches: BTreeMap<u64, QueuedFetch>,
    call_fetches: Vec<u64>,
    /// The broker's state at the import, for every T2 capability.
    network_permission: PermissionState,
    /// `process:spawn` (0.3): the manifest's programs and environment names, the host's
    /// counters, the processes not finished yet, and the ones the running call spawned. The
    /// harness has one worktree, trusted unless a test says otherwise.
    process_policy: ProcessPolicy,
    process_counters: ProcessCounters,
    processes: BTreeMap<u64, FakeProcess>,
    call_spawns: Vec<u64>,
    worktree: Option<bool>,
    /// `workspace:edit` (0.3): edits accepted and not delivered yet, and the running call's.
    edit_counters: EditCounters,
    edits: BTreeMap<u64, WorkspaceEdit>,
    call_edits: Vec<u64>,
    /// `agent:observe` (0.3): whether the host has agent reviews here, and what the running call
    /// asked for (`Some(true)` start, `Some(false)` stop), applied after it.
    agent_reviews_available: bool,
    observe_op: Option<bool>,
    /// `permissions` (0.3): the states the running call saw (the first query of a capability
    /// fixes its answer for the call), and the prompts it asked for.
    permission_mirror: BTreeMap<&'static str, PermissionState>,
    permission_requests: Vec<&'static str>,
}

/// A process `spawn` accepted, until its `process-exited` or `spawn-failed` is delivered.
#[derive(Clone, Debug)]
pub(crate) struct FakeProcess {
    pub(crate) request: SpawnRequest,
    /// The program is not in the manifest: the outcome is `spawn-failed(not-found)`.
    pub(crate) not_declared: bool,
    /// Started by `Harness::deliver_io`, which ran it once stdin is closed.
    pub(crate) started: bool,
    pub(crate) stdin_open: bool,
    pub(crate) stdin: Vec<u8>,
    pub(crate) killed: bool,
}

/// A request `fetch` accepted, waiting for `Harness::deliver_io`.
#[derive(Clone, Debug)]
pub(crate) struct QueuedFetch {
    pub(crate) request: FetchRequest,
    pub(crate) cancelled: bool,
}

impl ApiState {
    pub(crate) fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    pub(crate) fn new(version: ApiVersion, capabilities: Vec<String>) -> Self {
        let live = LiveIds::new(version, &[], &capabilities)
            .expect("an empty surface set passes every check");
        Self {
            version,
            capabilities,
            live,
            shown_modals: BTreeSet::new(),
            activation: Activation::None,
            effects: Vec::new(),
            pushes: 0,
            pushed_items: 0,
            decoration_effects: 0,
            images: BTreeSet::new(),
            next_image: 1,
            open_notifications: BTreeMap::new(),
            running_progresses: BTreeMap::new(),
            next_notification: 1,
            next_progress: 1,
            notify_times: VecDeque::new(),
            progress_start_times: VecDeque::new(),
            notification_effects: 0,
            progress_updates: 0,
            network_domains: DomainList::default(),
            fetch_counters: FetchCounters::default(),
            fetches: BTreeMap::new(),
            call_fetches: Vec::new(),
            // As the host without a broker: no T2 capability is granted.
            network_permission: PermissionState::Unavailable,
            process_policy: ProcessPolicy::default(),
            process_counters: ProcessCounters::default(),
            processes: BTreeMap::new(),
            call_spawns: Vec::new(),
            worktree: Some(true),
            edit_counters: EditCounters::default(),
            edits: BTreeMap::new(),
            call_edits: Vec::new(),
            agent_reviews_available: true,
            observe_op: None,
            permission_mirror: BTreeMap::new(),
            permission_requests: Vec::new(),
        }
    }

    /// The static surfaces from `surfaces()`, checked as their version checks them. In 0.2 they
    /// join the surfaces `add-surface` registered during `surfaces()`.
    pub(crate) fn register_static(&mut self, surfaces: &[model::Surface]) -> Result<(), ApiError> {
        match self.version {
            ApiVersion::V0_1_0 => {
                self.live = LiveIds::new(self.version, surfaces, &self.capabilities)?;
            }
            // 0.3 registers static surfaces as 0.2 does. The harness refuses 0.4 components before
            // this point; the arm keeps the match total.
            ApiVersion::V0_2_0 | ApiVersion::V0_3_0 | ApiVersion::V0_4_0 => {
                for surface in surfaces {
                    self.live.add_surface(surface, &self.capabilities)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn live(&self) -> &LiveIds {
        &self.live
    }

    pub(crate) fn begin_call(&mut self) {
        self.effects.clear();
        self.pushes = 0;
        self.pushed_items = 0;
        self.decoration_effects = 0;
        self.notification_effects = 0;
        self.progress_updates = 0;
        self.call_fetches.clear();
        self.call_spawns.clear();
        self.call_edits.clear();
        self.observe_op = None;
        self.permission_mirror.clear();
        self.permission_requests.clear();
        if let Activation::Until(until) = self.activation
            && Instant::now() >= until
        {
            self.activation = Activation::None;
        }
    }

    /// Before `handle-event`: a click or Enter grants an activation, and a dismissed modal is no
    /// longer shown.
    pub(crate) fn begin_event(&mut self, surface_id: &str, event: &model::UiEvent) {
        match event {
            model::UiEvent::Click(_)
            | model::UiEvent::RowClick(..)
            | model::UiEvent::InputSubmitted(..)
            | model::UiEvent::CanvasClick(_) => self.activation = Activation::DuringCall,
            model::UiEvent::Dismissed(_) => {
                self.shown_modals.remove(surface_id);
            }
            // §M: a clicked notification action is a gesture, and it, like a closed
            // notification, frees the notification's open slot.
            model::UiEvent::NotificationAction(action) => {
                self.activation = Activation::DuringCall;
                self.open_notifications.remove(&action.notification);
            }
            model::UiEvent::NotificationClosed(id, _) => {
                self.open_notifications.remove(id);
            }
            model::UiEvent::InputChanged(..)
            | model::UiEvent::RowSelected(..)
            | model::UiEvent::ProgressCancelRequested(_) => {}
        }
    }

    /// The owner is gone: its notifications close and its progresses end, without events.
    fn forget_owner(&mut self, owner: &str) {
        self.open_notifications.retain(|_, open| open != owner);
        self.running_progresses
            .retain(|_, running| running != owner);
    }

    /// After a call: its effects, in order. An activation it granted starts counting down.
    pub(crate) fn finish_call(&mut self) -> Vec<HostEffect> {
        if self.activation == Activation::DuringCall {
            self.activation = Activation::Until(Instant::now() + ACTIVATION_WINDOW);
        }
        std::mem::take(&mut self.effects)
    }

    /// The renderer removed a decoration whose tree did not fit and whose remount failed.
    pub(crate) fn decoration_dropped(&mut self, id: &str) {
        self.live.remove_decoration(id, &self.capabilities).ok();
    }

    fn has_activation(&self) -> bool {
        match self.activation {
            Activation::None => false,
            Activation::DuringCall => true,
            Activation::Until(until) => Instant::now() < until,
        }
    }

    /// A live surface's kind, or the error for a decoration id or an unknown one.
    fn surface_kind(
        &self,
        surface_id: &str,
        what: &str,
    ) -> Result<&model::SurfaceKind, model::HostError> {
        if self.live.is_decoration(surface_id) {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!("{what} does not work for decoration {surface_id:?}"),
            ));
        }
        self.live.surface_kind(surface_id).ok_or_else(|| {
            api_error(ApiError::UnknownSurface {
                id: surface_id.to_owned(),
            })
        })
    }

    pub(crate) fn push_update(
        &mut self,
        surface_id: String,
        update: model::Update,
    ) -> Result<(), model::HostError> {
        if self.live.surface_kind(&surface_id).is_none() && !self.live.is_decoration(&surface_id) {
            return Err(api_error(ApiError::UnknownSurface { id: surface_id }));
        }
        if self.pushes >= MAX_PUSHES_PER_CALL {
            return Err(host_error(
                model::HostErrorCode::LimitExceeded,
                format!("at most {MAX_PUSHES_PER_CALL} pushes per call"),
            ));
        }
        update
            .check_caps(self.version.tree_limits())
            .map_err(|error| host_error(model::HostErrorCode::LimitExceeded, error.to_string()))?;
        self.check_images(&surface_id, update_props(&update))?;
        let items = match &update {
            model::Update::Unchanged => 0,
            model::Update::Replace(nodes) => nodes.len(),
            model::Update::Patches(patches) => patches.len(),
        };
        if self.pushed_items + items > MAX_PUSHED_ITEMS_PER_CALL_V0_2 {
            return Err(host_error(
                model::HostErrorCode::LimitExceeded,
                format!(
                    "the pushes of one call carry at most {MAX_PUSHED_ITEMS_PER_CALL_V0_2} \
                     patches and nodes together"
                ),
            ));
        }
        self.pushes += 1;
        self.pushed_items += items;
        self.effects.push(HostEffect::Push { surface_id, update });
        Ok(())
    }

    pub(crate) fn request_tick(
        &mut self,
        surface_id: String,
        delay_ms: u32,
    ) -> Result<(), model::HostError> {
        let kind = self.surface_kind(&surface_id, "request-tick")?;
        let delay = Duration::from_millis(delay_ms.into());
        if delay > MAX_TICK_DELAY {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!("a tick delay is at most {MAX_TICK_DELAY:?}, got {delay_ms} ms"),
            ));
        }
        if matches!(kind, model::SurfaceKind::Modal(_)) && !self.shown_modals.contains(&surface_id)
        {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!("modal {surface_id:?} is not shown"),
            ));
        }
        self.effects.push(HostEffect::RequestTick {
            surface_id,
            delay: delay.max(MIN_TICK_DELAY),
        });
        Ok(())
    }

    pub(crate) fn cancel_tick(&mut self, surface_id: String) {
        if self.live.surface_kind(&surface_id).is_some() {
            self.effects.push(HostEffect::CancelTick { surface_id });
        }
    }

    pub(crate) fn add_surface(&mut self, surface: model::Surface) -> Result<(), model::HostError> {
        self.live
            .add_surface(&surface, &self.capabilities)
            .map_err(api_error)?;
        self.effects.push(HostEffect::AddSurface(surface));
        Ok(())
    }

    pub(crate) fn remove_surface(&mut self, surface_id: String) -> Result<(), model::HostError> {
        self.live.remove_surface(&surface_id).map_err(api_error)?;
        self.shown_modals.remove(&surface_id);
        self.forget_owner(&surface_id);
        self.effects.push(HostEffect::RemoveSurface { surface_id });
        Ok(())
    }

    pub(crate) fn show_surface(&mut self, surface_id: String) -> Result<(), model::HostError> {
        match self.surface_kind(&surface_id, "show-surface")? {
            model::SurfaceKind::Panel(_) => {}
            model::SurfaceKind::StatusItem(_) => {
                return Err(host_error(
                    model::HostErrorCode::InvalidArgument,
                    format!("status item {surface_id:?} is always shown"),
                ));
            }
            model::SurfaceKind::Modal(_) => {
                if self.shown_modals.contains(&surface_id) {
                    return Ok(());
                }
                if !self.has_activation() {
                    return Err(host_error(
                        model::HostErrorCode::NotPermitted,
                        format!(
                            "showing modal {surface_id:?} needs a click or Enter: during \
                             handle-event for click, row-click or input-submitted, or within \
                             {ACTIVATION_WINDOW:?} after it, once"
                        ),
                    ));
                }
                self.activation = Activation::None;
                self.shown_modals.insert(surface_id.clone());
            }
        }
        self.effects.push(HostEffect::ShowSurface { surface_id });
        Ok(())
    }

    pub(crate) fn hide_surface(&mut self, surface_id: String) -> Result<(), model::HostError> {
        match self.surface_kind(&surface_id, "hide-surface")? {
            model::SurfaceKind::Panel(_) => {}
            model::SurfaceKind::StatusItem(_) => {
                return Err(host_error(
                    model::HostErrorCode::InvalidArgument,
                    format!("status item {surface_id:?} cannot be hidden"),
                ));
            }
            model::SurfaceKind::Modal(_) => {
                if !self.shown_modals.remove(&surface_id) {
                    return Ok(());
                }
            }
        }
        self.effects.push(HostEffect::HideSurface { surface_id });
        Ok(())
    }

    /// Checked in design.md's order (L): the capability, the id, no live surface with it, the
    /// path, the node count and the live cap (all `LiveIds`), then the per-call cap.
    pub(crate) fn place_decoration(
        &mut self,
        id: String,
        location: model::DecorationLocation,
        nodes: Vec<model::Node>,
    ) -> Result<(), model::HostError> {
        let placement = self
            .live
            .place_decoration(&id, &location, nodes.len(), &self.capabilities)
            .map_err(api_error)?;
        if self.decoration_effects >= MAX_DECORATION_EFFECTS_PER_CALL {
            if placement == Placement::New {
                self.live
                    .remove_decoration(&id, &self.capabilities)
                    .expect("just placed");
            }
            return Err(decoration_effects_exceeded());
        }
        self.decoration_effects += 1;
        self.effects.push(HostEffect::PlaceDecoration {
            id,
            location,
            nodes,
        });
        Ok(())
    }

    /// The per-call cap comes first, so a refused removal leaves the live set alone.
    pub(crate) fn remove_decoration(&mut self, id: String) -> Result<(), model::HostError> {
        if self.decoration_effects >= MAX_DECORATION_EFFECTS_PER_CALL {
            return Err(decoration_effects_exceeded());
        }
        self.live
            .remove_decoration(&id, &self.capabilities)
            .map_err(api_error)?;
        self.forget_owner(&id);
        self.decoration_effects += 1;
        self.effects.push(HostEffect::RemoveDecoration { id });
        Ok(())
    }
}

impl ApiState {
    /// `images.add-image` (0.3): declared `ui:images`, at most
    /// `ui_ext::image::MAX_GENERATED_IMAGE_BYTES`, a PNG header within the side cap or a plain
    /// SVG, and fewer than `MAX_GENERATED_IMAGES` live images. Handles are never reused.
    pub(crate) fn add_image(
        &mut self,
        format: ImageFormat,
        bytes: Vec<u8>,
    ) -> Result<u32, model::HostError> {
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == image::CAPABILITY_IMAGES)
        {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!(
                    "images need {:?} in [fyx_ui].capabilities",
                    image::CAPABILITY_IMAGES
                ),
            ));
        }
        image::check_image_bytes(format, &bytes, image::MAX_GENERATED_IMAGE_BYTES)
            .map_err(|error| host_error(error.code(), error.to_string()))?;
        if self.images.len() >= image::MAX_GENERATED_IMAGES {
            let error = image::ImageError::TooManyImages;
            return Err(host_error(error.code(), error.to_string()));
        }
        let handle = self.next_image;
        self.next_image = handle.checked_add(1).ok_or_else(|| {
            host_error(
                model::HostErrorCode::LimitExceeded,
                "this instance has used up its image handles",
            )
        })?;
        self.images.insert(handle);
        self.effects.push(HostEffect::AddImage {
            handle,
            format,
            bytes: bytes.into(),
        });
        Ok(handle)
    }

    /// `images.remove-image`: unknown handles are ignored.
    pub(crate) fn remove_image(&mut self, handle: u32) {
        if self.images.remove(&handle) {
            self.effects.push(HostEffect::RemoveImage { handle });
        }
    }

    /// The `image` nodes among `props`, checked when an update or a reply for `surface_id` is
    /// applied (design 0.3, section 12): the props rules, panels only, and a `generated` handle
    /// that `add-image` returned earlier (decision D3). A handle removed since is fine: the
    /// renderer draws the alt text for it (D2).
    pub(crate) fn check_images<'a>(
        &self,
        surface_id: &str,
        props: impl IntoIterator<Item = &'a model::Props>,
    ) -> Result<(), model::HostError> {
        for props in props {
            if let model::Props::Canvas(canvas) = props {
                ui_ext::canvas::check_canvas_props(canvas)
                    .and_then(|()| {
                        ui_ext::canvas::check_canvas_placement(self.live.surface_kind(surface_id))
                    })
                    .map_err(|error| host_error(error.code(), error.to_string()))?;
                continue;
            }
            let model::Props::Image(props) = props else {
                continue;
            };
            image::check_image_props(props)
                .and_then(|()| image::check_image_placement(self.live.surface_kind(surface_id)))
                .map_err(|error| host_error(error.code(), error.to_string()))?;
            if let image::ImageSource::Generated(handle) = props.source
                && (handle == 0 || handle >= self.next_image)
            {
                return Err(host_error(
                    model::HostErrorCode::InvalidArgument,
                    format!("image handle {handle} was not returned by add-image"),
                ));
            }
        }
        Ok(())
    }

    /// Whether `add-image` ever returned `handle` and it is still live.
    pub(crate) fn image_is_live(&self, handle: u32) -> bool {
        self.images.contains(&handle)
    }
}

/// Every props value an update carries.
pub(crate) fn update_props(update: &model::Update) -> Vec<&model::Props> {
    match update {
        model::Update::Unchanged => Vec::new(),
        model::Update::Replace(nodes) => nodes.iter().map(|node| &node.props).collect(),
        model::Update::Patches(patches) => patches
            .iter()
            .filter_map(|patch| match patch {
                model::Patch::SetProps(_, props) => Some(props),
                model::Patch::Insert(insert) => Some(&insert.node.props),
                model::Patch::Remove(_) | model::Patch::InvalidateRows(_) => None,
            })
            .collect(),
    }
}

/// Notifications and progress (design.md M), as the host checks them (`ui_ext_wasm`'s
/// `ApiState`), with the shared rules of `ui_ext::v0_2::notifications`.
impl ApiState {
    fn require_notifications(&self) -> Result<(), model::HostError> {
        if self
            .capabilities
            .iter()
            .any(|capability| capability == notify::CAPABILITY_NOTIFICATIONS)
        {
            Ok(())
        } else {
            Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!(
                    "notifications and progress need {:?} in [fyx_ui].capabilities",
                    notify::CAPABILITY_NOTIFICATIONS
                ),
            ))
        }
    }

    /// A live surface or decoration of the extension.
    fn require_owner(&self, owner: &str) -> Result<(), model::HostError> {
        if self.live.surface_kind(owner).is_some() || self.live.is_decoration(owner) {
            Ok(())
        } else {
            Err(api_error(ApiError::UnknownSurface {
                id: owner.to_owned(),
            }))
        }
    }

    /// Drops the times that left the rolling window and says whether another one fits.
    fn window_has_room(times: &mut VecDeque<Instant>, max: usize) -> bool {
        let now = Instant::now();
        while times
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= notify::RATE_WINDOW)
        {
            times.pop_front();
        }
        times.len() < max
    }

    /// `notify`: the capability, the owner, the text rules, the per-call cap, the open cap, the
    /// rate cap. Past a cap the error is `limit-exceeded`.
    pub(crate) fn notify(&mut self, notification: Notification) -> Result<u64, model::HostError> {
        self.require_notifications()?;
        self.require_owner(&notification.owner)?;
        notification
            .validate()
            .map_err(|error| host_error(model::HostErrorCode::InvalidArgument, error.0))?;
        let limit = |message: String| host_error(model::HostErrorCode::LimitExceeded, message);
        if self.notification_effects >= notify::MAX_NOTIFICATION_EFFECTS_PER_CALL {
            return Err(limit(format!(
                "at most {} notifications per call",
                notify::MAX_NOTIFICATION_EFFECTS_PER_CALL
            )));
        }
        if self.open_notifications.len() >= notify::MAX_OPEN_NOTIFICATIONS {
            return Err(limit(format!(
                "at most {} open notifications per extension",
                notify::MAX_OPEN_NOTIFICATIONS
            )));
        }
        if !Self::window_has_room(&mut self.notify_times, notify::MAX_NOTIFY_PER_WINDOW) {
            return Err(limit(format!(
                "at most {} notifications per {} s per extension",
                notify::MAX_NOTIFY_PER_WINDOW,
                notify::RATE_WINDOW.as_secs()
            )));
        }
        let id = self.next_notification;
        self.next_notification += 1;
        self.notify_times.push_back(Instant::now());
        self.notification_effects += 1;
        self.open_notifications
            .insert(id, notification.owner.clone());
        self.effects.push(HostEffect::Notify { id, notification });
        Ok(id)
    }

    /// `dismiss-notification`: unknown or closed ids are ignored.
    pub(crate) fn dismiss_notification(&mut self, id: u64) {
        if self.open_notifications.remove(&id).is_some() {
            self.effects.push(HostEffect::DismissNotification { id });
        }
    }

    /// `progress-start`: the capability, the owner (a panel for `panel`), the title, the running
    /// cap, the rate cap.
    pub(crate) fn progress_start(
        &mut self,
        options: ProgressOptions,
    ) -> Result<u64, model::HostError> {
        self.require_notifications()?;
        self.require_owner(&options.owner)?;
        if options.location == notify::ProgressLocation::Panel
            && !matches!(
                self.live.surface_kind(&options.owner),
                Some(model::SurfaceKind::Panel(_))
            )
        {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!(
                    "a `panel` progress needs a panel as its owner, and {:?} is not one",
                    options.owner
                ),
            ));
        }
        options
            .validate()
            .map_err(|error| host_error(model::HostErrorCode::InvalidArgument, error.0))?;
        if self.running_progresses.len() >= notify::MAX_RUNNING_PROGRESSES {
            return Err(host_error(
                model::HostErrorCode::LimitExceeded,
                format!(
                    "at most {} running progresses per extension",
                    notify::MAX_RUNNING_PROGRESSES
                ),
            ));
        }
        if !Self::window_has_room(
            &mut self.progress_start_times,
            notify::MAX_PROGRESS_STARTS_PER_WINDOW,
        ) {
            return Err(host_error(
                model::HostErrorCode::LimitExceeded,
                format!(
                    "at most {} progress starts per {} s per extension",
                    notify::MAX_PROGRESS_STARTS_PER_WINDOW,
                    notify::RATE_WINDOW.as_secs()
                ),
            ));
        }
        let id = self.next_progress;
        self.next_progress += 1;
        self.progress_start_times.push_back(Instant::now());
        self.running_progresses.insert(id, options.owner.clone());
        self.effects.push(HostEffect::ProgressStart { id, options });
        Ok(id)
    }

    /// `progress-update`: a running id, a valid value and message, at most
    /// `MAX_PROGRESS_UPDATES_PER_CALL` per call.
    pub(crate) fn progress_update(
        &mut self,
        id: u64,
        value: ProgressValue,
        message: Option<String>,
    ) -> Result<(), model::HostError> {
        self.require_notifications()?;
        if !self.running_progresses.contains_key(&id) {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!("progress {id} is not running"),
            ));
        }
        notify::check_progress_update(value, message.as_deref())
            .map_err(|error| host_error(model::HostErrorCode::InvalidArgument, error.0))?;
        if self.progress_updates >= notify::MAX_PROGRESS_UPDATES_PER_CALL {
            return Err(host_error(
                model::HostErrorCode::LimitExceeded,
                format!(
                    "at most {} progress updates per call",
                    notify::MAX_PROGRESS_UPDATES_PER_CALL
                ),
            ));
        }
        self.progress_updates += 1;
        self.effects
            .push(HostEffect::ProgressUpdate { id, value, message });
        Ok(())
    }

    /// `progress-end`: unknown or ended ids are ignored.
    pub(crate) fn progress_end(&mut self, id: u64) {
        if self.running_progresses.remove(&id).is_some() {
            self.effects.push(HostEffect::ProgressEnd { id });
        }
    }
}

/// `network:fetch` (design.md §8), as the host's import checks it: the capability, the broker's
/// state, the domain list, `ui_ext::network::check_request`, then the shared counters.
impl ApiState {
    pub(crate) fn set_network_domains(&mut self, domains: DomainList) {
        self.network_domains = domains;
    }

    pub(crate) fn set_network_permission(&mut self, state: PermissionState) {
        self.network_permission = state;
    }

    pub(crate) fn fetch(&mut self, request: FetchRequest) -> Result<u64, model::HostError> {
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == net::CAPABILITY_NETWORK)
        {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!("{:?} is not in the manifest", net::CAPABILITY_NETWORK),
            ));
        }
        // `ask` is accepted: the prompt comes when the request is applied.
        match self.network_permission {
            PermissionState::Granted | PermissionState::Ask => {}
            state @ (PermissionState::Denied | PermissionState::Unavailable) => {
                return Err(host_error(
                    model::HostErrorCode::Denied,
                    format!("{} is {state:?}", net::CAPABILITY_NETWORK),
                ));
            }
        }
        if self.network_domains.is_empty() {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!(
                    "fetch needs {:?} and [fyx_ui.network] domains in the manifest",
                    net::CAPABILITY_NETWORK
                ),
            ));
        }
        net::check_request(&request, &self.network_domains)?;
        let id = self.fetch_counters.start(Instant::now())?;
        self.fetches.insert(
            id,
            QueuedFetch {
                request,
                cancelled: false,
            },
        );
        self.call_fetches.push(id);
        Ok(id)
    }

    /// `cancel-fetch`: unknown or delivered ids are ignored; a waiting one ends `cancelled`.
    pub(crate) fn cancel_fetch(&mut self, id: u64) {
        if let Some(fetch) = self.fetches.get_mut(&id) {
            fetch.cancelled = true;
        }
    }

    /// A call that trapped or ran over: its requests are dropped without an outcome, and they
    /// do not count, as in the host.
    pub(crate) fn abort_call(&mut self) {
        for id in std::mem::take(&mut self.call_fetches) {
            self.fetches.remove(&id);
            self.fetch_counters.finished(id);
        }
        for id in std::mem::take(&mut self.call_spawns) {
            self.processes.remove(&id);
            self.process_counters.finished(id);
        }
        for id in std::mem::take(&mut self.call_edits) {
            self.edits.remove(&id);
            self.edit_counters.finished(id);
        }
        self.observe_op = None;
        self.permission_requests.clear();
    }

    pub(crate) fn queued_fetches(&self) -> impl Iterator<Item = (u64, &QueuedFetch)> {
        self.fetches.iter().map(|(id, fetch)| (*id, fetch))
    }

    /// The oldest accepted request, for delivery; it stops counting as in flight, as the host
    /// does before `handle-io`.
    pub(crate) fn take_fetch(&mut self) -> Option<(u64, QueuedFetch)> {
        let (id, fetch) = self.fetches.pop_first()?;
        self.fetch_counters.finished(id);
        Some((id, fetch))
    }
}

/// `process:spawn` (design.md §9), as the host's import checks it: the capability and the
/// broker's state, the manifest's programs, `ui_ext::process::check_spawn`, the worktree, then the
/// shared counters.
impl ApiState {
    pub(crate) fn set_process_policy(&mut self, policy: ProcessPolicy) {
        self.process_policy = policy;
    }

    /// `None`: no worktree is open; `Some(trusted)`.
    pub(crate) fn set_worktree(&mut self, worktree: Option<bool>) {
        self.worktree = worktree;
    }

    fn permit_process(&self) -> Result<(), model::HostError> {
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == proc::CAPABILITY_PROCESS)
        {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!("{:?} is not in the manifest", proc::CAPABILITY_PROCESS),
            ));
        }
        match self.network_permission {
            PermissionState::Granted | PermissionState::Ask => Ok(()),
            state => Err(host_error(
                model::HostErrorCode::Denied,
                format!("{} is {state:?}", proc::CAPABILITY_PROCESS),
            )),
        }
    }

    pub(crate) fn spawn(&mut self, request: SpawnRequest) -> Result<u64, model::HostError> {
        self.permit_process()?;
        if self.process_policy.programs.is_empty() {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!(
                    "spawn needs {:?} and [[fyx_ui.processes]] in the manifest",
                    proc::CAPABILITY_PROCESS
                ),
            ));
        }
        let not_declared = matches!(
            proc::check_spawn(&request, &self.process_policy)?,
            SpawnCheck::NotDeclared
        );
        match self.worktree {
            None => {
                return Err(host_error(
                    model::HostErrorCode::NoWorktree,
                    "no worktree is open",
                ));
            }
            Some(false) => {
                return Err(host_error(
                    model::HostErrorCode::UntrustedWorktree,
                    "the worktree is not trusted",
                ));
            }
            Some(true) => {}
        }
        let id = self.process_counters.start()?;
        self.processes.insert(
            id,
            FakeProcess {
                stdin_open: request.stdin,
                request,
                not_declared,
                started: false,
                stdin: Vec::new(),
                killed: false,
            },
        );
        self.call_spawns.push(id);
        Ok(id)
    }

    pub(crate) fn write_stdin(&mut self, id: u64, data: Vec<u8>) -> Result<(), model::HostError> {
        self.permit_process()?;
        if data.len() > proc::MAX_STDIN_WRITE_BYTES {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!(
                    "at most {} bytes per write-stdin",
                    proc::MAX_STDIN_WRITE_BYTES
                ),
            ));
        }
        let process = self.live_process(id)?;
        if !process.stdin_open {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!(
                    "process {id} has no stdin open (spawn it with stdin, and do not close it)"
                ),
            ));
        }
        if process.stdin.len() + data.len() > proc::MAX_STDIN_QUEUED_BYTES {
            return Err(host_error(
                model::HostErrorCode::LimitExceeded,
                format!(
                    "at most {} bytes queued for stdin",
                    proc::MAX_STDIN_QUEUED_BYTES
                ),
            ));
        }
        process.stdin.extend(data);
        Ok(())
    }

    pub(crate) fn close_stdin(&mut self, id: u64) -> Result<(), model::HostError> {
        self.live_process(id)?.stdin_open = false;
        Ok(())
    }

    /// An id that exited (or never was) is `invalid-argument`.
    pub(crate) fn kill(&mut self, id: u64) -> Result<(), model::HostError> {
        if !self.process_counters.is_live(id) {
            return Err(host_error(
                model::HostErrorCode::InvalidArgument,
                format!("process {id} has exited"),
            ));
        }
        self.live_process(id)?.killed = true;
        Ok(())
    }

    fn live_process(&mut self, id: u64) -> Result<&mut FakeProcess, model::HostError> {
        self.processes.get_mut(&id).ok_or_else(|| {
            host_error(
                model::HostErrorCode::InvalidArgument,
                format!("process {id} has exited"),
            )
        })
    }

    pub(crate) fn processes(&self) -> impl Iterator<Item = (u64, &FakeProcess)> {
        self.processes.iter().map(|(id, process)| (*id, process))
    }

    pub(crate) fn process_mut(&mut self, id: u64) -> Option<&mut FakeProcess> {
        self.processes.get_mut(&id)
    }

    /// Its `process-exited` or `spawn-failed` is about to be delivered.
    pub(crate) fn process_finished(&mut self, id: u64) -> Option<FakeProcess> {
        self.process_counters.finished(id);
        self.processes.remove(&id)
    }
}

/// `workspace:edit` and `agent:observe` (design.md §10, §11), as the host checks them: the
/// capability and the broker's state, then `ui_ext::edit::check_edit` and the in-flight cap, or
/// whether agent reviews exist here.
impl ApiState {
    fn permit_t2(&self, capability: &str) -> Result<(), model::HostError> {
        if !self.capabilities.iter().any(|name| name == capability) {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!("{capability:?} is not in the manifest"),
            ));
        }
        match self.network_permission {
            PermissionState::Granted | PermissionState::Ask => Ok(()),
            state => Err(host_error(
                model::HostErrorCode::Denied,
                format!("{capability} is {state:?}"),
            )),
        }
    }

    pub(crate) fn apply_edit(&mut self, edit: WorkspaceEdit) -> Result<u64, model::HostError> {
        self.permit_t2(ui_ext::edit::CAPABILITY_EDIT)?;
        if self.worktree.is_none() {
            return Err(host_error(
                model::HostErrorCode::Unavailable,
                "no workspace is open",
            ));
        }
        ui_ext::edit::check_edit(&edit)?;
        let id = self.edit_counters.start()?;
        self.edits.insert(id, edit);
        self.call_edits.push(id);
        Ok(id)
    }

    /// The oldest accepted edit, for delivery; it stops counting as in flight.
    pub(crate) fn take_edit(&mut self) -> Option<(u64, WorkspaceEdit)> {
        let (id, edit) = self.edits.pop_first()?;
        self.edit_counters.finished(id);
        Some((id, edit))
    }

    pub(crate) fn set_agent_reviews_available(&mut self, available: bool) {
        self.agent_reviews_available = available;
    }

    pub(crate) fn observe_start(&mut self) -> Result<(), model::HostError> {
        self.permit_t2(ui_ext::observe::CAPABILITY_OBSERVE)?;
        if !self.agent_reviews_available {
            return Err(host_error(
                model::HostErrorCode::Unavailable,
                "agent reviews are not available here",
            ));
        }
        self.observe_op = Some(true);
        Ok(())
    }

    pub(crate) fn observe_stop(&mut self) {
        self.observe_op = Some(false);
    }

    /// `permissions.query`: `unavailable` for an undeclared capability, else the broker's
    /// state as of the first query of the call.
    pub(crate) fn query_permission(&mut self, capability: &'static str) -> PermissionState {
        if !self.capabilities.iter().any(|name| name == capability) {
            return PermissionState::Unavailable;
        }
        *self
            .permission_mirror
            .entry(capability)
            .or_insert(self.network_permission)
    }

    /// `permissions.request`: declared, and the click-or-Enter activation, which it uses up.
    /// Only `ask` has a prompt, shown after the call, once per capability per call.
    pub(crate) fn request_permission(
        &mut self,
        capability: &'static str,
    ) -> Result<(), model::HostError> {
        if !self.capabilities.iter().any(|name| name == capability) {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                format!("{capability:?} is not in the manifest"),
            ));
        }
        if !self.take_activation() {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                "permissions.request needs a click or Enter: during handle-event for click, \
                 row-click, input-submitted, canvas-click or notification-action, or shortly \
                 after it, once",
            ));
        }
        if self.query_permission(capability) == PermissionState::Ask
            && !self.permission_requests.contains(&capability)
        {
            self.permission_requests.push(capability);
        }
        Ok(())
    }

    /// The prompts the call that just finished asked for.
    pub(crate) fn take_permission_requests(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.permission_requests)
    }

    /// 0.4 `settings.open`: only while the user acts on one of the extension's surfaces
    /// (`not-permitted` otherwise), and only for a declared key (`invalid-argument`).
    pub(crate) fn open_settings(
        &mut self,
        settings: &crate::settings::FakeSettings,
        key: Option<String>,
    ) -> Result<(), model::HostError> {
        if !self.take_activation() {
            return Err(host_error(
                model::HostErrorCode::NotPermitted,
                "settings.open needs the user acting on one of the extension's surfaces",
            ));
        }
        if let Some(key) = &key
            && settings.get(key).is_none()
        {
            return Err(undeclared_setting(key));
        }
        self.effects.push(HostEffect::OpenSettings { key });
        Ok(())
    }

    fn take_activation(&mut self) -> bool {
        let had = self.has_activation();
        if had {
            self.activation = Activation::None;
        }
        had
    }

    /// What the call that just finished asked of the agent feed.
    pub(crate) fn take_observe_op(&mut self) -> Option<bool> {
        self.observe_op.take()
    }
}

fn decoration_effects_exceeded() -> model::HostError {
    host_error(
        model::HostErrorCode::LimitExceeded,
        format!("at most {MAX_DECORATION_EFFECTS_PER_CALL} decoration changes per call"),
    )
}
