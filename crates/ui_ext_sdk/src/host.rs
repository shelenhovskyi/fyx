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

use crate::{
    DecorationLocation, HostError, LogLevel, Node, Notification, ProgressOptions, ProgressValue,
    Surface, Update,
};
#[cfg(feature = "fyx-ui-0-3")]
use crate::{HttpRequest, ImageFormat, SpawnRequest, WorkspaceEdit};

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
    safe fn __fyx_ui_notify(notification: &Notification) -> Result<u64, HostError>;
    safe fn __fyx_ui_dismiss_notification(id: u64);
    safe fn __fyx_ui_progress_start(options: &ProgressOptions) -> Result<u64, HostError>;
    safe fn __fyx_ui_progress_update(
        id: u64,
        value: ProgressValue,
        message: Option<&str>,
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_progress_end(id: u64);
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_add_image(format: ImageFormat, bytes: &[u8]) -> Result<u32, HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_remove_image(image: u32);
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_fetch(request: &HttpRequest) -> Result<u64, HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_cancel_fetch(request: u64);
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_spawn(request: &SpawnRequest) -> Result<u64, HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_write_stdin(process: u64, data: &[u8]) -> Result<(), HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_close_stdin(process: u64) -> Result<(), HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_kill(process: u64) -> Result<(), HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_apply_edit(edit: &WorkspaceEdit) -> Result<u64, HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_observe_start() -> Result<(), HostError>;
    #[cfg(feature = "fyx-ui-0-3")]
    safe fn __fyx_ui_observe_stop();
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
    /// [`notify`]; off WASM the ids are 1, 2, … shared with [`progress_start`].
    Notify {
        id: u64,
        notification: Notification,
    },
    DismissNotification(u64),
    ProgressStart {
        id: u64,
        options: ProgressOptions,
    },
    ProgressUpdate {
        id: u64,
        value: ProgressValue,
        message: Option<String>,
    },
    ProgressEnd(u64),
    /// `add_image`; the bytes are left out of the record, only their length is kept.
    #[cfg(feature = "fyx-ui-0-3")]
    AddImage {
        image: GeneratedImage,
        format: ImageFormat,
        len: usize,
    },
    #[cfg(feature = "fyx-ui-0-3")]
    RemoveImage(GeneratedImage),
    /// [`fetch`]; off WASM the ids are the shared 1, 2, … of [`notify`].
    #[cfg(feature = "fyx-ui-0-3")]
    Fetch {
        id: u64,
        request: HttpRequest,
    },
    #[cfg(feature = "fyx-ui-0-3")]
    CancelFetch(u64),
    /// [`spawn`]; ids are the shared 1, 2, ….
    #[cfg(feature = "fyx-ui-0-3")]
    Spawn {
        id: u64,
        request: SpawnRequest,
    },
    /// [`write_stdin`]; the bytes are kept.
    #[cfg(feature = "fyx-ui-0-3")]
    WriteStdin {
        process: u64,
        data: Vec<u8>,
    },
    #[cfg(feature = "fyx-ui-0-3")]
    CloseStdin(u64),
    #[cfg(feature = "fyx-ui-0-3")]
    Kill(u64),
    /// [`apply_edit`]; ids are the shared 1, 2, ….
    #[cfg(feature = "fyx-ui-0-3")]
    ApplyEdit {
        id: u64,
        edit: WorkspaceEdit,
    },
    #[cfg(feature = "fyx-ui-0-3")]
    StartObserving,
    #[cfg(feature = "fyx-ui-0-3")]
    StopObserving,
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
thread_local! {
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

#[cfg(not(target_arch = "wasm32"))]
fn next_id() -> u64 {
    NEXT_ID.replace(NEXT_ID.get() + 1)
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

/// Shows a notification in the active workspace window without moving focus, and returns its
/// id (design.md, M.1). Needs `ui:notifications`. Its button clicks and its closing come back to
/// `notification.owner` as [`UiEvent::NotificationAction`](crate::UiEvent) and
/// `NotificationClosed`. Caps: 5 accepted calls per rolling 60 s and 3 open notifications per
/// extension, past which it returns `limit-exceeded`. `fyx:ui` 0.2; `unavailable` for 0.1.
pub fn notify(notification: &Notification) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_notify(notification);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = next_id();
        record(HostEffect::Notify {
            id,
            notification: notification.clone(),
        })?;
        Ok(id)
    }
}

/// Closes a notification of this extension without an event. Unknown or closed ids are ignored.
pub fn dismiss_notification(id: u64) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_dismiss_notification(id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::DismissNotification(id)).ok();
}

