//! Functions the extension calls on the host.
//!
//! On WASM each one goes through an `extern "Rust"` shim that `register_extension!` defines for
//! the world it exports, so a component imports only that world's host functions, and only the
//! ones it calls. Off WASM (unit tests of an extension) they record a [`HostEffect`] instead and
//! succeed; [`take_host_effects`] and [`take_pushed_updates`] read the record back.
//!
//! Every function with an effect is checked when it is called and returns the result at once;
//! the effect itself happens after the current export call returns, in call order, before that
//! call's own reply is applied (design.md, section A).

#[cfg(not(target_arch = "wasm32"))]
use std::cell::RefCell;

#[cfg(feature = "fyx-ui-0-3")]
use crate::ImageFormat;
use crate::{DecorationLocation, HostError, LogLevel, Node, Surface, Update};

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_exported_api_version() -> (u16, u16, u16);
    safe fn __fyx_ui_log(level: LogLevel, message: &str);
    safe fn __fyx_ui_push_update(surface_id: &str, update: &Update) -> Result<(), HostError>;
    safe fn __fyx_ui_request_tick(surface_id: &str, delay_ms: u32) -> Result<(), HostError>;
    safe fn __fyx_ui_cancel_tick(surface_id: &str);
    safe fn __fyx_ui_add_surface(surface: &Surface) -> Result<(), HostError>;
    safe fn __fyx_ui_remove_surface(surface_id: &str) -> Result<(), HostError>;
    safe fn __fyx_ui_show_surface(surface_id: &str) -> Result<(), HostError>;
    safe fn __fyx_ui_hide_surface(surface_id: &str) -> Result<(), HostError>;
    safe fn __fyx_ui_place_decoration(
        id: &str,
        location: &DecorationLocation,
        nodes: &[Node],
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_remove_decoration(id: &str) -> Result<(), HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_add_image(format: ImageFormat, bytes: &[u8]) -> Result<u32, HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_remove_image(image: u32);
}

/// What a host function did, recorded off WASM for unit tests.
#[derive(Debug, Clone, PartialEq)]
pub enum HostEffect {
    Push {
        surface_id: String,
        update: Update,
    },
    RequestTick {
        surface_id: String,
        delay_ms: u32,
    },
    CancelTick {
        surface_id: String,
    },
    AddSurface(Surface),
    RemoveSurface(String),
    ShowSurface(String),
    HideSurface(String),
    PlaceDecoration {
        id: String,
        location: DecorationLocation,
        nodes: Vec<Node>,
    },
    RemoveDecoration(String),
    /// `add_image`; the bytes are left out of the record, only their length is kept.
    #[cfg(feature = "fyx-ui-0-3")]
    AddImage {
        image: GeneratedImage,
        format: ImageFormat,
        len: usize,
    },
    #[cfg(feature = "fyx-ui-0-3")]
    RemoveImage(GeneratedImage),
}

/// A generated image from [`add_image`], for `ImageProps::generated`. `Copy` and without `Drop`:
/// a handle outlives remounts, and freeing it with [`remove_image`] is the extension's decision,
/// as removing a decoration is.
#[cfg(feature = "fyx-ui-0-3")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GeneratedImage(u32);

