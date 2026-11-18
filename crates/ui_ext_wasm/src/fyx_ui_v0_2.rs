//! Host side of the `fyx:ui@0.2.0` world (`crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit`): its
//! bindings, the conversions into `ui_ext::v0_2`, and the 0.2 host imports.
//!
//! Every import with an effect is checked when it is called (design.md, A) against the state in
//! [`ApiState`], and its effect is recorded there, to be returned with the call's reply.

use ui_ext::v0_2 as model;

use crate::{
    UiState,
    conversions::{self, try_collect},
};

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

impl wit_decorations::Host for UiState {
    fn place_decoration(
        &mut self,
        id: String,
        location: wit::DecorationLocation,
        nodes: Vec<wit::Node>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        let nodes: Vec<model::Node> = match try_collect(nodes) {
            Ok(nodes) => nodes,
            Err(error) => return result(Err(error)),
        };
        self.count_lifted_text(
            id.len()
                + location.path.len()
                + nodes
                    .iter()
                    .map(|node| conversions::props_text_len(&node.props))
                    .sum::<usize>(),
        )?;
        if let Err(error) = self.permit(model::CAPABILITY_EDITOR_DECORATIONS) {
            return result(Err(error));
        }
        result(self.api.place_decoration(id, location.into(), nodes))
    }

    fn remove_decoration(
        &mut self,
        id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(id.len())?;
        if let Err(error) = self.permit(model::CAPABILITY_EDITOR_DECORATIONS) {
            return result(Err(error));
        }
        result(self.api.remove_decoration(id))
    }
}

conversions::conversions! {
    props: {}
    kinds: {}
}

impl From<wit::DecorationLocation> for model::DecorationLocation {
    fn from(value: wit::DecorationLocation) -> Self {
        Self {
            path: value.path,
            line: value.line,
            placement: match value.placement {
                wit::DecorationPlacement::Above => model::DecorationPlacement::Above,
                wit::DecorationPlacement::Below => model::DecorationPlacement::Below,
            },
            height_lines: value.height_lines,
        }
    }
}