/// Starts a progress, indeterminate until the first [`progress_update`], and returns its id
/// (design.md, M.2). Needs `ui:notifications`. At most 4 running per extension and 30 starts per
/// rolling 60 s. A cancellable one sends `UiEvent::ProgressCancelRequested` to `options.owner`.
/// `fyx:ui` 0.2; `unavailable` for 0.1.
pub fn progress_start(options: &ProgressOptions) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_progress_start(options);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = next_id();
        record(HostEffect::ProgressStart {
            id,
            options: options.clone(),
        })?;
        Ok(id)
    }
}

/// Sets a running progress's value and, when given, a one-line message of at most 120 bytes. At
/// most 64 updates per export call; the host redraws at most 10 times a second.
pub fn progress_update(
    id: u64,
    value: ProgressValue,
    message: Option<&str>,
) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_progress_update(id, value, message);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::ProgressUpdate {
        id,
        value,
        message: message.map(str::to_string),
    })
}

/// Ends a progress. Unknown or ended ids are ignored; removing its owner ends it too.
pub fn progress_end(id: u64) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_progress_end(id);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::ProgressEnd(id)).ok();
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

/// `fyx:ui` 0.3, `network:fetch` (design.md §8): starts `request` after the current call returns
/// and returns its id. The outcome arrives in [`Extension::handle_io`](crate::Extension) as
/// `IoEvent::FetchDone((id, result))`. The URL must be `https://` and a host of the manifest's
/// `[fyx_ui.network] domains` (`http://` only for a declared `localhost` with a port). The first
/// fetch asks the user; until they allow it, and if they refuse, the result is
/// `FetchError::Denied`. At most 8 requests in flight and 120 started per minute. Off WASM it
/// records [`HostEffect::Fetch`] and hands out ids.
#[cfg(feature = "fyx-ui-0-3")]
pub fn fetch(request: &HttpRequest) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_fetch(request);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = next_id();
        record(HostEffect::Fetch {
            id,
            request: request.clone(),
        })?;
        Ok(id)
    }
}

/// `fyx:ui` 0.3: cancels a request; `FetchDone` follows with `FetchError::Cancelled` unless it
/// already completed. Unknown ids are ignored.
#[cfg(feature = "fyx-ui-0-3")]
pub fn cancel_fetch(request: u64) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_cancel_fetch(request);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::CancelFetch(request)).ok();
}

/// `fyx:ui` 0.3, `process:spawn` (design.md §9): starts a program the manifest's
/// `[[fyx_ui.processes]]` names, after the current call returns, and returns its id. Output
/// arrives in [`Extension::handle_io`](crate::Extension) as `IoEvent::ProcessOutput`, then one
/// `IoEvent::ProcessExited`, or one `IoEvent::SpawnFailed` instead. The first spawn asks the
/// user; if they refuse, it fails with `SpawnError::Denied`. At most 4 processes per extension.
/// A program the manifest does not name gets an id and then `SpawnError::NotFound`. Off WASM it
/// records [`HostEffect::Spawn`] and hands out ids.
#[cfg(feature = "fyx-ui-0-3")]
pub fn spawn(request: &SpawnRequest) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_spawn(request);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = next_id();
        record(HostEffect::Spawn {
            id,
            request: request.clone(),
        })?;
        Ok(id)
    }
}

/// `fyx:ui` 0.3: writes to a process spawned with `stdin`. At most 1 MiB per call and 4 MiB
/// queued per process.
#[cfg(feature = "fyx-ui-0-3")]
pub fn write_stdin(process: u64, data: &[u8]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_write_stdin(process, data);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::WriteStdin {
        process,
        data: data.to_vec(),
    })
}

/// `fyx:ui` 0.3: closes a process's stdin, which many programs wait for before they finish.
#[cfg(feature = "fyx-ui-0-3")]
pub fn close_stdin(process: u64) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_close_stdin(process);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::CloseStdin(process))
}

/// `fyx:ui` 0.3: kills a process and everything it started; `ProcessExited` follows with
/// `killed` set. A process that already exited is `invalid-argument`.
#[cfg(feature = "fyx-ui-0-3")]
pub fn kill(process: u64) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_kill(process);
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::Kill(process))
}

