//! Host side of the staged `fyx:ui@0.3.0` world (`crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit`),
//! as far as it is built: what 0.3 copies forward from 0.2 (`types`, `host`), and the `images`
//! interface of design.md section 12 (plan 4.10). Every other 0.3 interface is linked as a trap
//! until its plan task lands (`UiBindings::instantiate`), so a component that never calls them
//! loads and runs.
//!
//! Values the `ui_ext` model cannot hold yet are refused as `invalid-argument` (in a push) or as
//! the call's error (in a reply): `image` and `canvas` nodes and `picker` surfaces. The image
//! checks themselves are in `ui_ext::image` and run on `add-image` already.

use ui_ext::{
    image::{self as model_image},
    v0_2 as model,
};

use crate::{
    UiState,
    conversions::{self, unsupported},
};

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit/since_v0.3.0",
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
use bindings::fyx::ui::{host as wit_host, images as wit_images, types as wit};

conversions::host_imports!();

impl wit_images::Host for UiState {
    fn add_image(
        &mut self,
        format: wit_images::ImageFormat,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<u32, wit_host::HostError>> {
        if let Err(error) = self.permit(model_image::CAPABILITY_IMAGES) {
            return Ok(Err(error.into()));
        }
        let format = match format {
            wit_images::ImageFormat::Png => model_image::ImageFormat::Png,
            wit_images::ImageFormat::Svg => model_image::ImageFormat::Svg,
        };
        Ok(self.api.add_image(format, bytes).map_err(Into::into))
    }

    fn remove_image(&mut self, image: u32) -> wasmtime::Result<()> {
        self.api.remove_image(image);
        Ok(())
    }
}

conversions::conversions! {
    props: {
        wit::Props::Image(_) => return Err(unsupported("an image node")),
        wit::Props::Canvas(_) => return Err(unsupported("a canvas node")),
    }
    kinds: {
        wit::SurfaceKind::Picker(_) => return Err(unsupported("a picker surface")),
    }
}

/// The 0.3 linker: WASI and the interfaces above; the rest trap (see the module doc).
pub(crate) fn add_to_linker(
    linker: &mut wasmtime::component::Linker<UiState>,
) -> wasmtime::Result<()> {
    wit::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_host::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_images::add_to_linker::<_, UiState>(linker, |state| state)?;
    Ok(())
}
