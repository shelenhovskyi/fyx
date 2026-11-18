//! The effect list of a call and the per-extension state the 0.2 host imports are checked against
//! (design.md, A): every effectful import is checked when it is called, against the live surface
//! and decoration ids as of that point in the call, and its effect is recorded to be applied after
//! the call returns, in call order, before the call's own value.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use ui_ext::{
    ApiError, ApiVersion, LiveIds, MAX_DECORATION_EFFECTS_PER_CALL, Placement,
    image::{self, ImageFormat},
    v0_2::{
        self,
        notifications::{self as notify, Notification, ProgressOptions, ProgressValue},
    },
};

use crate::MAX_PUSHES_PER_CALL;

/// Patches plus nodes all of one call's 0.2 pushes may carry together; past it `push-update`
/// returns `limit-exceeded`. Two whole trees, so a guest can always resend one surface after a
/// failed push (a single `replace` may carry [`ui_ext::TreeLimits::max_nodes`]).
pub const MAX_PUSHED_ITEMS_PER_CALL_V0_2: usize = 2 * 16384;
/// Shorter tick delays are clamped up to this (design.md, B; the WIT states it).
pub const MIN_TICK_DELAY: Duration = Duration::from_millis(100);
/// Longer tick delays are refused with `invalid-argument`: it keeps `Instant` arithmetic safe and
/// catches unit mix-ups.
pub const MAX_TICK_DELAY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long after a click or Enter an extension may show a modal (design.md, C).
pub const ACTIVATION_WINDOW: Duration = Duration::from_secs(5);

/// Something a 0.2 host import asked for, checked at the call and applied after it.
///
/// A 0.1 component only ever produces [`HostEffect::Push`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEffect {
    /// `push-update`, to a surface or a decoration.
    Push {
        surface_id: String,
        update: v0_2::Update,
    },
    /// `request-tick`. `delay` is already clamped up to [`MIN_TICK_DELAY`]; the renderer applies
    /// the rest of design.md's tick rules ([`crate::ticks`]). It counts from when the effect is
    /// applied, and replaces any pending tick of the surface.
    RequestTick { surface_id: String, delay: Duration },
    /// `cancel-tick` of a live surface.
    CancelTick { surface_id: String },
    /// `add-surface`: add the panel or status item to every workspace and mount it; register a
    /// modal closed.
    AddSurface(v0_2::Surface),
    /// `remove-surface`: close its containers, drop its tree, tick and queued events. No event.
    RemoveSurface { surface_id: String },
    /// `show-surface`: open a modal in the active workspace window and mount it, or reveal a
    /// panel's dock without moving focus. For a modal, the gesture check has passed.
    ShowSurface { surface_id: String },
    /// `hide-surface`: close a modal (no `dismissed` event) or hide a panel's dock if that panel
    /// is active.
    HideSurface { surface_id: String },
    /// `place-decoration`: anchor `nodes` at `location`; a live id moves and its tree is replaced.
    PlaceDecoration {
        id: String,
        location: v0_2::DecorationLocation,
        nodes: Vec<v0_2::Node>,
    },
    /// `remove-decoration`: remove it in every editor. No event.
    RemoveDecoration { id: String },
    /// `images.add-image` (0.3): the bytes passed the size cap and the header checks of
    /// [`ui_ext::image::check_image_bytes`]; decode them off the main thread under the limits of
    /// design.md section 12. `handle` never repeats within an instance.
    AddImage {
        handle: u32,
        format: ImageFormat,
        bytes: Arc<[u8]>,
    },
    /// `images.remove-image` of a live handle: nodes that show it fall back to their alt text.
    RemoveImage { handle: u32 },
    /// `notifications.notify` (design.md M.1): show it in the active workspace window without
    /// moving focus, with the extension's provenance line. Its events go to `handle-event` of
    /// `notification.owner`.
    Notify { id: u64, notification: Notification },
    /// `dismiss-notification` of an open notification: close it, with no event.
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
}

/// The validated result of a call with the effects the guest made during it: apply `effects` in
/// order, then `value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectReply<T> {
    pub effects: Vec<HostEffect>,
    pub value: T,
}

/// A call that ran to completion but whose result was rejected (the extension's own error, or a
/// reply that failed validation). Its effects still apply: the guest already counts them as done.
/// Reach it with `error.downcast_ref::<EffectReplyError>()`; any other error (trap, time budget,
/// disabled extension) carries no effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectReplyError {
    pub effects: Vec<HostEffect>,
    pub message: String,
}

