//! The 0.2 host imports' checks and effect list (`docs/fyx/wit-0.2/design.md`, A): every effectful
//! import is checked when it is called, against the live ids as of that point in the call, and
//! its effect is recorded to be applied after the call returns, in call order, before the call's
//! own value. It mirrors the host runtime (`ui_ext_wasm`); the id rules, caps and capability
//! checks are `ui_ext::LiveIds`, which both use.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ui_ext::image::{self, ImageFormat};

use ui_ext::v0_2 as model;
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
}

impl ApiState {
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
        }
    }

    /// The static surfaces from `surfaces()`, checked as their version checks them. In 0.2 they
    /// join the surfaces `add-surface` registered during `surfaces()`.
    pub(crate) fn register_static(&mut self, surfaces: &[model::Surface]) -> Result<(), ApiError> {
        match self.version {
            ApiVersion::V0_1_0 => {
                self.live = LiveIds::new(self.version, surfaces, &self.capabilities)?;
            }
            // The harness refuses 0.3 components before this point (plan 4.12 adds them); 0.3
            // registers static surfaces as 0.2 does.
            ApiVersion::V0_2_0 | ApiVersion::V0_3_0 => {
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
            | model::UiEvent::InputSubmitted(..) => self.activation = Activation::DuringCall,
            model::UiEvent::Dismissed(_) => {
                self.shown_modals.remove(surface_id);
            }
            // §M: a clicked notification action is a gesture, as in the host.
            model::UiEvent::NotificationAction(_) => self.activation = Activation::DuringCall,
            model::UiEvent::InputChanged(..)
            | model::UiEvent::RowSelected(..)
            | model::UiEvent::NotificationClosed(..)
            | model::UiEvent::ProgressCancelRequested(_) => {}
        }
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

fn decoration_effects_exceeded() -> model::HostError {
    host_error(
        model::HostErrorCode::LimitExceeded,
        format!("at most {MAX_DECORATION_EFFECTS_PER_CALL} decoration changes per call"),
    )
}
