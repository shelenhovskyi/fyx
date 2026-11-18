//! The effect list of a call and the per-extension state the 0.2 host imports are checked against
//! (design.md, A): every effectful import is checked when it is called, against the live surface
//! and decoration ids as of that point in the call, and its effect is recorded to be applied after
//! the call returns, in call order, before the call's own value.

use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

use ui_ext::{ApiError, ApiVersion, LiveIds, MAX_DECORATION_EFFECTS_PER_CALL, Placement, v0_2};

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
            ApiVersion::V0_2_0 => {
                for surface in surfaces {
                    self.live.add_surface(surface, &self.capabilities[..])?;
                }
            }
        }
        Ok(())
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

    /// Before `handle-event`: a click or Enter grants an activation, and a `dismissed` modal is
    /// no longer shown.
    pub(crate) fn begin_event(&mut self, surface_id: &str, event: &v0_2::UiEvent) {
        match event {
            v0_2::UiEvent::Click(_)
            | v0_2::UiEvent::RowClick(..)
            | v0_2::UiEvent::InputSubmitted(..) => self.activation = Activation::DuringCall,
            v0_2::UiEvent::Dismissed(_) => {
                self.shown_modals.remove(surface_id);
            }
            v0_2::UiEvent::InputChanged(..) | v0_2::UiEvent::RowSelected(..) => {}
        }
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
        self.decoration_effects += 1;
        self.effects.push(HostEffect::RemoveDecoration { id });
        Ok(())
    }
}

fn decoration_effects_exceeded() -> v0_2::HostError {
    host_error(
        v0_2::HostErrorCode::LimitExceeded,
        format!("at most {MAX_DECORATION_EFFECTS_PER_CALL} decoration changes per call"),
    )
}
