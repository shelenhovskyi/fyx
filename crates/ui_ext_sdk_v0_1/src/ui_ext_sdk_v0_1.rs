//! The `fyx:ui@0.1.0` guest bindings, used by `ui_ext_sdk`'s `register_extension!(T, v0_1)` to
//! export an extension written against the SDK's 0.2 API to hosts that only run 0.1.
//!
//! A crate of its own because wit-bindgen names a world's exported macro after the world, and
//! both versions call their world `ui-extension`: two `generate!` calls in one crate would export
//! two macros with the same name. Extensions never depend on it directly.

pub mod bindings {
    wit_bindgen::generate!({
        path: "../ui_ext/wit",
        world: "ui-extension",
        pub_export_macro: true,
        default_bindings_module: "ui_ext_sdk_v0_1::bindings",
        additional_derives: [PartialEq, Eq],
    });
}
