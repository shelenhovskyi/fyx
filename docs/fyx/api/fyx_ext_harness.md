# `fyx_ext_harness` public items

Generated from `crates/fyx_ext_harness/src/fyx_ext_harness.rs` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

Every public item, in path order. For signatures, run `script/fyx-api-docs` and open the rustdoc it builds.

| Item | Kind | Summary |
|---|---|---|
| `API_VERSION_SECTION` | const | The section `register_extension!` writes: three big-endian `u16`s. |
| `CALL_BUDGET` | const | Every later call. |
| `COMPONENT_VARIABLE` | const | `fyx-ext test` sets it to the built component; `Harness::from_env` reads it. |
| `EPOCH_INTERVAL` | const |  |
| `EventOutcome` | struct | What one event did. |
| `EventOutcome::pushed` | field | Pushed during the call, applied first, in order. |
| `EventOutcome::touched` | field | Node ids the reply touched in the event's surface (see `ui_ext::Tree::apply_update`). |
| `EventOutcome::update` | field | The call's own reply, applied to the event's surface after the pushes. |
| `Harness` | struct | One loaded extension and the trees of its mounted surfaces. |
| `Harness::api_version` | method |  |
| `Harness::click` | method | Clicks the first button with this label. |
| `Harness::disabled` | method | Why the extension is disabled, after a trap or an overrun call. |
| `Harness::event` | method | Sends one event to a mounted surface and applies what it pushed and replied. |
| `Harness::find` | method | The first node in tree order whose props match. |
| `Harness::find_button` | method |  |
| `Harness::find_text` | method |  |
| `Harness::from_env` | method | Loads the component named by `FYX_EXT_COMPONENT`, which `fyx-ext test` sets. |
| `Harness::handle_event_raw` | method | Calls `handle-event` without a mounted surface or applying the reply: what the host runtime does before the renderer applies anything. Pushes are applied and recorded as usual. `Err` means the call trapped or overran its budget, which disables the extension. |
| `Harness::limits` | method |  |
| `Harness::load` | method | Compiles and instantiates a component and asks it for its surfaces. |
| `Harness::load_file` | method |  |
| `Harness::logs` | method | Everything the extension logged so far. |
| `Harness::mount` | method | Mounts a declared surface (again, if it was mounted) and returns its validated tree. |
| `Harness::pushed` | method | Every update pushed so far, in order. |
| `Harness::rows` | method | Rows `start..start + count` of a list, each validated as a single-root tree. |
| `Harness::surfaces` | method |  |
| `Harness::text` | method | The content of a text node. |
| `Harness::tree` | method | The tree of a mounted surface, as the host holds it. |
| `INIT_BUDGET` | const | Instantiation and `surfaces()` together. |
| `MAX_PUSHES_PER_CALL` | const |  |
| `Pushed` | struct | An update the extension pushed during a call. |
| `Pushed::surface_id` | field |  |
| `Pushed::update` | field |  |
| `RawReply` | struct | One export call as the host receives it: the updates pushed during the call, and the guest's own result, whose `Err` is the message the guest returned. |
| `RawReply::pushed` | field |  |
| `RawReply::value` | field |  |
| `SUPPORTED_API_VERSIONS` | const | The `fyx:ui` versions the harness runs, the same as the host's supported range. |
| `api_version_section` | fn | The version in the component's `fyx:ui-api-version` section, if it has one. |
