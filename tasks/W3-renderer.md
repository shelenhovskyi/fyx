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
