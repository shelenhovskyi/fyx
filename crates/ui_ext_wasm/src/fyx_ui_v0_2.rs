//! Host side of the `fyx:ui@0.2.0` world (`crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit`): its
//! bindings, the conversions into `ui_ext::v0_2`, and the 0.2 host imports.
//!
//! Every import with an effect is checked when it is called (design.md, A) against the state in
//! [`ApiState`], and its effect is recorded there, to be returned with the call's reply.

use ui_ext::v0_2 as model;

use crate::{UiState, conversions};

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit/since_v0.2.0",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
        exports: {
            default: async,
        },
    });
}

pub(crate) use bindings::UiExtension;
use bindings::fyx::ui::{
    decorations as wit_decorations, host as wit_host, notifications as wit_notifications,
    types as wit,
};

conversions::host_imports!(denied: wit_host::HostErrorCode::NotPermitted);
conversions::notification_imports!();
conversions::decoration_imports!();

conversions::conversions! {
    props: {}
    kinds: {}
    events: {
        // Never sent: a 0.2 surface has no canvas, and `call_handle_event` drops the event
        // before it gets here (defence in depth, like 0.1's).
        model::UiEvent::CanvasClick(click) => Self::Click(click.canvas),
    }
}
