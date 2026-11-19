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
| `HostEffect::ApplyEdit` | variant | `apply_edit`; ids are the shared 1, 2, …. |
| `HostEffect::CancelFetch` | variant |  |
| `HostEffect::CancelTick` | variant |  |
| `HostEffect::CloseStdin` | variant |  |
| `HostEffect::Context` | variant | A `crate::context` function (`fyx:ui` 0.4). |
| `HostEffect::DismissNotification` | variant |  |
| `HostEffect::Fetch` | variant | `fetch`; off WASM the ids are the shared 1, 2, … of `notify`. |
| `HostEffect::HideSurface` | variant |  |
| `HostEffect::Kill` | variant |  |
| `HostEffect::Listener` | variant | A `crate::listeners` function (`fyx:ui` 0.4). |
| `HostEffect::Notify` | variant | `notify`; off WASM the ids are 1, 2, … shared with `progress_start`. |
| `HostEffect::PlaceDecoration` | variant |  |
| `HostEffect::ProgressEnd` | variant |  |
| `HostEffect::ProgressStart` | variant |  |
| `HostEffect::ProgressUpdate` | variant |  |
| `HostEffect::Push` | variant |  |
| `HostEffect::RemoveDecoration` | variant |  |
| `HostEffect::RemoveImage` | variant |  |
| `HostEffect::RemoveSurface` | variant |  |
| `HostEffect::RequestPermission` | variant | `request_permission`. |
| `HostEffect::RequestTick` | variant |  |
| `HostEffect::ShowSurface` | variant |  |
| `HostEffect::Spawn` | variant | `spawn`; ids are the shared 1, 2, …. |
| `HostEffect::StartObserving` | variant |  |
| `HostEffect::StopObserving` | variant |  |
| `HostEffect::Terminal` | variant | A `crate::terminals` function (`fyx:ui` 0.4). |
| `HostEffect::WriteStdin` | variant | `write_stdin`; the bytes are kept. |
| `add_image` | fn | `fyx:ui` 0.3: registers a PNG or SVG made at run time, for `ImageProps::generated`. Needs `ui:images`; at most 1 MiB, at most 32 live images per extension. The host checks the bytes at once (the PNG header, or that an SVG is plain text) and decodes them after the call; until then, and if decoding fails, image nodes show their alt text. Off WASM it hands out handles 1, 2, … and records `HostEffect::AddImage`. |
| `add_surface` | fn | Adds a surface at runtime. The manifest must declare the capability of its kind; at most 32 live surfaces per extension. Panels and status items are mounted at once; a modal waits for `show_surface`. |
| `apply_edit` | fn | `fyx:ui` 0.3, `workspace:edit` (design.md §10): applies `edit` after the current call returns, all files or none, as one undo step per buffer, and returns its id. The outcome arrives in `Extension::handle_io`(crate::Extension) as `IoEvent::EditDone((id, result))`. Nothing is saved: edited buffers stay dirty. The first edit asks the user. At most 4 edits in flight, 64 files and 4 MiB of new text each; protected paths (`.git/`, `.fyx/`, `.zed/`, …) are refused. Off WASM it records `HostEffect::ApplyEdit` and hands out ids. |
| `cancel_fetch` | fn | `fyx:ui` 0.3: cancels a request; `FetchDone` follows with `FetchError::Cancelled` unless it already completed. Unknown ids are ignored. |
| `cancel_tick` | fn | Drops the pending tick of a surface, if any. |
| `close_stdin` | fn | `fyx:ui` 0.3: closes a process's stdin, which many programs wait for before they finish. |
| `dismiss_notification` | fn | Closes a notification of this extension without an event. Unknown or closed ids are ignored. |
| `exported_api_version` | fn | The `fyx:ui` version the component exports: `crate::API_VERSION` for `register_extension!(T)`, `crate::API_VERSION_V0_2` for `register_extension!(T, v0_2)` and `crate::API_VERSION_V0_1` for `register_extension!(T, v0_1)`. Off WASM it is `crate::API_VERSION`. |
| `fetch` | fn | `fyx:ui` 0.3, `network:fetch` (design.md §8): starts `request` after the current call returns and returns its id. The outcome arrives in `Extension::handle_io`(crate::Extension) as `IoEvent::FetchDone((id, result))`. The URL must be `https://` and a host of the manifest's `[fyx_ui.network] domains` (`http://` only for a declared `localhost` with a port). The first fetch asks the user; until they allow it, and if they refuse, the result is `FetchError::Denied`. At most 8 requests in flight and 120 started per minute. Off WASM it records `HostEffect::Fetch` and hands out ids. |
| `hide_surface` | fn | Closes a modal (no `dismissed` event) or hides a panel's dock if that panel is active. |
| `kill` | fn | `fyx:ui` 0.3: kills a process and everything it started; `ProcessExited` follows with `killed` set. A process that already exited is `invalid-argument`. |
| `log` | fn | Writes a message to the host's log. Off WASM it goes to stderr. |
| `notify` | fn | Shows a notification in the active workspace window without moving focus, and returns its id (design.md, M.1). Needs `ui:notifications`. Its button clicks and its closing come back to `notification.owner` as `UiEvent::NotificationAction`(crate::UiEvent) and `NotificationClosed`. Caps: 5 accepted calls per rolling 60 s and 3 open notifications per extension, past which it returns `limit-exceeded`. `fyx:ui` 0.2; `unavailable` for 0.1. |
| `place_decoration` | fn | Places a decoration: a tree drawn as a block above or below a buffer line, anchored by the host. Needs `ui:editor-decorations`; at most 512 per extension. Placing a live id again moves it and replaces its tree. The id then works like a surface id for `push_update`, `handle-event`, `list-rows` and `mount`. |
| `progress_end` | fn | Ends a progress. Unknown or ended ids are ignored; removing its owner ends it too. |
| `progress_start` | fn | Starts a progress, indeterminate until the first `progress_update`, and returns its id (design.md, M.2). Needs `ui:notifications`. At most 4 running per extension and 30 starts per rolling 60 s. A cancellable one sends `UiEvent::ProgressCancelRequested` to `options.owner`. `fyx:ui` 0.2; `unavailable` for 0.1. |
| `progress_update` | fn | Sets a running progress's value and, when given, a one-line message of at most 120 bytes. At most 64 updates per export call; the host redraws at most 10 times a second. |
| `push_update` | fn | Sends an update for `surface_id` during a call: from `mount`, `handle-event`, `list-rows` or `tick`, typically for a surface other than the one the call is about. |
| `query_permission` | fn | `fyx:ui` 0.3, `permissions.query` (design.md §7): the state of a T2 capability as of the start of the current call. `Unavailable` when the manifest does not declare it (or, for `process:spawn` and `workspace:edit`, the worktree is not trusted); `Ask` until the user is asked. Off WASM it answers what `set_permission_state` set, `Ask` by default. |
| `remove_decoration` | fn | Removes a live decoration in every editor; no event is sent. |
| `remove_image` | fn | `fyx:ui` 0.3: frees a generated image; nodes that still show it show their alt text. An unknown handle is ignored. |
| `remove_surface` | fn | Removes a live surface, static or runtime. No event is sent; the id may be added again. |
| `request_permission` | fn | `fyx:ui` 0.3, `permissions.request`: shows the consent prompt after the current call, if the state is `Ask`; any other state stays as it is. It needs the click-or-Enter activation (call it from a button click) and uses it up; without one, or for an undeclared capability, it is `not-permitted`. The answer arrives in `handle_io` as `IoEvent::PermissionChanged`. |
| `request_tick` | fn | Asks for one `tick(surface_id)` call at least `delay_ms` after the current call returns. One pending tick per surface: a new request replaces the earlier one. The host clamps the delay up to 100 ms, and further while Fyx is in the background; a tick for a hidden panel waits until it is shown. Re-request it inside `Extension::tick`(crate::Extension::tick) for a periodic timer. |
| `set_permission_state` | fn (host only) | Off WASM, what `query_permission` answers for `capability` on this thread (`Ask` until set), for unit tests of an extension. |
| `show_surface` | fn | Opens a modal and focuses it, or reveals a panel. A modal needs a click or Enter: it is allowed during `handle-event` for `click`, `row-click` or `input-submitted`, or within 5 s after one, once. |
| `spawn` | fn | `fyx:ui` 0.3, `process:spawn` (design.md §9): starts a program the manifest's `[[fyx_ui.processes]]` names, after the current call returns, and returns its id. Output arrives in `Extension::handle_io`(crate::Extension) as `IoEvent::ProcessOutput`, then one `IoEvent::ProcessExited`, or one `IoEvent::SpawnFailed` instead. The first spawn asks the user; if they refuse, it fails with `SpawnError::Denied`. At most 4 processes per extension. A program the manifest does not name gets an id and then `SpawnError::NotFound`. Off WASM it records `HostEffect::Spawn` and hands out ids. |
| `start_observing` | fn | `fyx:ui` 0.3, `agent:observe` (design.md §11): starts the read-only feed of the host's review of agent edits. A `Snapshot` of the open reviews comes first, then `Opened`, `Updated`, `Resolved`, `Closed`, `TurnEnded`, `UnreviewedChanges` and `ReviewMode` as `IoEvent::Agent`. The first start asks the user. Nothing in the API can accept, reject or comment. |
| `stop_observing` | fn | `fyx:ui` 0.3: stops the agent feed. |
| `take_host_effects` | fn (host only) | Everything the host functions did on this thread since the last call, in order. Off WASM only, for unit tests of an extension. |
| `take_pushed_updates` | fn (host only) | The updates `push_update` sent on this thread since the last call; other effects stay recorded for `take_host_effects`. Off WASM only. |
| `write_stdin` | fn | `fyx:ui` 0.3: writes to a process spawned with `stdin`. At most 1 MiB per call and 4 MiB queued per process. |
