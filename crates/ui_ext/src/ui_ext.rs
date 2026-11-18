//! Transport-independent model of the `fyx:ui` contract. The same tree and patch types must be
//! reachable from a WASM component and, later, from a JSON-RPC sidecar, so nothing here may
//! depend on wasmtime or GPUI.
//!
//! The crate root mirrors the 0.1 world; [`v0_2`] mirrors the 0.2 world and is the model 0.1 data
//! converts into. [`ApiVersion`] selects the rules that differ between the two.

mod api_version;
pub mod image;
mod tree;
mod types;
pub mod v0_2;

pub use api_version::{
    ApiError, ApiVersion, IdKind, IdLimits, LiveIds, MAX_DECORATION_EFFECTS_PER_CALL,
    MAX_DECORATION_PATH_BYTES, MAX_DECORATIONS, MAX_ID_BYTES, MAX_INDENT_LEVELS, MAX_SURFACES,
    MAX_TOOLTIP_BYTES, PathProblem, Placement, check_decoration_path,
};
pub use tree::{Tree, TreeError, TreeLimits, TreeNode};
pub use types::*;

/// Directory holding the 0.1 `fyx-ui.wit`, for `wasmtime::component::bindgen!` in the host. WIT
/// tools read only the `.wit` files directly in it, so the 0.2 directory inside it is not part of
/// this package.
pub const WIT_DIR: &str = WIT_DIR_V0_1_0;
pub const WIT_DIR_V0_1_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");
pub const WIT_DIR_V0_2_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit/since_v0.2.0");
/// The staged 0.3 world (docs/fyx/wit-0.3/design.md); its images model is in [`image`].
pub const WIT_DIR_V0_3_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit/since_v0.3.0");
