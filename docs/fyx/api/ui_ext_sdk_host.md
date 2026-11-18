# `ui_ext_sdk_host` public items

Generated from `crates/ui_ext_sdk/src/host.rs` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

Every public item, in path order. For signatures, run `script/fyx-api-docs` and open the rustdoc it builds.

| Item | Kind | Summary |
|---|---|---|
| `GeneratedImage` | struct | A generated image from `add_image`, for `ImageProps::generated`. `Copy` and without `Drop`: a handle outlives remounts, and freeing it with `remove_image` is the extension's decision, as removing a decoration is. |
| `GeneratedImage::handle` | method | The handle as the WIT carries it, in `image-source::generated`. |
| `HostEffect` | enum | What a host function did, recorded off WASM for unit tests. |
| `HostEffect::AddImage` | variant | `add_image`; the bytes are left out of the record, only their length is kept. |
| `HostEffect::AddSurface` | variant |  |
| `HostEffect::CancelTick` | variant |  |
| `HostEffect::DismissNotification` | variant |  |
| `HostEffect::HideSurface` | variant |  |
| `HostEffect::Notify` | variant | `notify`; off WASM the ids are 1, 2, … shared with `progress_start`. |
| `HostEffect::PlaceDecoration` | variant |  |
| `HostEffect::ProgressEnd` | variant |  |
| `HostEffect::ProgressStart` | variant |  |
| `HostEffect::ProgressUpdate` | variant |  |
| `HostEffect::Push` | variant |  |
| `HostEffect::RemoveDecoration` | variant |  |
| `HostEffect::RemoveImage` | variant |  |
| `HostEffect::RemoveSurface` | variant |  |
| `HostEffect::RequestTick` | variant |  |
| `HostEffect::ShowSurface` | variant |  |
| `add_image` | fn | `fyx:ui` 0.3: registers a PNG or SVG made at run time, for `ImageProps::generated`. Needs `ui:images`; at most 1 MiB, at most 32 live images per extension. The host checks the bytes at once (the PNG header, or that an SVG is plain text) and decodes them after the call; until then, and if decoding fails, image nodes show their alt text. Off WASM it hands out handles 1, 2, … and records `HostEffect::AddImage`. |
| `add_surface` | fn | Adds a surface at runtime. The manifest must declare the capability of its kind; at most 32 live surfaces per extension. Panels and status items are mounted at once; a modal waits for `show_surface`. |
| `cancel_tick` | fn | Drops the pending tick of a surface, if any. |
| `dismiss_notification` | fn | Closes a notification of this extension without an event. Unknown or closed ids are ignored. |
| `exported_api_version` | fn | The `fyx:ui` version the component exports: `crate::API_VERSION` for `register_extension!(T)`, `crate::API_VERSION_V0_2` for `register_extension!(T, v0_2)` and `crate::API_VERSION_V0_1` for `register_extension!(T, v0_1)`. Off WASM it is `crate::API_VERSION`. |
| `hide_surface` | fn | Closes a modal (no `dismissed` event) or hides a panel's dock if that panel is active. |
| `log` | fn | Writes a message to the host's log. Off WASM it goes to stderr. |
| `notify` | fn | Shows a notification in the active workspace window without moving focus, and returns its id (design.md, M.1). Needs `ui:notifications`. Its button clicks and its closing come back to `notification.owner` as `UiEvent::NotificationAction`(crate::UiEvent) and `NotificationClosed`. Caps: 5 accepted calls per rolling 60 s and 3 open notifications per extension, past which it returns `limit-exceeded`. `fyx:ui` 0.2; `unavailable` for 0.1. |
| `place_decoration` | fn | Places a decoration: a tree drawn as a block above or below a buffer line, anchored by the host. Needs `ui:editor-decorations`; at most 512 per extension. Placing a live id again moves it and replaces its tree. The id then works like a surface id for `push_update`, `handle-event`, `list-rows` and `mount`. |
| `progress_end` | fn | Ends a progress. Unknown or ended ids are ignored; removing its owner ends it too. |
| `progress_start` | fn | Starts a progress, indeterminate until the first `progress_update`, and returns its id (design.md, M.2). Needs `ui:notifications`. At most 4 running per extension and 30 starts per rolling 60 s. A cancellable one sends `UiEvent::ProgressCancelRequested` to `options.owner`. `fyx:ui` 0.2; `unavailable` for 0.1. |
| `progress_update` | fn | Sets a running progress's value and, when given, a one-line message of at most 120 bytes. At most 64 updates per export call; the host redraws at most 10 times a second. |
| `push_update` | fn | Sends an update for `surface_id` during a call: from `mount`, `handle-event`, `list-rows` or `tick`, typically for a surface other than the one the call is about. |
| `remove_decoration` | fn | Removes a live decoration in every editor; no event is sent. |
| `remove_image` | fn | `fyx:ui` 0.3: frees a generated image; nodes that still show it show their alt text. An unknown handle is ignored. |
| `remove_surface` | fn | Removes a live surface, static or runtime. No event is sent; the id may be added again. |
| `request_tick` | fn | Asks for one `tick(surface_id)` call at least `delay_ms` after the current call returns. One pending tick per surface: a new request replaces the earlier one. The host clamps the delay up to 100 ms, and further while Fyx is in the background; a tick for a hidden panel waits until it is shown. Re-request it inside `Extension::tick`(crate::Extension::tick) for a periodic timer. |
| `show_surface` | fn | Opens a modal and focuses it, or reveals a panel. A modal needs a click or Enter: it is allowed during `handle-event` for `click`, `row-click` or `input-submitted`, or within 5 s after one, once. |
| `take_host_effects` | fn (host only) | Everything the host functions did on this thread since the last call, in order. Off WASM only, for unit tests of an extension. |
| `take_pushed_updates` | fn (host only) | The updates `push_update` sent on this thread since the last call; other effects stay recorded for `take_host_effects`. Off WASM only. |