impl std::fmt::Display for EffectReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EffectReplyError {}

/// A transient activation (design.md, C): granted by `handle-event` for a click or Enter, it lasts
/// until [`ACTIVATION_WINDOW`] after that call returns, or until one modal is shown with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Activation {
    None,
    DuringCall,
    Until(Instant),
}

fn host_error(code: v0_2::HostErrorCode, message: impl Into<String>) -> v0_2::HostError {
    v0_2::HostError {
        code,
        message: message.into(),
    }
}

fn api_error(error: ApiError) -> v0_2::HostError {
    v0_2::HostError::from(&error)
}

/// One extension's live ids, capabilities, shown modals and activation, plus the effects of the
/// running call.
pub(crate) struct ApiState {
    version: ApiVersion,
    capabilities: Arc<[String]>,
    live: LiveIds,
    /// Modals shown as far as the host knows: shown by `show-surface`, closed by `hide-surface`,
    /// `remove-surface`, a `dismissed` event, or the renderer
    /// ([`crate::UiExtension::modal_closed`]).
    shown_modals: BTreeSet<String>,
    activation: Activation,
    activation_window: Duration,
    effects: Vec<HostEffect>,
    pushes: usize,
    pushed_items: usize,
    decoration_effects: usize,
    /// Live generated images (0.3), at most [`image::MAX_GENERATED_IMAGES`].
    images: BTreeSet<u32>,
    next_image: u32,
    /// Open notifications and running progresses by id, with their owners (§M).
    open_notifications: BTreeMap<u64, String>,
    running_progresses: BTreeMap<u64, String>,
    next_notification: u64,
    next_progress: u64,
    /// When the accepted `notify` and `progress-start` calls of the rolling window were made.
    notify_times: VecDeque<Instant>,
    progress_start_times: VecDeque<Instant>,
    /// `notify` effects and `progress-update` calls of the running call.
    notification_effects: usize,
    progress_updates: usize,
}

impl ApiState {
    pub(crate) fn new(version: ApiVersion, capabilities: Arc<[String]>) -> Self {
        let live = LiveIds::new(version, &[], &capabilities[..])
            .expect("an empty surface set passes every check");
        Self {
            version,
            capabilities,
            live,
            shown_modals: BTreeSet::new(),
            activation: Activation::None,
            activation_window: ACTIVATION_WINDOW,
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
        }
    }

    /// The static surfaces from `surfaces()`, checked as their version checks them. In 0.2 they
    /// join the surfaces `add-surface` registered during `surfaces()`, whose effects apply after
    /// the static ones.
    pub(crate) fn register_static(&mut self, surfaces: &[v0_2::Surface]) -> Result<(), ApiError> {
        match self.version {
            ApiVersion::V0_1_0 => {
                self.live = LiveIds::new(self.version, surfaces, &self.capabilities[..])?;
            }
            ApiVersion::V0_2_0 | ApiVersion::V0_3_0 => {
                for surface in surfaces {
                    self.live.add_surface(surface, &self.capabilities[..])?;
                }
            }
        }
        Ok(())
    }

    /// The kind of a live surface (not a decoration).
    pub(crate) fn live_surface_kind(&self, surface_id: &str) -> Option<&v0_2::SurfaceKind> {
        self.live.surface_kind(surface_id)
    }

    pub(crate) fn begin_call(&mut self) {
        self.effects.clear();
        self.pushes = 0;
        self.pushed_items = 0;
        self.decoration_effects = 0;
        self.notification_effects = 0;
        self.progress_updates = 0;
        if let Activation::Until(until) = self.activation
            && Instant::now() >= until
        {
            self.activation = Activation::None;
        }
    }

