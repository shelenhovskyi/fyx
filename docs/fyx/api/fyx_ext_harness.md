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
| `EditAnswer` | type | What the editor answers an edit the user allowed: `Ok` applies it (all or nothing), or an error such as a conflict. |
| `EventOutcome` | struct | What one event did, in the 0.1 model. |
| `EventOutcome::pushed` | field | Pushed during the call, applied first, in order. |
| `EventOutcome::touched` | field | Node ids the reply touched in the event's surface (see `ui_ext::Tree::apply_update`). |
| `EventOutcome::update` | field | The call's own reply, applied to the event's surface after the pushes. |
| `FakeProgram` | type | What a fake program does when it runs: it sees the request and everything written to its stdin, and returns its output and exit code. |
| `FakeRun` | struct | The run of a fake program: its whole stdout and stderr, and its exit code. |
| `FakeRun::code` | field | `None` as for a program ended by a signal. |
| `FakeRun::fail` | method | Prints `stderr` and exits with `code`. |
| `FakeRun::ok` | method | Prints `stdout` and exits with 0. |
| `FakeRun::stderr` | field |  |
| `FakeRun::stdout` | field |  |
| `FakeTerminal` | re-export | `terminals::FakeTerminal` |
| `FetchResponder` | type | What a test broker answers for one request it lets through. |
| `Harness` | struct | One loaded extension and the trees of its mounted surfaces and decorations. |
| `Harness::accessible_name` | method | The name a screen reader announces for a button node, by the renderer's rule: its `accessible-label` (at most 256 bytes), else a non-empty label, else the tooltip, else the icon's name. `None` for a node that is not a button. |
| `Harness::agent_event` | method | Queues an event of the host's agent review, with the host's ids, for `Harness::deliver_io`. Only a running feed delivers it; otherwise it is dropped. |
| `Harness::api_version` | method |  |
| `Harness::applied_edits` | method | The edits the editor applied, oldest first. |
| `Harness::broker` | method |  |
| `Harness::cancel_progress` | method | Clicks Cancel on a running cancellable progress; its owner, which must be mounted, gets `progress-cancel-requested`. The progress keeps running until the extension ends it. |
| `Harness::canvas` | method | The draw list of canvas node `id` on a mounted surface (0.3), for assertions. |
| `Harness::canvas_click` | method | Clicks canvas `id` at `(x, y)` in canvas coordinates, as the renderer does: it sends `canvas-click`, a gesture like a button click. It fails without a call when the node is not a canvas, the canvas is not `clickable`, or the point lies outside its box, since the renderer never sends those. |
| `Harness::click` | method | Clicks the first button with this label. |
| `Harness::click_v0_1` | method | `Harness::click` in the 0.1 model. |
| `Harness::client_received` | method | What the client got since the last call, in order: the host's answers, the extension's `respond`, `notify` and `send`. |
| `Harness::client_send` | method | The client sends one text frame. On an `mcp-*` listener it is a JSON-RPC message: the host's methods are answered at once, and a `tools/call` reaches the extension as `mcp-call` on `Harness::deliver_io`. On a `websocket` listener it is `listener-message`. |
| `Harness::close_notification` | method | Closes an open notification as the user (`User`), its timer (`Expired`) or its window (`WindowClosed`) would; its owner, which must be mounted, gets `notification-closed`. |
| `Harness::close_profile` | method | The user closes a profile terminal; with `terminal:observe`, `Harness::deliver_io` delivers `terminal-closed`. |
| `Harness::connect` | method | A client connects to the websocket kind `name` (`mcp-websocket`, `mcp-streamable-http` as one session, or `websocket`), having passed the host's token, `Origin` and `Host` checks. Returns the connection id the extension sees. |
| `Harness::decide_review` | method | The user decides proposal `id` in agent review; `Harness::deliver_io` delivers `review-decided`. False when `id` is not open (withdrawn, or decided already). |
| `Harness::decoration` | method | Where a live decoration is placed; its tree is `Harness::nodes` of its id. |
| `Harness::decorations` | method | The live decoration ids. |
| `Harness::deliver_io` | method | Delivers the outcome of every pending request through `handle-io`, oldest first, and applies each call's effects. A cancelled one ends `cancelled`. Otherwise the broker decides: an `Ask` state prompts once and takes the answer, a granted request gets the responder's answer, and anything else ends `denied`. Returns each call's effects and the extension's own result. |
| `Harness::disabled` | method | Why the extension is disabled, after a trap or an overrun call. |
| `Harness::disconnect` | method | The client closes `connection`; `Harness::deliver_io` delivers `mcp-cancelled` for its open calls. |
| `Harness::dismiss` | method | Closes a shown modal as the user would: its tree and pending tick go first, then `dismissed(reason)` is sent. Its effects apply; its reply does not, and should be `unchanged` (the host ignores anything else). |
| `Harness::edit_review` | method | The user edits proposal `id`'s text; `withdraw` returns it. |
| `Harness::effects` | method | Every effect so far, in order, including those of loading. |
| `Harness::event` | method | Sends one event to a mounted surface or decoration and applies its effects, then its reply. Typed text and a `row-selected` selection are written into the tree first. Use `Harness::dismiss` for `dismissed`. |
| `Harness::event_v0_1` | method | `Harness::event` in the 0.1 model: the pushes and the reply only. |
| `Harness::exit_terminal` | method | Ends a created terminal's program; `Harness::deliver_io` delivers `terminal-exited`. |
| `Harness::fail_review` | method | Proposal `id`'s review ends undecided (its file went away): `review-decided` with `Err`. |
| `Harness::fake_program` | method | Runs `program` for spawns of `name` instead of a real program. A declared program with no fake ends `spawn-failed(not-found)`. |
| `Harness::find` | method | The first node in tree order whose props match. |
| `Harness::find_button` | method |  |
| `Harness::find_text` | method |  |
| `Harness::find_v0_1` | method | `Harness::find` over the 0.1 model of the tree. |
| `Harness::from_env` | method | Loads what `fyx-ext test` names: the extension folder in `FYX_EXT_PACKAGE`, or else the component in `FYX_EXT_COMPONENT`. |
| `Harness::handle_event_raw` | method | Calls `handle-event` without a mounted surface or applying the reply: what the host runtime does before the renderer applies anything. Effects are applied and recorded as usual. `Err` means the call trapped or overran its budget, which disables the extension. |
| `Harness::handle_event_raw_v0_1` | method | `Harness::handle_event_raw` in the 0.1 model: the pushes and the reply only. |
| `Harness::http_reply` | method | The extension's `reply` to request `id`, once. |
| `Harness::http_request` | method | A request to the `http` listener `name`; `Harness::deliver_io` delivers it as `listener-request`. Returns its id. |
| `Harness::image_failures` | method | Image nodes the renderer would draw as their alt text, with the reason (design 0.3, section 12; decisions D1, D2, D4): an asset that is missing, outside the package, too large or not a valid PNG or SVG, a generated image that is no longer live, or `tint` on a PNG (drawn untinted). None of them is an update error. |
| `Harness::is_observing` | method | Whether the agent feed runs: started, allowed, and not stopped. |
| `Harness::is_shown` | method | Whether a modal is open: shown by the extension and not hidden, removed or dismissed. |
| `Harness::limits` | method |  |
| `Harness::listener_open` | method | Whether listener `name` is open: declared and granted. |
| `Harness::listener_port` | method | `${listen.<name>.port}` while the listener is open: a fake port, no socket. |
| `Harness::load` | method | Compiles and instantiates a component with every capability (`ALL_CAPABILITIES`) and asks it for its surfaces. |
| `Harness::load_file` | method |  |
| `Harness::load_package` | method | Loads an extension folder as Fyx installs it: `extension.toml` (its `[fyx_ui]` `component` and `capabilities`), the component, and `assets/` for image nodes. |
| `Harness::load_with_capabilities` | method | Like `Harness::load`, with the `[fyx_ui].capabilities` of a manifest. |
| `Harness::load_with_listeners` | method | Like `Harness::load_with_capabilities`, with the `[[fyx_ui.listeners]]` of a manifest (0.4, design.md B.7), checked as the host checks them: `network:listen` is declared and `ui_ext::integration::IntegrationTables::check` passes. They are known before the component is instantiated, so its constructor may register tools. `Harness::load_package` reads them from the manifest. |
| `Harness::logs` | method | Everything the extension logged so far, and the harness's own `warn` lines about what the renderer would have dropped. |
| `Harness::mount` | method | Mounts a live surface or decoration (again, if it was mounted) and returns its validated tree, parents first. A modal mounts only while it is shown. |
| `Harness::mount_v0_1` | method | `Harness::mount`, returning the tree in the 0.1 model. |
| `Harness::nodes` | method | The tree of a mounted surface or decoration, parents first. |
| `Harness::notification_action` | method | Clicks button `action` of an open notification, as the user would: the notification closes and its owner, which must be mounted, gets `notification-action`. The click is a gesture, like a button click. |
| `Harness::notifications` | method | The open notifications (design.md M.1), by id: shown and not yet dismissed, closed or acted on. The harness has no clock, so `auto_dismiss` ones stay until `Harness::close_notification`. |
| `Harness::open_mcp_calls` | method | The MCP calls the extension has not answered, by call id. |
| `Harness::open_profile` | method | The user starts profile `id` from "+", with account profile `account` when the profile has an `account_tool` (design.md C.6; the extension sees only its name). With `terminal:observe`, `Harness::deliver_io` delivers `terminal-created`. Returns the extension's handle for it. |
| `Harness::opened_files` | method | Every `context.open` the extension made, in order. |
| `Harness::pending_fetches` | method | Requests `fetch` accepted that `Harness::deliver_io` has not delivered yet, oldest first. |
| `Harness::pending_tick` | method | The delay of the surface's pending tick, already clamped up to `MIN_TICK_DELAY`. |
| `Harness::processes` | method | Processes `spawn` accepted that have not finished, oldest first. |
| `Harness::profile_command_line` | method | The command line the host would start for profile `id`: `${package}` is the package folder (`Harness::load_package`'s, or `<package>` for a harness loaded from bytes) and `${relay}` is `fyx-relay`. An argument whose placeholder has no value is left out, as the host does. |
| `Harness::profile_env` | method | The `[fyx_ui.terminal_env]` variables the host would set in profile `id`'s terminal, as `Harness::profile_command_line` fills templates (`${listen.<name>.token}` is `<token>`); a variable whose placeholder has no value is left out. Empty without `terminal_env` (only `Harness::load_package` reads it). |
| `Harness::progresses` | method | The running progresses (M.2), by id, with their last value and message. |
| `Harness::proposals` | method | The undecided `review` proposals (0.4), by id. Their paths need `Harness::set_worktrees`. |
| `Harness::props` | method | The props of one node of a mounted surface or decoration. |
| `Harness::pushed_v0_1` | method | Every update pushed so far, in order, in the 0.1 model; `Harness::effects` holds them in the newest one. |
| `Harness::registered_tools` | method | The tools the extension registered on listener `name` (0.4), in its order. |
| `Harness::relay_answer` | method | The extension's `relay.respond` to request `id`, once. |
| `Harness::relay_channels` | method | The relay channels, in manifest order. |
| `Harness::relay_request` | method | A hook runs `fyx-relay --channel <channel>` in `terminal` (the extension's handle for it) with `stdin`; `Harness::deliver_io` delivers `relay-request`. `channel` is `<extension-id>/<name>`, as hooks pass it, or the bare name. Returns the request's id, or why the host refuses it (the relay then prints the channel's `on_failure`). |
| `Harness::resize_terminal` | method | A new size for a terminal; `Harness::deliver_io` delivers the last one as `terminal-resized`. |
| `Harness::review_turns` | method | The extension's `begin-turn` and `end-turn` calls, in order. |
| `Harness::rows` | method | Rows `start..start + count` of a list, each validated as a single-root tree; `indent` as the extension sent it. |
| `Harness::rows_v0_1` | method | `Harness::rows` in the 0.1 model: each row's index and tree. |
| `Harness::set_agent_reviews` | method | The host's open agent reviews, which `agent-observe.start` sends first as a snapshot (with this extension's opaque ids, cut as the host cuts them). `None`: there are no agent reviews here, so `start` is `unavailable`. |
| `Harness::set_bracketed_paste` | method | Whether a profile terminal's program has bracketed paste on (it starts on); `paste` is `unavailable` while it is off (design.md B.11). |
| `Harness::set_broker` | method | Installs the permission broker and network for `network:fetch`; see `TestBroker`. |
| `Harness::set_diagnostics` | method | What the language servers report (0.4 `context.diagnostics`); while the extension watches, `Harness::deliver_io` delivers `diagnostics-changed` with the files whose entries changed. |
| `Harness::set_edit_answer` | method | How the editor answers `workspace:edit` edits the user allowed; without one they apply. |
| `Harness::set_editor_context` | method | What the editor shows now; while the extension watches, `Harness::deliver_io` delivers it, filtered as the host filters it, as `context-changed`. |
| `Harness::set_listener_granted` | method | Grants or revokes `network:listen#<name>`. Revoking closes the listener: its connections end, and `Harness::deliver_io` delivers `mcp-cancelled` for their open calls. |
| `Harness::set_private_file` | method | A file the user keeps private: `to-file-ref` and `absolute-path` refuse it, and the context carries no selection text of it. |
| `Harness::set_relay_channels` | method | The `[[fyx_ui.relay_channels]]` (0.4, design.md B.9), checked as the host checks them; `Harness::load_package` reads them from the manifest. They need `relay:hooks`. |
| `Harness::set_setting_specs` | method | The `[[fyx_ui.settings]]` (0.4, design.md §D), already checked with `settings::SettingSpec::parse_all`; `Harness::load_package` reads them from the manifest. Refused for a component below 0.4. The user's values are kept and checked again. |
| `Harness::set_user_settings` | method | The user edits `fyx_extensions.<id>` in the settings file: `values` is that object. An invalid value gives the extension its default. Returns the keys whose effective value changed, in manifest order: what `io-event::settings-changed` carries once the WIT has it (design.md §D.4). |
| `Harness::set_worktree` | method | The worktree `spawn` runs in: `None` for no open worktree (`unavailable`), or whether it is trusted (an untrusted one is `denied`). Trusted by default. |
| `Harness::set_worktrees` | method | The open worktrees for `context` (0.4): (id, root as the host spells it, `\` or `/`). The roots need not exist. |
| `Harness::setting` | method | What the extension gets from `settings.get(key)`; `None` for an undeclared key. |
| `Harness::setting_error` | method | Why the user's value for `key` is not passed on (the form shows it); `None` when it is valid or unset. |
| `Harness::setting_specs` | method | The declared settings, in manifest order. |
| `Harness::settings` | method | What the extension gets from `settings.all`: every declared key's effective value, in manifest order. |
| `Harness::surfaces` | method | The live surfaces: the static ones, then those `add-surface` added, minus removed ones. |
| `Harness::surfaces_v0_1` | method | `Harness::surfaces` in the 0.1 model; a 0.2 component's modals are left out. |
| `Harness::take_settings_changed` | method | The `settings-changed` events the host would have sent, oldest first, and forgets them. |
| `Harness::terminal` | method | One of the extension's 0.4 terminals (pseudo, created or profile), while it is open. |
| `Harness::terminal_nodes` | method | The handles the `terminal` nodes of a mounted surface show, in tree order. |
| `Harness::terminal_profiles` | method | The checked `[[fyx_ui.terminal_profiles]]`, in manifest order. |
| `Harness::terminals` | method | The extension's open 0.4 terminals, by handle. |
| `Harness::text` | method | The content of a text node. |
| `Harness::tick` | method | Runs the surface's pending tick, as the host does when it falls due, and applies its effects, then its reply. It fails without a call when no tick is pending, when the surface is not mounted, or after `MAX_CONSECUTIVE_TICK_ERRORS` failed ticks in a row until the next event on the surface. |
| `Harness::tree_v0_1` | method | The tree of a mounted surface or decoration in the 0.1 model, as the host's 0.1 view holds it. |
| `Harness::type_into_terminal` | method | Keys or text the user types into a pseudo-terminal; `Harness::deliver_io` hands them to the extension as `terminal-input`. |
| `Harness::ui_api_version` | method |  |
| `Harness::waiting_relay_requests` | method | The relay requests the extension has not answered, by id. |
| `Harness::watching_context` | method | Whether the extension watches the editor context. |
| `Harness::watching_diagnostics` | method | Whether the extension watches diagnostics. |
| `Harness::with_network_domains` | method | The hosts `network:fetch` may reach, as `[fyx_ui.network] domains` gives them; `Harness::load_package` reads them from the manifest. |
| `Harness::with_package_root` | method | Checks asset images against the extension folder `dir`, as `Harness::load_package` does, for a harness loaded from bytes. |
| `Harness::with_processes` | method | The programs `process:spawn` may run, as `[[fyx_ui.processes]]` and `[fyx_ui.process] env` declare them; `Harness::load_package` reads them from the manifest. |
| `Harness::with_terminal_profiles` | method | The profiles the user can start from "+", as `[[fyx_ui.terminal_profiles]]` declares them (design.md B.1), with the host's checks: `terminal:profiles` is declared, the entries pass `ui_ext::terminal::check_profiles`, and `${package}` needs `package:files`. `Harness::load_package` reads them from the manifest. |
| `Harness::with_terminal_programs` | method | The programs `terminal:create` may run, as `[[fyx_ui.terminal_programs]]` declares them; `Harness::load_package` reads them from the manifest. |
| `Harness::withdrawn_reviews` | method | The proposals the extension withdrew, in order. |
| `HostEffect` | re-export | `api_state::HostEffect` |
| `INIT_BUDGET` | const | Instantiation and `surfaces()` together. |
| `INSTANCES_LIMIT` | const |  |
| `ListenerEvent` | re-export | `listeners::ListenerEvent` |
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
| `McpCall` | re-export | `listeners::McpCall` |
| `McpError` | re-export | `listeners::McpError` |
| `OpenedFile` | re-export | `context::OpenedFile` |
| `Outcome` | struct | What one event or tick did. |
| `Outcome::effects` | field | The call's effects, applied first, in order. |
| `Outcome::invalidated` | field | The reply's `invalidate-rows` patches, in order. |
| `Outcome::touched` | field | Node ids the reply touched, including the targets of `invalidate-rows`. |
| `Outcome::update` | field | The call's own reply, applied to the surface after the effects. For `dismissed` it is not applied: the modal has no tree any more. |
| `PACKAGE_VARIABLE` | const | `fyx-ext test` sets it to the extension folder; `Harness::from_env` prefers it. |
| `Progress` | struct | A running progress as the renderer draws it (design.md M.2). |
| `Progress::message` | field | The last message an update gave; an update without one keeps it. |
| `Progress::options` | field |  |
| `Progress::value` | field | Indeterminate until the first `progress-update`. |
| `Pushed` | struct | An update the extension pushed during a call, in the 0.1 model. |
| `Pushed::surface_id` | field |  |
| `Pushed::update` | field |  |
| `RawEffects` | struct | One export call as the host receives it: the effects of the call, applied, and the guest's own result, not applied. |
| `RawEffects::effects` | field |  |
| `RawEffects::value` | field |  |
| `RawReply` | struct | One export call as the host receives it, in the 0.1 model: the updates pushed during the call, and the guest's own result, whose `Err` is the message the guest returned. |
| `RawReply::pushed` | field |  |
| `RawReply::value` | field |  |
| `ReviewTurn` | re-export | `review::ReviewTurn` |
| `SUPPORTED_API_VERSIONS` | const | The `fyx:ui` versions the harness runs, the same as the host's supported range. |
| `TABLES_LIMIT` | const |  |
| `TABLE_ELEMENTS_LIMIT` | const |  |
| `TestBroker` | struct | A stand-in for the user and the network, for `network:fetch` and `process:spawn` tests (0.3); its state is the state of every T2 capability. Without one the harness grants no T2 capability, as the host without a broker: `fetch` returns `denied`. |
| `TestBroker::answer` | field | How the user answers the prompt of an `Ask` state. |
| `TestBroker::asking` | method | Not asked yet: the first delivered request prompts, and the user answers `answer`. |
| `TestBroker::denied` | method | Refused: `fetch` returns `denied` at the import. |
| `TestBroker::granted` | method | Granted already: `query` says `granted`, nothing prompts, and every request reaches `respond`. |
| `TestBroker::prompts` | method | How many times the prompt was shown. |
| `TestBroker::state` | field | The state at the import. `Ask` accepts the request and asks when it is delivered. |
| `ToolDef` | re-export | `listeners::ToolDef` |
| `api_version_section` | fn | The version in the component's `fyx:ui-api-version` section, if it has one. |
| `cache_compiled_components` | re-export | `compile_cache::cache_compiled_components` |
| `plain_text` | re-export | `terminals::plain_text` |
| `settings` | module |  |
| `variables_set` | fn | Whether `PACKAGE_VARIABLE` or `COMPONENT_VARIABLE` is set, as under `fyx-ext test`. A headless test skips when neither is, and fails when `Harness::from_env` fails although one is. |
