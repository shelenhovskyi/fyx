//! Host side of the `fyx:ui@0.3.0` world (`crates/ui_ext/wit/since_v0.3.0/fyx-ui.wit`), the
//! smaller 0.3 (integration-5 A3): what 0.3 copies forward from 0.2 (`types`, `host`,
//! `notifications`, `decorations`), `images` (design.md §12), canvas nodes (§13), and the T2
//! interfaces `network` (§8), `process` (§9), `workspace-edit` (§10), `agent-observe` (§11) and
//! `permissions` (§7), whose operations start after their call and report through `handle-io`.
//! Every interface of the world is linked (`LINKED_INTERFACES`), so `define_unbuilt_as_traps`
//! stubs nothing for 0.3; 0.4 uses it for the interfaces that land with later plan tasks. The
//! imports 0.3 shares with 0.4 are in `t2_shared.rs`.
//!
//! `image` and `canvas` nodes convert into the model; their checks run when the update or the
//! reply is applied ([`crate::effects::ApiState::check_images`]).

use ui_ext::{
    canvas as model_canvas,
    image::{self as model_image},
    v0_2 as model,
};

use crate::{UiState, conversions, edit, network, observe, permissions::PermissionState, process};

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
    agent_observe as wit_observe, decorations as wit_decorations, host as wit_host,
    images as wit_images, io as wit_io, network as wit_network, notifications as wit_notifications,
    permissions as wit_permissions, process as wit_process, types as wit,
    workspace_edit as wit_edit,
};

conversions::host_imports!(
    denied: wit_host::HostErrorCode::Denied,
    untrusted_worktree: wit_host::HostErrorCode::UntrustedWorktree,
    no_worktree: wit_host::HostErrorCode::NoWorktree
);
conversions::notification_imports!();
conversions::decoration_imports!();

include!("t2_shared.rs");

fn t2_capability(capability: wit_permissions::T2Capability) -> &'static str {
    match capability {
        wit_permissions::T2Capability::NetworkFetch => network::CAPABILITY_NETWORK,
        wit_permissions::T2Capability::ProcessSpawn => process::CAPABILITY_PROCESS,
        wit_permissions::T2Capability::WorkspaceEdit => edit::CAPABILITY_EDIT,
        wit_permissions::T2Capability::AgentObserve => observe::CAPABILITY_OBSERVE,
    }
}

fn t2_from_name(capability: &str) -> Option<wit_permissions::T2Capability> {
    Some(match capability {
        network::CAPABILITY_NETWORK => wit_permissions::T2Capability::NetworkFetch,
        process::CAPABILITY_PROCESS => wit_permissions::T2Capability::ProcessSpawn,
        edit::CAPABILITY_EDIT => wit_permissions::T2Capability::WorkspaceEdit,
        observe::CAPABILITY_OBSERVE => wit_permissions::T2Capability::AgentObserve,
        _ => return None,
    })
}

/// 0.3 has no terminals.
fn terminal_io_event(_: ui_ext::terminal::TerminalEvent) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no listeners.
fn listener_io_event(_: ui_ext::integration::ListenerEvent) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no editor context.
fn context_io_event(_: ui_ext::context::EditorContext) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no diagnostics feed.
fn diagnostics_io_event(_: Vec<ui_ext::process::FileRef>) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no relay.
fn relay_io_event(_: ui_ext::integration::RelayRequest) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no proposed edits.
fn review_io_event(
    _: u64,
    _: Result<ui_ext::review::ReviewOutcome, String>,
) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no proposed edits, so no comments on them.
fn review_comments_io_event(_: ui_ext::review::ReviewComments) -> Option<wit_io::IoEvent> {
    None
}

/// 0.3 has no approval cards.
fn approval_io_event(
    _: u64,
    _: Option<ui_ext::approvals::ApprovalOption>,
) -> Option<wit_io::IoEvent> {
    None
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
        wit::Props::Canvas(props) => Self::Canvas(canvas(props)),
    }
    kinds: {}
    events: {
        model::UiEvent::CanvasClick(click) => Self::CanvasClick(wit::CanvasClick {
            canvas: click.canvas,
            x: click.x.0,
            y: click.y.0,
        }),
    }
}

/// The `fyx:ui` 0.3 interfaces [`add_to_linker`] links.
pub(crate) const LINKED_INTERFACES: &[&str] = &[
    "types",
    "host",
    "decorations",
    "images",
    "notifications",
    "network",
    "process",
    "workspace-edit",
    "agent-observe",
    "permissions",
];

/// Stubs the `fyx:ui` interfaces `component` imports that are not in `linked` (those this build
/// does not implement yet) as functions that trap when called, so a component that never calls
/// them loads and runs.
///
/// Only `fyx:ui/*` imports are stubbed. `Linker::define_unknown_imports_as_traps` would also stub
/// WASI imports at a newer patch version than wasmtime-wasi defines (rustc's wasip2 std imports
/// `wasi:random/insecure-seed@0.2.9` for `HashMap`), shadowing the semver-compatible
/// definition, so any guest with a `HashMap` trapped.
pub(crate) fn define_unbuilt_as_traps(
    linked: &[&str],
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
        if linked.contains(&interface) {
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
    wit_decorations::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_images::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_notifications::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_network::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_process::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_edit::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_observe::add_to_linker::<_, UiState>(linker, |state| state)?;
    wit_permissions::add_to_linker::<_, UiState>(linker, |state| state)?;
    Ok(())
}
