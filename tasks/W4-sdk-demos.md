# W4 Guest SDK and demos

**Owns:** `crates/ui_ext_sdk/`, `extensions/fyx_demo_*`, `script/fyx-build-demos`.

**Deliverable**
- Ergonomic Rust SDK over the generated bindings: a node builder, `register_extension!`-style macro, patch helpers. Apache-2.0.
- Demo 1: counter / list panel.
- Demo 2: workspace TODO aggregator (needs worktree read access; mock until W2 exposes it).
- Both build for `wasm32-wasip2` (`cargo build -p <demo> --target wasm32-wasip2`).

**Done when:** both demos build to components that `wasm-tools component wit` reports as implementing `fyx:ui/ui-extension`.
