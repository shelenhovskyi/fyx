//! Transport-independent model of the `fyx:ui` contract. The same tree and patch types must be
//! reachable from a WASM component and, later, from a JSON-RPC sidecar, so nothing here may
//! depend on wasmtime or GPUI.

/// Directory holding `fyx-ui.wit`, for `wasmtime::component::bindgen!` in the host.
pub const WIT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");
