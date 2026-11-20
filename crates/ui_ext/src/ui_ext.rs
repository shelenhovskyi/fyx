//! Transport-independent model of the `fyx:ui` contract. The same tree and patch types must be
//! reachable from a WASM component and, later, from a JSON-RPC sidecar, so nothing here may
//! depend on wasmtime or GPUI.
//!
//! The crate root mirrors the 0.1 world; [`v0_2`] mirrors the 0.2 world and is the model 0.1 data
//! converts into. [`ApiVersion`] selects the rules that differ between the two.

mod api_version;
/// fyx:ui 0.4 approval cards (§B.12).
pub mod approvals;
pub mod canvas;
#[cfg(feature = "t2")]
pub mod context;
#[cfg(feature = "t2")]
pub mod edit;
/// fyx:ui 0.4 file decorations (plan 6.7, design §F.2).
#[cfg(feature = "t2")]
pub mod file_decorations;
pub mod image;
/// fyx:ui 0.4 integration kit manifest tables (plan 5.5).
#[cfg(feature = "t2")]
pub mod integration;
#[cfg(feature = "t2")]
pub mod network;
#[cfg(feature = "t2")]
pub mod observe;
#[cfg(feature = "t2")]
pub mod process;
pub mod protected;
/// fyx:ui 0.4 diagnostics from extensions (plan 6.6, design §E).
#[cfg(feature = "t2")]
pub mod published_diagnostics;
#[cfg(feature = "t2")]
pub mod review;
/// Plan 7.5: the macOS Seatbelt profile of a `process:spawn` child (pure text, every OS).
#[cfg(feature = "process")]
pub mod seatbelt;
/// fyx:ui 0.4 extension settings (plan 6.1, design §D): no capability, no format crate.
pub mod settings;
/// fyx:ui 0.4 terminals (plan 5.5); behind `t2` until W0 adds a `terminal` feature alias.
#[cfg(feature = "t2")]
pub mod terminal;
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

/// Untrusted text (an extension's `log` messages and errors) as it goes into a log line:
/// control characters, newlines included, are escaped, so one entry cannot forge another
/// (triage-3, SEC T6), and the text is cut at `MAX_LOGGED_TEXT` characters. The same rule as
/// `fyx_guard`'s helper of this name, with room for an error's context chain.
pub fn sanitize_for_log(text: &str) -> String {
    const MAX_LOGGED_TEXT: usize = 2048;
    let mut sanitized = String::with_capacity(text.len().min(MAX_LOGGED_TEXT));
    for (count, character) in text.chars().enumerate() {
        if count == MAX_LOGGED_TEXT {
            sanitized.push('…');
            break;
        }
        if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
            sanitized.extend(character.escape_default());
        } else {
            sanitized.push(character);
        }
    }
    sanitized
}

#[cfg(test)]
mod log_tests {
    #[test]
    fn test_sanitize_for_log() {
        assert_eq!(
            super::sanitize_for_log("ok\n[ERROR] forged\r\u{1b}[31m"),
            r"ok\n[ERROR] forged\r\u{1b}[31m"
        );
        assert_eq!(super::sanitize_for_log("plain — text"), "plain — text");
        let long = "x".repeat(3000);
        assert_eq!(super::sanitize_for_log(&long).chars().count(), 2049);
    }
}

/// Directory holding the 0.1 `fyx-ui.wit`, for `wasmtime::component::bindgen!` in the host. WIT
/// tools read only the `.wit` files directly in it, so the 0.2 directory inside it is not part of
/// this package.
pub const WIT_DIR: &str = WIT_DIR_V0_1_0;
pub const WIT_DIR_V0_1_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");
pub const WIT_DIR_V0_2_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit/since_v0.2.0");
/// The staged 0.3 world (docs/fyx/wit-0.3/design.md); its images model is in [`image`].
pub const WIT_DIR_V0_3_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit/since_v0.3.0");
/// The staged 0.4 world (docs/fyx/wit-0.4/design.md); its terminal model is in `terminal` (feature `t2`).
pub const WIT_DIR_V0_4_0: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit/since_v0.4.0");