#[cfg(feature = "fyx-ui-0-3")]
impl GeneratedImage {
    /// The handle as the WIT carries it, in `image-source::generated`.
    pub fn handle(self) -> u32 {
        self.0
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static EFFECTS: RefCell<Vec<HostEffect>> = const { RefCell::new(Vec::new()) };
    #[cfg(feature = "fyx-ui-0-3")]
    static NEXT_IMAGE: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
}

#[cfg(not(target_arch = "wasm32"))]
fn record(effect: HostEffect) -> Result<(), HostError> {
    EFFECTS.with_borrow_mut(|effects| effects.push(effect));
    Ok(())
}

/// Everything the host functions did on this thread since the last call, in order. Off WASM
/// only, for unit tests of an extension.
#[cfg(not(target_arch = "wasm32"))]
pub fn take_host_effects() -> Vec<HostEffect> {
    EFFECTS.with_borrow_mut(std::mem::take)
}

/// The updates [`push_update`] sent on this thread since the last call; other effects stay
/// recorded for [`take_host_effects`]. Off WASM only.
#[cfg(not(target_arch = "wasm32"))]
pub fn take_pushed_updates() -> Vec<(String, Update)> {
    EFFECTS.with_borrow_mut(|effects| {
        let mut pushed = Vec::new();
        effects.retain(|effect| match effect {
            HostEffect::Push { surface_id, update } => {
                pushed.push((surface_id.clone(), update.clone()));
                false
            }
            _ => true,
        });
        pushed
    })
}

/// The `fyx:ui` version the component exports: [`crate::API_VERSION`] for
/// `register_extension!(T)`, [`crate::API_VERSION_V0_2`] for `register_extension!(T, v0_2)` and
/// [`crate::API_VERSION_V0_1`] for `register_extension!(T, v0_1)`.
/// Off WASM it is [`crate::API_VERSION`].
pub fn exported_api_version() -> (u16, u16, u16) {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_exported_api_version();
    #[cfg(not(target_arch = "wasm32"))]
    crate::API_VERSION
}

/// Writes a message to the host's log. Off WASM it goes to stderr.
pub fn log(level: LogLevel, message: &str) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_log(level, message);
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[{level:?}] {message}");
}

/// Sends an update for `surface_id` during a call: from `mount`, `handle-event`, `list-rows` or
/// `tick`, typically for a surface other than the one the call is about.
///
/// `Ok` means the update passed the checks the host can make at once (the surface is live, the
/// caps, at most 64 pushes per call); it can still fail to fit the tree when it is applied, which
/// remounts the surface. On `Err` nothing was queued and your mirror of that surface is ahead of
/// the host: [`SurfaceTree::push`](crate::SurfaceTree::push) handles that by resending it whole.
pub fn push_update(surface_id: &str, update: &Update) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_push_update(surface_id, update);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::Push {
        surface_id: surface_id.to_string(),
        update: update.clone(),
    })
}

/// Asks for one `tick(surface_id)` call at least `delay_ms` after the current call returns. One
/// pending tick per surface: a new request replaces the earlier one. The host clamps the delay
/// up to 100 ms, and further while Fyx is in the background; a tick for a hidden panel waits
/// until it is shown. Re-request it inside [`Extension::tick`](crate::Extension::tick) for a
/// periodic timer.
pub fn request_tick(surface_id: &str, delay_ms: u32) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_request_tick(surface_id, delay_ms);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::RequestTick {
        surface_id: surface_id.to_string(),
        delay_ms,
    })
}

/// Drops the pending tick of a surface, if any.
pub fn cancel_tick(surface_id: &str) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_cancel_tick(surface_id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::CancelTick {
        surface_id: surface_id.to_string(),
    })
    .ok();
}

/// Adds a surface at runtime. The manifest must declare the capability of its kind; at most 32
/// live surfaces per extension. Panels and status items are mounted at once; a modal waits for
/// [`show_surface`].
pub fn add_surface(surface: &Surface) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_add_surface(surface);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::AddSurface(surface.clone()))
}

/// Removes a live surface, static or runtime. No event is sent; the id may be added again.
pub fn remove_surface(surface_id: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_remove_surface(surface_id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::RemoveSurface(surface_id.to_string()))
}

/// Opens a modal and focuses it, or reveals a panel. A modal needs a click or Enter: it is
/// allowed during `handle-event` for `click`, `row-click` or `input-submitted`, or within 5 s
/// after one, once.
pub fn show_surface(surface_id: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_show_surface(surface_id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::ShowSurface(surface_id.to_string()))
}

/// Closes a modal (no `dismissed` event) or hides a panel's dock if that panel is active.
pub fn hide_surface(surface_id: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_hide_surface(surface_id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::HideSurface(surface_id.to_string()))
}

