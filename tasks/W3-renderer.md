# W3 Renderer and containers

**Owns:** `crates/ui_ext_host/`, the panel hook in `crates/workspace/src/dock.rs`, the `ui_ext_host::init` line and dependency in `crates/zed`.

**Deliverable**
- `ExtensionPanel` implementing `workspace::Panel` on behalf of an extension surface; status-bar item via `StatusItemView`.
- Map the primitives (flex, text, button, input, list, table, icon) to GPUI/`ui` elements; colours only from theme tokens.
- Route clicks/input by node id → one `handle-event` call → apply patches → `cx.notify()`. Never call WASM from `render()`.
- Dynamic panel registration from the extension host instead of the static list in `crates/zed/src/zed.rs`.

**Reference:** `ProjectPanel`, `AgentPanel`, `crates/workspace/src/dock.rs`.

**Iterate with:** `cargo check -p ui_ext_host`.

**Done when:** the W4 counter demo appears in the dock and its button updates the count.

## Backlog

**Plan 1.6 follow-ups** (integration-4 finding 7; deferred by the coordinator, 2026-11-18). 52a9103c61
capped the text lifted per call; still open:
- A byte cap on the retained tree per surface (for example 32 MiB of `Props` text, checked in
  `ui_ext::Tree` alongside `max_nodes`), since node counts alone do not bound memory across calls.
- A cap on the WASI `ResourceTable` (`ResourceTable::set_max_capacity`; the default is 1 000 000).
- A memory bound for the whole store instead of per memory: a `ResourceLimiter` that sums every
  memory's size against `UI_MEMORY_LIMIT` (8 memories are allowed now), or one memory if every
  supported guest (Rust, componentize-js) needs only one.
