//! The 0.2 host imports' checks and effect list (`docs/fyx/wit-0.2/design.md`, A): every effectful
//! import is checked when it is called, against the live ids as of that point in the call, and
//! its effect is recorded to be applied after the call returns, in call order, before the call's
//! own value. It mirrors the host runtime (`ui_ext_wasm`); the id rules, caps and capability
//! checks are `ui_ext::LiveIds`, which both use.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

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
        }
    }

    /// The static surfaces from `surfaces()`, checked as their version checks them. In 0.2 they
    /// join the surfaces `add-surface` registered during `surfaces()`.
    pub(crate) fn register_static(&mut self, surfaces: &[model::Surface]) -> Result<(), ApiError> {
        match self.version {
            ApiVersion::V0_1_0 => {
                self.live = LiveIds::new(self.version, surfaces, &self.capabilities)?;
            }
            ApiVersion::V0_2_0 | ApiVersion::V0_3_0 => {
                for surface in surfaces {
                    self.live.add_surface(surface, &self.capabilities)?;
                }
            }
            // W0 integration stopgap until platform plan 4.12 (W4): 0.3 is refused at load.
            ApiVersion::V0_3_0 => unreachable!("the harness refuses fyx:ui 0.3 components at load"),
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
            model::UiEvent::InputChanged(..) | model::UiEvent::RowSelected(..) => {}
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

fn decoration_effects_exceeded() -> model::HostError {
    host_error(
        model::HostErrorCode::LimitExceeded,
        format!("at most {MAX_DECORATION_EFFECTS_PER_CALL} decoration changes per call"),
    )
}
