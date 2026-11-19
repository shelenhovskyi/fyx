//! The `fyx:ui@0.2.0` guest bindings, used by `ui_ext_sdk`'s `register_extension!(T, v0_2)` to
//! export an extension written against the SDK's 0.3 API to hosts that run 0.2.
//!
//! A crate of its own for the same reason as `ui_ext_sdk_v0_1`: wit-bindgen names a world's
//! exported macro after the world, and every version calls its world `ui-extension`. Extensions
//! never depend on it directly.

pub mod bindings {
    wit_bindgen::generate!({
        path: "../ui_ext/wit/since_v0.2.0",
        world: "ui-extension",
        pub_export_macro: true,
        default_bindings_module: "ui_ext_sdk_v0_2::bindings",
        additional_derives: [PartialEq, Eq],
    });
}