/// Places a decoration: a tree drawn as a block above or below a buffer line, anchored by the
/// host. Needs `ui:editor-decorations`; at most 512 per extension. Placing a live id again moves
/// it and replaces its tree. The id then works like a surface id for [`push_update`],
/// `handle-event`, `list-rows` and `mount`.
pub fn place_decoration(
    id: &str,
    location: &DecorationLocation,
    nodes: &[Node],
) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_place_decoration(id, location, nodes);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::PlaceDecoration {
        id: id.to_string(),
        location: location.clone(),
        nodes: nodes.to_vec(),
    })
}

/// Removes a live decoration in every editor; no event is sent.
pub fn remove_decoration(id: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_remove_decoration(id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::RemoveDecoration(id.to_string()))
}

/// `fyx:ui` 0.3: registers a PNG or SVG made at run time, for `ImageProps::generated`. Needs
/// `ui:images`; at most 1 MiB, at most 32 live images per extension. The host checks the bytes
/// at once (the PNG header, or that an SVG is plain text) and decodes them after the call; until
/// then, and if decoding fails, image nodes show their alt text. Off WASM it hands out handles
/// 1, 2, … and records [`HostEffect::AddImage`].
#[cfg(feature = "fyx-ui-0-3")]
pub fn add_image(format: ImageFormat, bytes: &[u8]) -> Result<GeneratedImage, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_add_image(format, bytes).map(GeneratedImage);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let image = GeneratedImage(NEXT_IMAGE.replace(NEXT_IMAGE.get() + 1));
        record(HostEffect::AddImage {
            image,
            format,
            len: bytes.len(),
        })?;
        Ok(image)
    }
}

/// `fyx:ui` 0.3: frees a generated image; nodes that still show it show their alt text. An
/// unknown handle is ignored.
#[cfg(feature = "fyx-ui-0-3")]
pub fn remove_image(image: GeneratedImage) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_remove_image(image.0);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::RemoveImage(image)).ok();
}

/// The implementations behind the shims of `register_extension!(T)`: the SDK's own world, 0.2
/// or (with `fyx-ui-0-3`) 0.3.
#[doc(hidden)]
pub mod newest {
    #[cfg(feature = "fyx-ui-0-3")]
    use crate::ImageFormat;
    #[cfg(feature = "fyx-ui-0-3")]
    use crate::bindings::fyx::ui::images;
    use crate::bindings::fyx::ui::{decorations, host};
    use crate::{DecorationLocation, HostError, LogLevel, Node, Surface, Update};

    pub const API_VERSION: (u16, u16, u16) = crate::API_VERSION;

    pub fn log(level: LogLevel, message: &str) {
        host::log(level, message);
    }

    pub fn push_update(surface_id: &str, update: &Update) -> Result<(), HostError> {
        host::push_update(surface_id, update)
    }

    pub fn request_tick(surface_id: &str, delay_ms: u32) -> Result<(), HostError> {
        host::request_tick(surface_id, delay_ms)
    }

    pub fn cancel_tick(surface_id: &str) {
        host::cancel_tick(surface_id);
    }

    pub fn add_surface(surface: &Surface) -> Result<(), HostError> {
        host::add_surface(surface)
    }

    pub fn remove_surface(surface_id: &str) -> Result<(), HostError> {
        host::remove_surface(surface_id)
    }

    pub fn show_surface(surface_id: &str) -> Result<(), HostError> {
        host::show_surface(surface_id)
    }

    pub fn hide_surface(surface_id: &str) -> Result<(), HostError> {
        host::hide_surface(surface_id)
    }

    pub fn place_decoration(
        id: &str,
        location: &DecorationLocation,
        nodes: &[Node],
    ) -> Result<(), HostError> {
        decorations::place_decoration(id, location, nodes)
    }

    pub fn remove_decoration(id: &str) -> Result<(), HostError> {
        decorations::remove_decoration(id)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn add_image(format: ImageFormat, bytes: &[u8]) -> Result<u32, HostError> {
        images::add_image(format, bytes)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn remove_image(image: u32) {
        images::remove_image(image);
    }
}
