# `fyx_ext_harness` public items

Generated from `crates/fyx_ext_harness/src/fyx_ext_harness.rs` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

Every public item, in path order. For signatures, run `script/fyx-api-docs` and open the rustdoc it builds.

| Item | Kind | Summary |
|---|---|---|
| `ACTIVATION_WINDOW` | re-export | `api_state::ACTIVATION_WINDOW` |
| `ALL_CAPABILITIES` | const | What `Harness::load` grants: every `[fyx_ui].capabilities` entry a component can use. |
| `API_VERSION_SECTION` | const | The section `register_extension!` writes: three big-endian `u16`s. |
| `CALL_BUDGET` | const | Every later call. |
| `COMPONENT_VARIABLE` | const | `fyx-ext test` sets it to the built component; `Harness::from_env` reads it. |
| `EPOCH_INTERVAL` | const |  |
| `EventOutcome` | struct | What one event did, in the 0.1 model. |
| `EventOutcome::pushed` | field | Pushed during the call, applied first, in order. |
| `EventOutcome::touched` | field | Node ids the reply touched in the event's surface (see `ui_ext::Tree::apply_update`). |
| `EventOutcome::update` | field | The call's own reply, applied to the event's surface after the pushes. |
| `Harness` | struct | One loaded extension and the trees of its mounted surfaces and decorations. |
| `Harness::api_version` | method |  |
| `Harness::click` | method | Clicks the first button with this label (0.1 model). |
| `Harness::click_v0_2` | method | Clicks the first button with this label. |
| `Harness::decoration` | method | Where a live decoration is placed; its tree is `Harness::tree` of its id. |
| `Harness::decorations` | method | The live decoration ids. |
| `Harness::disabled` | method | Why the extension is disabled, after a trap or an overrun call. |
| `Harness::dismiss` | method | Closes a shown modal as the user would: its tree and pending tick go first, then `dismissed(reason)` is sent. Its effects apply; its reply does not, and should be `unchanged` (the host ignores anything else). |
| `Harness::effects` | method | Every effect so far, in order, including those of loading. |
| `Harness::event` | method | Sends one event to a mounted surface and applies what it pushed and replied (0.1 model). |
| `Harness::event_v0_2` | method | Sends one event to a mounted surface or decoration and applies its effects, then its reply. Typed text and a `row-selected` selection are written into the tree first. Use `Harness::dismiss` for `dismissed`. |
| `Harness::find` | method | The first node in tree order whose props match. |
| `Harness::find_button` | method |  |
| `Harness::find_text` | method |  |
| `Harness::from_env` | method | Loads what `fyx-ext test` names: the extension folder in `FYX_EXT_PACKAGE`, or else the component in `FYX_EXT_COMPONENT`. |
| `Harness::handle_event_raw` | method | Calls `handle-event` without a mounted surface or applying the reply: what the host runtime does before the renderer applies anything. Pushes are applied and recorded as usual. `Err` means the call trapped or overran its budget, which disables the extension. |
| `Harness::handle_event_raw_v0_2` | method | `Harness::handle_event_raw` in the 0.2 model, with every effect. |
| `Harness::image_failures` | method | Image nodes the renderer would draw as their alt text, with the reason (design 0.3, section 12; decisions D1, D2, D4): an asset that is missing, outside the package, too large or not a valid PNG or SVG, a generated image that is no longer live, or `tint` on a PNG (drawn untinted). None of them is an update error. |
| `Harness::is_shown` | method | Whether a modal is open: shown by the extension and not hidden, removed or dismissed. |
| `Harness::limits` | method |  |
| `Harness::load` | method | Compiles and instantiates a component with every capability (`ALL_CAPABILITIES`) and asks it for its surfaces. |
| `Harness::load_file` | method |  |
| `Harness::load_package` | method | Loads an extension folder as Fyx installs it: `extension.toml` (its `[fyx_ui]` `component` and `capabilities`), the component, and `assets/` for image nodes. |
| `Harness::load_with_capabilities` | method | Like `Harness::load`, with the `[fyx_ui].capabilities` of a manifest. |
| `Harness::logs` | method | Everything the extension logged so far, and the harness's own `warn` lines about what the renderer would have dropped. |
| `Harness::mount` | method | Mounts a live surface or decoration (again, if it was mounted) and returns its validated tree. A modal mounts only while it is shown. |
| `Harness::nodes_v0_2` | method | The tree of a mounted surface or decoration in the 0.2 model, parents first. |
| `Harness::pending_tick` | method | The delay of the surface's pending tick, already clamped up to `MIN_TICK_DELAY`. |
| `Harness::props_v0_2` | method | The 0.2 props of one node of a mounted surface or decoration. |
| `Harness::pushed` | method | Every update pushed so far, in order, in the 0.1 model. |
| `Harness::rows` | method | Rows `start..start + count` of a list, each validated as a single-root tree (0.1 model). |
| `Harness::rows_v0_2` | method | Rows `start..start + count` of a list, each validated as a single-root tree; `indent` as the extension sent it. |
| `Harness::surfaces` | method | The live surfaces in the 0.1 model; a 0.2 component's modals are left out. |
| `Harness::surfaces_v0_2` | method | The live surfaces: the static ones, then those `add-surface` added, minus removed ones. |
| `Harness::text` | method | The content of a text node. |
| `Harness::tick` | method | Runs the surface's pending tick, as the host does when it falls due, and applies its effects, then its reply. It fails without a call when no tick is pending, when the surface is not mounted, or after `MAX_CONSECUTIVE_TICK_ERRORS` failed ticks in a row until the next event on the surface. |
| `Harness::tree` | method | The tree of a mounted surface or decoration, as the host holds it. |
| `Harness::ui_api_version` | method |  |
| `Harness::with_package_root` | method | Checks asset images against the extension folder `dir`, as `Harness::load_package` does, for a harness loaded from bytes. |
| `HostEffect` | re-export | `api_state::HostEffect` |
| `INIT_BUDGET` | const | Instantiation and `surfaces()` together. |
| `INSTANCES_LIMIT` | const |  |
| `MAX_CONSECUTIVE_TICK_ERRORS` | const | Consecutive `tick` errors after which a surface's ticks stop until an event on it. |
| `MAX_LIFTED_TEXT_PER_CALL` | const | String bytes all of one call's host-import arguments may carry together (`push-update`, `log`, and in 0.2 every other import); past it the call traps. |
| `MAX_LOG_MESSAGE_LEN` | const | Longest `log` message kept; the rest is cut off. It still counts in full toward `MAX_LIFTED_TEXT_PER_CALL`. |
| `MAX_PUSHED_ITEMS_PER_CALL` | const | Patches plus nodes all of one call's 0.1 pushes may carry; past it the call traps. |
| `MAX_PUSHED_ITEMS_PER_CALL_V0_2` | re-export | `api_state::MAX_PUSHED_ITEMS_PER_CALL_V0_2` |
| `MAX_PUSHES_PER_CALL` | const |  |
| `MAX_TICK_DELAY` | re-export | `api_state::MAX_TICK_DELAY` |
| `MEMORIES_LIMIT` | const |  |
| `MEMORY_LIMIT` | const | Linear memory one component may grow to, and its other store caps: the host's (`ui_ext_wasm::UI_*_LIMIT`). Growing past one traps, which disables the extension. |
| `MIN_TICK_DELAY` | re-export | `api_state::MIN_TICK_DELAY` |
| `Outcome` | struct | What one event or tick did. |
| `Outcome::effects` | field | The call's effects, applied first, in order. |
| `Outcome::invalidated` | field | The reply's `invalidate-rows` patches, in order. |
| `Outcome::touched` | field | Node ids the reply touched, including the targets of `invalidate-rows`. |
| `Outcome::update` | field | The call's own reply, applied to the surface after the effects. For `dismissed` it is not applied: the modal has no tree any more. |
| `PACKAGE_VARIABLE` | const | `fyx-ext test` sets it to the extension folder; `Harness::from_env` prefers it. |
| `Pushed` | struct | An update the extension pushed during a call, in the 0.1 model. |
| `Pushed::surface_id` | field |  |
| `Pushed::update` | field |  |
| `RawEffects` | struct | One export call as the host receives it: the effects of the call, applied, and the guest's own result, not applied. |
| `RawEffects::effects` | field |  |
| `RawEffects::value` | field |  |
| `RawReply` | struct | One export call as the host receives it, in the 0.1 model: the updates pushed during the call, and the guest's own result, whose `Err` is the message the guest returned. |
| `RawReply::pushed` | field |  |
| `RawReply::value` | field |  |
| `SUPPORTED_API_VERSIONS` | const | The `fyx:ui` versions the harness runs, the same as the host's supported range. |
| `TABLES_LIMIT` | const |  |
| `TABLE_ELEMENTS_LIMIT` | const |  |
| `api_version_section` | fn | The version in the component's `fyx:ui-api-version` section, if it has one. |