    /// Before `handle-event`: a click or Enter grants an activation, and a `dismissed` modal is
    /// no longer shown.
    /// A clicked notification action is a gesture too (design.md M.3), and it, like a closed
    /// notification, frees the notification's open slot.
    pub(crate) fn begin_event(&mut self, surface_id: &str, event: &v0_2::UiEvent) {
        match event {
            v0_2::UiEvent::Click(_)
            | v0_2::UiEvent::RowClick(..)
            | v0_2::UiEvent::InputSubmitted(..)
            | v0_2::UiEvent::CanvasClick(_) => self.activation = Activation::DuringCall,
            v0_2::UiEvent::Dismissed(_) => {
                self.shown_modals.remove(surface_id);
            }
            v0_2::UiEvent::NotificationAction(action) => {
                self.activation = Activation::DuringCall;
                self.open_notifications.remove(&action.notification);
            }
            v0_2::UiEvent::NotificationClosed(id, _) => {
                self.open_notifications.remove(id);
            }
            v0_2::UiEvent::InputChanged(..)
            | v0_2::UiEvent::RowSelected(..)
            | v0_2::UiEvent::ProgressCancelRequested(_) => {}
        }
    }

    /// The owner is gone: its notifications close and its progresses end, without events (the
    /// renderer does the same when it applies the removal).
    fn forget_owner(&mut self, owner: &str) {
        self.open_notifications.retain(|_, open| open != owner);
        self.running_progresses
            .retain(|_, running| running != owner);
    }

