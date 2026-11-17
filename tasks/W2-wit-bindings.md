# W2 WIT and host bindings

**Owns:** `crates/ui_ext/src/`, a new `fyx_ui` module inside `crates/extension_host/src/wasm_host/`.

**Deliverable**
- Host bindings for `fyx:ui@0.1.0` via `wasmtime::component::bindgen!` (path `ui_ext::WIT_DIR`), on the wasmtime version Zed pins.
- Rust tree model in `ui_ext`: build a tree from `list<node>`, apply patches, validate (parents precede children, unique ids, node cap per patch). Unit tests without GPUI.
- UI calls on their own queue or `Store` so they are not stuck behind slow calls of the same extension; shorter epoch deadline for UI calls.
- Version gating for the `fyx:ui` world that does not inherit the stable-channel cap at v0.6.0.
- First task: confirm against the current component-model spec that WIT types cannot be recursive (the flat-tree design depends on it).

**Do not** edit the WIT; request changes from W0.

**Done when:** a test instantiates a component built with `ui_ext_sdk`, calls `surfaces`/`mount`/`handle-event`, and applies the returned patches.
