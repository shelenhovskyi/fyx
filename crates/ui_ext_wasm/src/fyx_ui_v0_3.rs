//! Host side of the staged `fyx:ui@0.3.0` world (`crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit`),
//! as far as it is built: what 0.3 copies forward from 0.2 (`types`, `host`, `notifications`), and the `images`
//! interface of design.md section 12 (plan 4.10). Every other 0.3 interface is linked as a trap
//! until its plan task lands (`define_unbuilt_as_traps`), so a component that never calls them
//! loads and runs.
//!
//! `image` nodes convert into [`model::Props::Image`]; their checks (`ui_ext::image`) run when the
//! update or the reply is applied ([`crate::effects::ApiState::check_images`]). Values the
//! `ui_ext` model cannot hold yet are refused as `invalid-argument` (in a push) or as the call's
//! error (in a reply): `canvas` nodes and `picker` surfaces.

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
use bindings::fyx::ui::{
    host as wit_host, images as wit_images, notifications as wit_notifications, types as wit,
};

conversions::host_imports!();
conversions::notification_imports!();

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
        wit::Props::Image(props) => Self::Image(model_image::ImageProps {
            source: match props.source {
                wit::ImageSource::Asset(path) => model_image::ImageSource::Asset(path),
                wit::ImageSource::Generated(handle) => model_image::ImageSource::Generated(handle),
            },
            alt: props.alt,
            width: props.width,
            height: props.height,
            fit: match props.fit {
                wit::ImageFit::Contain => model_image::ImageFit::Contain,
                wit::ImageFit::Cover => model_image::ImageFit::Cover,
            },
            tint: props.tint.map(Into::into),
        }),
        wit::Props::Canvas(_) => return Err(unsupported("a canvas node")),
    }
    kinds: {
        wit::SurfaceKind::Picker(_) => return Err(unsupported("a picker surface")),
    }
}

/// The `fyx:ui` 0.3 interfaces [`add_to_linker`] links.
const LINKED_INTERFACES: [&str; 4] = ["types", "host", "images", "notifications"];

/// Stubs the `fyx:ui` 0.3 interfaces `component` imports that this build does not implement yet,
/// as functions that trap when called, so a component that never calls them loads and runs.
///
/// Only `fyx:ui/*` imports are stubbed. `Linker::define_unknown_imports_as_traps` would also stub
/// WASI imports at a newer patch version than wasmtime-wasi defines (rustc's wasip2 std imports
/// `wasi:random/insecure-seed@0.2.9` for `HashMap`), shadowing the semver-compatible
/// definition, so any guest with a `HashMap` trapped.
pub(crate) fn define_unbuilt_as_traps(
    linker: &mut wasmtime::component::Linker<UiState>,
    component: &wasmtime::component::Component,
    engine: &wasmtime::Engine,
) -> wasmtime::Result<()> {
    use wasmtime::component::{ResourceType, types::ComponentItem};

    /// The host type behind every stubbed resource; no value of it is ever made.
    struct Unbuilt;

    let component_type = component.component_type();
    for (name, import) in component_type.imports(engine) {
        let Some(interface) = name.strip_prefix("fyx:ui/") else {
            continue;
        };
        let interface = interface.split('@').next().unwrap_or(interface);
        if LINKED_INTERFACES.contains(&interface) {
            continue;
        }
        let ComponentItem::ComponentInstance(instance_type) = import.ty else {
            continue;
        };
        let mut instance = linker.instance(name)?;
        for (export, item) in instance_type.exports(engine) {
            match item.ty {
                ComponentItem::ComponentFunc(_) => {
                    let qualified = format!("{name}#{export}");
                    instance.func_new(export, move |_, _, _, _| {
                        wasmtime::bail!("{qualified} is not implemented by this build of Fyx yet")
                    })?;
                }
                ComponentItem::Resource(_) => {
                    instance.resource(export, ResourceType::host::<Unbuilt>(), |_, _| Ok(()))?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// The 0.3 linker: WASI and the interfaces above; the rest trap (see the module doc).
pub(crate) fn add_to_linker(
    linker: &mut wasmtime::component::Linker<UiState>,
) -> wasmtime::Result<()> {
    wit::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_host::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_images::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_notifications::add_to_linker::<_, UiState>(linker, |state| state)?;
    Ok(())
}