    /// Whether the manifest (or, without one, the load options) declares `capability`.
    pub(crate) fn declares(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|declared| declared == capability)
    }

    /// After a call: the effects it made, in order. An activation granted by it starts counting
    /// down.
    pub(crate) fn finish_call(&mut self) -> Vec<HostEffect> {
        if self.activation == Activation::DuringCall {
            self.activation = Activation::Until(Instant::now() + self.activation_window);
        }
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn modal_closed(&mut self, surface_id: &str) {
        self.shown_modals.remove(surface_id);
    }

    #[cfg(test)]
    pub(crate) fn set_activation_window(&mut self, window: Duration) {
        self.activation_window = window;
    }

    /// For a call that needs the activation and uses it up (`permissions.request`, §0.4):
    /// whether there was one; it is gone after this.
    pub(crate) fn take_activation(&mut self) -> bool {
        let had = self.has_activation();
        if had {
            self.activation = Activation::None;
        }
        had
    }

    fn has_activation(&self) -> bool {
        match self.activation {
            Activation::None => false,
            Activation::DuringCall => true,
            Activation::Until(until) => Instant::now() < until,
        }
    }

    /// A live surface's kind, or the error for an id that is a decoration or nothing.
    fn surface_kind(
        &self,
        surface_id: &str,
        what: &str,
    ) -> Result<&v0_2::SurfaceKind, v0_2::HostError> {
        if self.live.is_decoration(surface_id) {
            return Err(host_error(
                v0_2::HostErrorCode::InvalidArgument,
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
        update: v0_2::Update,
    ) -> Result<(), v0_2::HostError> {
        if self.live.surface_kind(&surface_id).is_none() && !self.live.is_decoration(&surface_id) {
            return Err(api_error(ApiError::UnknownSurface { id: surface_id }));
        }
        if self.pushes >= MAX_PUSHES_PER_CALL {
            return Err(host_error(
                v0_2::HostErrorCode::LimitExceeded,
                format!("at most {MAX_PUSHES_PER_CALL} pushes per call"),
            ));
        }
        let limits = self.version.tree_limits();
        update
            .check_caps(limits)
            .map_err(|error| host_error(v0_2::HostErrorCode::LimitExceeded, error.to_string()))?;
        self.check_images(&surface_id, update_props(&update))?;
        let items = match &update {
            v0_2::Update::Unchanged => 0,
            v0_2::Update::Replace(nodes) => nodes.len(),
            v0_2::Update::Patches(patches) => patches.len(),
        };
        if self.pushed_items + items > MAX_PUSHED_ITEMS_PER_CALL_V0_2 {
            return Err(host_error(
                v0_2::HostErrorCode::LimitExceeded,
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
    ) -> Result<(), v0_2::HostError> {
        let kind = self.surface_kind(&surface_id, "request-tick")?;
        let delay = Duration::from_millis(delay_ms.into());
        if delay > MAX_TICK_DELAY {
            return Err(host_error(
                v0_2::HostErrorCode::InvalidArgument,
                format!("a tick delay is at most {MAX_TICK_DELAY:?}, got {delay_ms} ms"),
            ));
        }
        if matches!(kind, v0_2::SurfaceKind::Modal(_)) && !self.shown_modals.contains(&surface_id) {
            return Err(host_error(
                v0_2::HostErrorCode::InvalidArgument,
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

    pub(crate) fn add_surface(&mut self, surface: v0_2::Surface) -> Result<(), v0_2::HostError> {
        self.live
            .add_surface(&surface, &self.capabilities[..])
            .map_err(api_error)?;
        self.effects.push(HostEffect::AddSurface(surface));
        Ok(())
    }

    pub(crate) fn remove_surface(&mut self, surface_id: String) -> Result<(), v0_2::HostError> {
        self.live.remove_surface(&surface_id).map_err(api_error)?;
        self.shown_modals.remove(&surface_id);
        self.forget_owner(&surface_id);
        self.effects.push(HostEffect::RemoveSurface { surface_id });
        Ok(())
    }

    pub(crate) fn show_surface(&mut self, surface_id: String) -> Result<(), v0_2::HostError> {
        match self.surface_kind(&surface_id, "show-surface")? {
            v0_2::SurfaceKind::Panel(_) => {}
            v0_2::SurfaceKind::StatusItem(_) => {
                return Err(host_error(
                    v0_2::HostErrorCode::InvalidArgument,
                    format!("status item {surface_id:?} is always shown"),
                ));
            }
            v0_2::SurfaceKind::Modal(_) => {
                if self.shown_modals.contains(&surface_id) {
                    return Ok(());
                }
                if !self.has_activation() {
                    return Err(host_error(
                        v0_2::HostErrorCode::NotPermitted,
                        format!(
                            "showing modal {surface_id:?} needs a click or Enter: during \
                             handle-event for click, row-click or input-submitted, or within \
                             {:?} after it, once",
                            self.activation_window
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

    pub(crate) fn hide_surface(&mut self, surface_id: String) -> Result<(), v0_2::HostError> {
        match self.surface_kind(&surface_id, "hide-surface")? {
            v0_2::SurfaceKind::Panel(_) => {}
            v0_2::SurfaceKind::StatusItem(_) => {
                return Err(host_error(
                    v0_2::HostErrorCode::InvalidArgument,
                    format!("status item {surface_id:?} cannot be hidden"),
                ));
            }
            v0_2::SurfaceKind::Modal(_) => {
                if !self.shown_modals.remove(&surface_id) {
                    return Ok(());
                }
            }
        }
        self.effects.push(HostEffect::HideSurface { surface_id });
        Ok(())
    }

    /// Checked in design.md's order (L): the capability, the id, no live surface with it, the
    /// path, the node count, the live cap for a new id, then the per-call cap.
    pub(crate) fn place_decoration(
        &mut self,
        id: String,
        location: v0_2::DecorationLocation,
        nodes: Vec<v0_2::Node>,
    ) -> Result<(), v0_2::HostError> {
        let placement = self
            .live
            .place_decoration(&id, &location, nodes.len(), &self.capabilities[..])
            .map_err(api_error)?;
        if self.decoration_effects >= MAX_DECORATION_EFFECTS_PER_CALL {
            if placement == Placement::New {
                self.live
                    .remove_decoration(&id, &self.capabilities[..])
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

    /// The per-call cap is checked first here, so the live set is never changed by a removal that
    /// is then refused.
    pub(crate) fn remove_decoration(&mut self, id: String) -> Result<(), v0_2::HostError> {
        if self.decoration_effects >= MAX_DECORATION_EFFECTS_PER_CALL {
            return Err(decoration_effects_exceeded());
        }
        self.live
            .remove_decoration(&id, &self.capabilities[..])
            .map_err(api_error)?;
        self.forget_owner(&id);
        self.decoration_effects += 1;
        self.effects.push(HostEffect::RemoveDecoration { id });
        Ok(())
    }
}

impl ApiState {
    /// `images.add-image` (0.3): declared `ui:images`, at most
    /// [`image::MAX_GENERATED_IMAGE_BYTES`], a PNG header within [`image::MAX_IMAGE_SIDE`] or a
    /// plain SVG, and fewer than [`image::MAX_GENERATED_IMAGES`] live images.
    pub(crate) fn add_image(
        &mut self,
        format: ImageFormat,
        bytes: Vec<u8>,
    ) -> Result<u32, v0_2::HostError> {
        if !self
            .capabilities
            .iter()
            .any(|capability| capability == image::CAPABILITY_IMAGES)
        {
            return Err(host_error(
                v0_2::HostErrorCode::NotPermitted,
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
                v0_2::HostErrorCode::LimitExceeded,
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

    /// The `image` and `canvas` nodes among `props` (0.3, design.md §12, §13): canvases get
    /// [`ui_ext::canvas::check_canvas_props`] and panels only. Images are checked when an update or a reply
    /// for `surface_id` is applied: the props rules of [`image::check_image_props`], panels only
    /// ([`image::check_image_placement`]), and a `generated` handle that `add-image` returned
    /// earlier. A handle removed since is fine: the renderer draws the alt text for it.
    pub(crate) fn check_images<'a>(
        &self,
        surface_id: &str,
        props: impl IntoIterator<Item = &'a v0_2::Props>,
    ) -> Result<(), v0_2::HostError> {
        for props in props {
            if let v0_2::Props::Canvas(canvas) = props {
                ui_ext::canvas::check_canvas_props(canvas)
                    .and_then(|()| {
                        ui_ext::canvas::check_canvas_placement(self.live.surface_kind(surface_id))
                    })
                    .map_err(|error| host_error(error.code(), error.to_string()))?;
                continue;
            }
            let v0_2::Props::Image(props) = props else {
                continue;
            };
            image::check_image_props(props)
                .and_then(|()| image::check_image_placement(self.live.surface_kind(surface_id)))
                .map_err(|error| host_error(error.code(), error.to_string()))?;
            if let image::ImageSource::Generated(handle) = props.source
                && (handle == 0 || handle >= self.next_image)
            {
                return Err(host_error(
                    v0_2::HostErrorCode::InvalidArgument,
                    format!("image handle {handle} was not returned by add-image"),
                ));
            }
        }
        Ok(())
    }

    /// `images.remove-image`: unknown handles are ignored.
    pub(crate) fn remove_image(&mut self, handle: u32) {
        if self.images.remove(&handle) {
            self.effects.push(HostEffect::RemoveImage { handle });
        }
    }
}

impl ApiState {
    fn require_notifications(&self) -> Result<(), v0_2::HostError> {
        if self
            .capabilities
            .iter()
            .any(|capability| capability == notify::CAPABILITY_NOTIFICATIONS)
        {
            Ok(())
        } else {
            Err(host_error(
                v0_2::HostErrorCode::NotPermitted,
                format!(
                    "notifications and progress need {:?} in [fyx_ui].capabilities",
                    notify::CAPABILITY_NOTIFICATIONS
                ),
            ))
        }
    }

    /// A live surface or decoration of the extension.
    fn require_owner(&self, owner: &str) -> Result<(), v0_2::HostError> {
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

    /// `notifications.notify` (design.md M.1), checked in this order: the capability, the
    /// owner, the text rules, the per-call cap, the open cap, the rate cap. Past a cap the error
    /// is `limit-exceeded`, and the caller writes the title to the log instead.
    pub(crate) fn notify(&mut self, notification: Notification) -> Result<u64, v0_2::HostError> {
        self.require_notifications()?;
        self.require_owner(&notification.owner)?;
        notification
            .validate()
            .map_err(|error| host_error(v0_2::HostErrorCode::InvalidArgument, error.0))?;
        let limit = |message: String| host_error(v0_2::HostErrorCode::LimitExceeded, message);
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

    /// `progress-start` (design.md M.2): the capability, the owner (a panel for `panel`), the
    /// title, the running cap, the rate cap.
    pub(crate) fn progress_start(
        &mut self,
        options: ProgressOptions,
    ) -> Result<u64, v0_2::HostError> {
        self.require_notifications()?;
        self.require_owner(&options.owner)?;
        if options.location == notify::ProgressLocation::Panel
            && !matches!(
                self.live.surface_kind(&options.owner),
                Some(v0_2::SurfaceKind::Panel(_))
            )
        {
            return Err(host_error(
                v0_2::HostErrorCode::InvalidArgument,
                format!(
                    "a `panel` progress needs a panel as its owner, and {:?} is not one",
                    options.owner
                ),
            ));
        }
        options
            .validate()
            .map_err(|error| host_error(v0_2::HostErrorCode::InvalidArgument, error.0))?;
        if self.running_progresses.len() >= notify::MAX_RUNNING_PROGRESSES {
            return Err(host_error(
                v0_2::HostErrorCode::LimitExceeded,
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
                v0_2::HostErrorCode::LimitExceeded,
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
    /// [`notify::MAX_PROGRESS_UPDATES_PER_CALL`] per call.
    pub(crate) fn progress_update(
        &mut self,
        id: u64,
        value: ProgressValue,
        message: Option<String>,
    ) -> Result<(), v0_2::HostError> {
        self.require_notifications()?;
        if !self.running_progresses.contains_key(&id) {
            return Err(host_error(
                v0_2::HostErrorCode::InvalidArgument,
                format!("progress {id} is not running"),
            ));
        }
        notify::check_progress_update(value, message.as_deref())
            .map_err(|error| host_error(v0_2::HostErrorCode::InvalidArgument, error.0))?;
        if self.progress_updates >= notify::MAX_PROGRESS_UPDATES_PER_CALL {
            return Err(host_error(
                v0_2::HostErrorCode::LimitExceeded,
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

fn decoration_effects_exceeded() -> v0_2::HostError {
    host_error(
        v0_2::HostErrorCode::LimitExceeded,
        format!("at most {MAX_DECORATION_EFFECTS_PER_CALL} decoration changes per call"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    fn state(capabilities: &[&str]) -> ApiState {
        ApiState::new(
            ApiVersion::V0_2_0,
            capabilities
                .iter()
                .map(|&capability| capability.into())
                .collect(),
        )
    }

    #[test]
    fn test_add_image_checks_and_handles() {
        let mut denied = state(&["ui:panel"]);
        let error = denied.add_image(ImageFormat::Png, png(4, 4)).unwrap_err();
        assert_eq!(error.code, v0_2::HostErrorCode::NotPermitted);

        let mut api = state(&["ui:panel", image::CAPABILITY_IMAGES]);
        api.begin_call();
        let first = api.add_image(ImageFormat::Png, png(4, 4)).expect("added");
        // The bomb: a tiny file whose header claims 65535 × 65535.
        let error = api
            .add_image(ImageFormat::Png, png(65535, 65535))
            .unwrap_err();
        assert_eq!(error.code, v0_2::HostErrorCode::LimitExceeded);
        let mut big = png(4, 4);
        big.resize(image::MAX_GENERATED_IMAGE_BYTES + 1, 0);
        let error = api.add_image(ImageFormat::Png, big).unwrap_err();
        assert_eq!(error.code, v0_2::HostErrorCode::LimitExceeded);
        let error = api
            .add_image(ImageFormat::Svg, b"<!ENTITY x \"y\"><svg/>".to_vec())
            .unwrap_err();
        assert_eq!(error.code, v0_2::HostErrorCode::InvalidArgument);

        // 32 live images; a removed handle frees a slot but is never reused.
        for _ in 1..image::MAX_GENERATED_IMAGES {
            api.add_image(ImageFormat::Svg, b"<svg/>".to_vec())
                .expect("added");
        }
        let error = api
            .add_image(ImageFormat::Svg, b"<svg/>".to_vec())
            .unwrap_err();
        assert_eq!(error.code, v0_2::HostErrorCode::LimitExceeded);
        api.remove_image(first);
        api.remove_image(first);
        api.remove_image(9999);
        let next = api
            .add_image(ImageFormat::Svg, b"<svg/>".to_vec())
            .expect("a slot is free");
        assert_eq!(next, image::MAX_GENERATED_IMAGES as u32 + 1);

        let effects = api.finish_call();
        assert_eq!(effects.len(), image::MAX_GENERATED_IMAGES + 2);
        assert!(matches!(
            effects[0],
            HostEffect::AddImage {
                handle: 1,
                format: ImageFormat::Png,
                ..
            }
        ));
        assert_eq!(
            effects[image::MAX_GENERATED_IMAGES],
            HostEffect::RemoveImage { handle: first }
        );
    }
}

/// The props an update sets: its nodes, or those of its `set-props` and `insert` patches.
pub(crate) fn update_props(update: &v0_2::Update) -> Vec<&v0_2::Props> {
    match update {
        v0_2::Update::Unchanged => Vec::new(),
        v0_2::Update::Replace(nodes) => nodes.iter().map(|node| &node.props).collect(),
        v0_2::Update::Patches(patches) => patches
            .iter()
            .filter_map(|patch| match patch {
                v0_2::Patch::SetProps(_, props) => Some(props),
                v0_2::Patch::Insert(insert) => Some(&insert.node.props),
                v0_2::Patch::Remove(_) | v0_2::Patch::InvalidateRows(_) => None,
            })
            .collect(),
    }
}