/// `fyx:ui` 0.3, `workspace:edit` (design.md §10): applies `edit` after the current call returns,
/// all files or none, as one undo step per buffer, and returns its id. The outcome arrives in
/// [`Extension::handle_io`](crate::Extension) as `IoEvent::EditDone((id, result))`. Nothing is
/// saved: edited buffers stay dirty. The first edit asks the user. At most 4 edits in flight,
/// 64 files and 4 MiB of new text each; protected paths (`.git/`, `.fyx/`, `.zed/`, …) are
/// refused. Off WASM it records [`HostEffect::ApplyEdit`] and hands out ids.
#[cfg(feature = "fyx-ui-0-3")]
pub fn apply_edit(edit: &WorkspaceEdit) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_apply_edit(edit);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = next_id();
        record(HostEffect::ApplyEdit {
            id,
            edit: edit.clone(),
        })?;
        Ok(id)
    }
}

/// `fyx:ui` 0.3, `agent:observe` (design.md §11): starts the read-only feed of the host's
/// review of agent edits. A `Snapshot` of the open reviews comes first, then `Opened`, `Updated`,
/// `Resolved`, `Closed`, `TurnEnded`, `UnreviewedChanges` and `ReviewMode` as `IoEvent::Agent`.
/// The first start asks the user. Nothing in the API can accept, reject or comment.
#[cfg(feature = "fyx-ui-0-3")]
pub fn start_observing() -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_observe_start();
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::StartObserving)
}

/// `fyx:ui` 0.3: stops the agent feed.
#[cfg(feature = "fyx-ui-0-3")]
pub fn stop_observing() {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_observe_stop();
    #[cfg(not(target_arch = "wasm32"))]
    record(HostEffect::StopObserving).ok();
}

/// The implementations behind the shims of `register_extension!(T)`: the SDK's own world, 0.2
/// or (with `fyx-ui-0-3`) 0.3.
#[doc(hidden)]
pub mod newest {
    #[cfg(feature = "fyx-ui-0-3")]
    use crate::ImageFormat;
    #[cfg(feature = "fyx-ui-0-3")]
    use crate::bindings::fyx::ui::{agent_observe, images, network, process, workspace_edit};
    use crate::bindings::fyx::ui::{decorations, host, notifications};
    use crate::{
        DecorationLocation, HostError, LogLevel, Node, Notification, ProgressOptions,
        ProgressValue, Surface, Update,
    };
    #[cfg(feature = "fyx-ui-0-3")]
    use crate::{HttpRequest, SpawnRequest, WorkspaceEdit};

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

    pub fn notify(notification: &Notification) -> Result<u64, HostError> {
        notifications::notify(notification)
    }

    pub fn dismiss_notification(id: u64) {
        notifications::dismiss_notification(id);
    }

    pub fn progress_start(options: &ProgressOptions) -> Result<u64, HostError> {
        notifications::progress_start(options)
    }

    pub fn progress_update(
        id: u64,
        value: ProgressValue,
        message: Option<&str>,
    ) -> Result<(), HostError> {
        notifications::progress_update(id, value, message)
    }

    pub fn progress_end(id: u64) {
        notifications::progress_end(id);
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn add_image(format: ImageFormat, bytes: &[u8]) -> Result<u32, HostError> {
        images::add_image(format, bytes)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn remove_image(image: u32) {
        images::remove_image(image);
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn fetch(request: &HttpRequest) -> Result<u64, HostError> {
        network::fetch(request)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn cancel_fetch(request: u64) {
        network::cancel_fetch(request);
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn spawn(request: &SpawnRequest) -> Result<u64, HostError> {
        process::spawn(request)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn write_stdin(id: u64, data: &[u8]) -> Result<(), HostError> {
        process::write_stdin(id, data)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn close_stdin(id: u64) -> Result<(), HostError> {
        process::close_stdin(id)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn kill(id: u64) -> Result<(), HostError> {
        process::kill(id)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn apply_edit(edit: &WorkspaceEdit) -> Result<u64, HostError> {
        workspace_edit::apply_edit(edit)
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn observe_start() -> Result<(), HostError> {
        agent_observe::start()
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn observe_stop() {
        agent_observe::stop();
    }
}
