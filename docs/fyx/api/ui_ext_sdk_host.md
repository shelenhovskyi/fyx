# `ui_ext_sdk_host` public items

Generated from `crates/ui_ext_sdk/src/host.rs` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

Every public item, in path order. For signatures, run `script/fyx-api-docs` and open the rustdoc it builds.

| Item | Kind | Summary |
|---|---|---|
| `HostEffect` | enum | What a host function did, recorded off WASM for unit tests. |
| `HostEffect::AddSurface` | variant |  |
| `HostEffect::CancelTick` | variant |  |
| `HostEffect::HideSurface` | variant |  |
| `HostEffect::PlaceDecoration` | variant |  |
| `HostEffect::Push` | variant |  |
| `HostEffect::RemoveDecoration` | variant |  |
| `HostEffect::RemoveSurface` | variant |  |
| `HostEffect::RequestTick` | variant |  |
| `HostEffect::ShowSurface` | variant |  |
| `add_surface` | fn | Adds a surface at runtime. The manifest must declare the capability of its kind; at most 32 live surfaces per extension. Panels and status items are mounted at once; a modal waits for `show_surface`. |
| `cancel_tick` | fn | Drops the pending tick of a surface, if any. |
| `hide_surface` | fn | Closes a modal (no `dismissed` event) or hides a panel's dock if that panel is active. |
| `log` | fn | Writes a message to the host's log. Off WASM it goes to stderr. |
| `place_decoration` | fn | Places a decoration: a tree drawn as a block above or below a buffer line, anchored by the host. Needs `ui:editor-decorations`; at most 512 per extension. Placing a live id again moves it and replaces its tree. The id then works like a surface id for `push_update`, `handle-event`, `list-rows` and `mount`. |
| `push_update` | fn | Sends an update for `surface_id` during a call: from `mount`, `handle-event`, `list-rows` or `tick`, typically for a surface other than the one the call is about. |
| `remove_decoration` | fn | Removes a live decoration in every editor; no event is sent. |
| `remove_surface` | fn | Removes a live surface, static or runtime. No event is sent; the id may be added again. |
| `request_tick` | fn | Asks for one `tick(surface_id)` call at least `delay_ms` after the current call returns. One pending tick per surface: a new request replaces the earlier one. The host clamps the delay up to 100 ms, and further while Fyx is in the background; a tick for a hidden panel waits until it is shown. Re-request it inside `Extension::tick`(crate::Extension::tick) for a periodic timer. |
| `show_surface` | fn | Opens a modal and focuses it, or reveals a panel. A modal needs a click or Enter: it is allowed during `handle-event` for `click`, `row-click` or `input-submitted`, or within 5 s after one, once. |
| `take_host_effects` | fn (host only) | Everything the host functions did on this thread since the last call, in order. Off WASM only, for unit tests of an extension. |
| `take_pushed_updates` | fn (host only) | The updates `push_update` sent on this thread since the last call; other effects stay recorded for `take_host_effects`. Off WASM only. |
