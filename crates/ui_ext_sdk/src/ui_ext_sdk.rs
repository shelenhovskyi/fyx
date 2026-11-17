//! Apache-2.0 so that extension authors, proprietary ones included, stay outside the GPL.

pub mod bindings {
    wit_bindgen::generate!({
        path: "../ui_ext/wit",
        world: "ui-extension",
        pub_export_macro: true,
        default_bindings_module: "ui_ext_sdk::bindings",
    });
}
