//! Guest SDK for Fyx UI extensions written in Rust and compiled to `wasm32-wasip2`.
//!
//! Apache-2.0 so that extension authors, proprietary ones included, stay outside the GPL.
//!
//! The API is that of `fyx:ui@0.3.0`, which is staged: Dev and Nightly builds of Fyx run it. An
//! extension implements [`Extension`] and exports it with [`register_extension!`]:
//! `register_extension!(T)` exports 0.3, `register_extension!(T, v0_2)` exports 0.2 and
//! `register_extension!(T, v0_1)` exports 0.1, all from the same code. Trees are
//! written as nested Rust code with [`TreeBuilder`], which assigns node ids and parents and emits
//! the flat, parents-first `list<node>` the `fyx:ui` world expects. [`SurfaceTree`] remembers the
//! live tree of one surface so `handle-event` can answer with a few patches instead of a full
//! `replace`:
//!
//! ```ignore
//! use ui_ext_sdk::*;
//!
//! #[derive(Default)]
//! struct Hello {
//!     tree: SurfaceTree,
//!     greeting: Option<NodeId>,
//!     button: Option<NodeId>,
//! }
//!
//! impl Extension for Hello {
//!     fn new() -> Self {
//!         Self::default()
//!     }
//!
//!     fn surfaces(&self) -> Vec<Surface> {
//!         vec![Surface::panel("hello", "Hello", DockPosition::Right, Icon::Sparkle, 240)]
//!     }
//!
//!     fn mount(&mut self, _surface_id: &str) -> Result<Vec<Node>, String> {
//!         Ok(self.tree.mount(|tree| {
//!             tree.column(|tree| {
//!                 self.greeting = Some(tree.text("Hello"));
//!                 self.button = Some(tree.button("Wave"));
//!             });
//!         })?)
//!     }
//!
//!     fn handle_event(&mut self, _surface_id: &str, event: UiEvent) -> Result<Update, String> {
//!         if let (UiEvent::Click(id), Some(greeting)) = (event, self.greeting)
//!             && Some(id) == self.button
//!         {
//!             self.tree.set_props(greeting, TextProps::new("Hello, world").bold())?;
//!         }
//!         Ok(self.tree.take_update())
//!     }
//! }
//!
//! // Every Fyx runs 0.1; `register_extension!(Hello)` exports 0.3.
//! register_extension!(Hello, v0_1);
//! ```

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;

mod host;
mod host_model;
mod v0_1;
mod v0_2;

pub use host::*;

pub mod bindings {
    wit_bindgen::generate!({
        path: "../ui_ext/wit/since_v0.3.0",
        world: "ui-extension",
        pub_export_macro: true,
        default_bindings_module: "ui_ext_sdk::bindings",
        // Not `Eq`: canvas coordinates are `f32`.
        additional_derives: [PartialEq],
        // Without this, every interface module holds a `#[used]` reference to the world's type
        // section, so a 0.1 or 0.2 export links it whenever that module shares a codegen unit
        // with SDK code it uses, and imports 0.3's type-only interfaces. The world's own
        // `export!` describes it for a 0.3 export.
        disable_custom_section_link_helpers: true,
    });
}

/// The `fyx:ui@0.1.0` bindings, for `register_extension!(T, v0_1)`.
#[doc(hidden)]
pub use ui_ext_sdk_v0_1::bindings as bindings_v0_1;
/// The `fyx:ui@0.2.0` bindings, for `register_extension!(T, v0_2)`.
#[doc(hidden)]
pub use ui_ext_sdk_v0_2::bindings as bindings_v0_2;

pub use bindings::fyx::ui::host::{HostError, HostErrorCode, LogLevel};
pub use bindings::fyx::ui::images::ImageFormat;
pub use bindings::fyx::ui::types::*;
/// The host's caps on a surface, which [`SurfaceTree`] enforces before the host sees an update.
pub use ui_ext::TreeLimits;

/// The `fyx:ui` WIT version of the SDK's API and of `register_extension!(T)`, as (major, minor,
/// patch). 0.3 is staged.
pub const API_VERSION: (u16, u16, u16) = (0, 3, 0);

/// The version `register_extension!(T, v0_2)` exports.
pub const API_VERSION_V0_2: (u16, u16, u16) = (0, 2, 0);

/// The version `register_extension!(T, v0_1)` exports, for hosts that only run 0.1.
pub const API_VERSION_V0_1: (u16, u16, u16) = (0, 1, 0);

/// Name of the custom section [`register_extension!`] adds to every component, from which the host
/// reads the version before instantiating it.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";

const fn version_section_bytes(version: (u16, u16, u16)) -> [u8; 6] {
    let (major, minor, patch) = version;
    let [major_high, major_low] = major.to_be_bytes();
    let [minor_high, minor_low] = minor.to_be_bytes();
    let [patch_high, patch_low] = patch.to_be_bytes();
    [
        major_high, major_low, minor_high, minor_low, patch_high, patch_low,
    ]
}

/// [`API_VERSION`] as stored in the section: three big-endian `u16`s, as in Zed's
/// `zed:api-version`.
pub const API_VERSION_SECTION_BYTES: [u8; 6] = version_section_bytes(API_VERSION);

/// [`API_VERSION_V0_2`] as stored in the section.
pub const API_VERSION_V0_2_SECTION_BYTES: [u8; 6] = version_section_bytes(API_VERSION_V0_2);

/// [`API_VERSION_V0_1`] as stored in the section.
pub const API_VERSION_V0_1_SECTION_BYTES: [u8; 6] = version_section_bytes(API_VERSION_V0_1);

/// First id handed out inside a [`ListRow`] subtree.
///
/// Rows are addressed by `(list node, index)` and rebuilt on every scroll, so their node ids are
/// only unique within one row. They start at this value so they can never be mistaken for a node
/// of the surface's main tree, whose ids [`IdAllocator`] hands out from 1 upwards.
pub const ROW_NODE_ID_BASE: NodeId = 0x8000_0000;

/// A UI extension. One instance lives for the lifetime of the WASM instance; the host calls it
/// from a single thread, one call at a time.
pub trait Extension: 'static {
    fn new() -> Self
    where
        Self: Sized;

    /// The static surfaces, called once after instantiation. [`add_surface`] adds more later.
    fn surfaces(&self) -> Vec<Surface>;

    /// The whole tree of `surface_id`, parents first. [`SurfaceTree::mount`] builds one.
    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String>;

    fn handle_event(&mut self, _surface_id: &str, _event: UiEvent) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }

    /// Rows `start..start + count` of the virtualized list `list_node`. [`list_rows`] clamps the
    /// range to the list's length and builds each row.
    fn list_rows(
        &mut self,
        _surface_id: &str,
        _list_node: NodeId,
        _start: u32,
        _count: u32,
    ) -> Vec<ListRow> {
        Vec::new()
    }

    /// The tick [`request_tick`] asked for. The update applies to `surface_id`, like an event's
    /// reply. `fyx:ui` 0.1 has no ticks, so a `v0_1` extension never gets this call.
    fn tick(&mut self, _surface_id: &str) -> Result<Update, String> {
        Ok(Update::Unchanged)
    }
}

/// Exports a type implementing [`Extension`]. Call it once, at the root of the extension's
/// `cdylib` crate.
///
/// - `register_extension!(MyExtension)` exports the `fyx:ui@0.2.0` world.
/// - `register_extension!(MyExtension, v0_1)` exports `fyx:ui@0.1.0` from the same code, for
///   hosts that only run 0.1. Values are converted at the boundary: icons become upstream names,
///   tooltips, selections and row indents are dropped, and what 0.1 cannot express at all (modal
///   surfaces, `invalidate-rows`, secret inputs, ticks, runtime surfaces, decorations) fails
///   with an error that says it needs 0.2.
///
/// The component then imports only the chosen world's host functions: the SDK calls them through
/// `extern "Rust"` shims that this macro defines for that world.
#[macro_export]
macro_rules! register_extension {
    ($extension_type:ty) => {
        struct __FyxUiExtension;

        fn __fyx_new_extension() -> ::std::boxed::Box<dyn $crate::Extension> {
            ::std::boxed::Box::new(<$extension_type as $crate::Extension>::new())
        }

        impl $crate::bindings::Guest for __FyxUiExtension {
            fn surfaces() -> ::std::vec::Vec<$crate::Surface> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.surfaces()
                })
            }

            fn mount(
                surface_id: ::std::string::String,
            ) -> ::std::result::Result<::std::vec::Vec<$crate::Node>, ::std::string::String> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.mount(&surface_id)
                })
            }

            fn handle_event(
                surface_id: ::std::string::String,
                event: $crate::UiEvent,
            ) -> ::std::result::Result<$crate::Update, ::std::string::String> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.handle_event(&surface_id, event)
                })
            }

            fn list_rows(
                surface_id: ::std::string::String,
                list_node: $crate::NodeId,
                start: u32,
                count: u32,
            ) -> ::std::vec::Vec<$crate::ListRow> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.list_rows(&surface_id, list_node, start, count)
                })
            }

            fn tick(
                surface_id: ::std::string::String,
            ) -> ::std::result::Result<$crate::Update, ::std::string::String> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension.tick(&surface_id)
                })
            }

            // The rest of the 0.3 world. The SDK has no API for these yet (plan 4.2 to 4.8), so
            // they answer as an extension that provides nothing; the host only calls them for
            // what the manifest declares.
            fn handle_io(
                _event: $crate::bindings::fyx::ui::io::IoEvent,
            ) -> ::std::result::Result<(), ::std::string::String> {
                Ok(())
            }

            fn handle_command(
                command: ::std::string::String,
                _argument: ::std::option::Option<::std::string::String>,
                _context: $crate::bindings::fyx::ui::requests::CommandContext,
            ) -> ::std::result::Result<(), ::std::string::String> {
                Err(::std::format!("no command {command:?}"))
            }

            fn provide_decorations(
                _request: $crate::bindings::fyx::ui::requests::DecorationRequest,
            ) -> ::std::result::Result<
                ::std::vec::Vec<$crate::bindings::fyx::ui::decorations::Decoration>,
                ::std::string::String,
            > {
                Ok(::std::vec::Vec::new())
            }

            fn provide_hover(
                _request: $crate::bindings::fyx::ui::requests::HoverRequest,
            ) -> ::std::result::Result<
                $crate::bindings::fyx::ui::requests::HoverReply,
                ::std::string::String,
            > {
                Err("no hover provider".into())
            }

            fn provide_code_lenses(
                _request: $crate::bindings::fyx::ui::requests::CodeLensRequest,
            ) -> ::std::result::Result<
                ::std::vec::Vec<$crate::bindings::fyx::ui::requests::CodeLens>,
                ::std::string::String,
            > {
                Ok(::std::vec::Vec::new())
            }

            fn provide_virtual_document(
                path: ::std::string::String,
            ) -> ::std::result::Result<
                $crate::bindings::fyx::ui::requests::VirtualDocument,
                ::std::string::String,
            > {
                Err(::std::format!("no virtual document {path:?}"))
            }

            fn picker_source_event(
                _source: ::std::string::String,
                _session: u64,
                _event: $crate::bindings::fyx::ui::requests::PickerSourceEvent,
            ) -> ::std::result::Result<(), ::std::string::String> {
                Ok(())
            }
        }

        $crate::__fyx_host_shims!(v0_3);

        // In the extension's own crate rather than the SDK's rlib, so the linker cannot drop it
        // as an unreferenced archive member.
        #[cfg(target_arch = "wasm32")]
        #[unsafe(link_section = "fyx:ui-api-version")]
        #[used]
        static __FYX_UI_API_VERSION: [u8; 6] = $crate::API_VERSION_SECTION_BYTES;

        $crate::bindings::export!(__FyxUiExtension with_types_in $crate::bindings);
    };
    ($extension_type:ty, v0_2) => {
        struct __FyxUiExtension;

        fn __fyx_new_extension() -> ::std::boxed::Box<dyn $crate::Extension> {
            ::std::boxed::Box::new(<$extension_type as $crate::Extension>::new())
        }

        impl $crate::bindings_v0_2::Guest for __FyxUiExtension {
            fn surfaces() -> ::std::vec::Vec<$crate::bindings_v0_2::fyx::ui::types::Surface> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    $crate::__private::v0_2::surfaces(extension.surfaces())
                })
            }

            fn mount(
                surface_id: ::std::string::String,
            ) -> ::std::result::Result<
                ::std::vec::Vec<$crate::bindings_v0_2::fyx::ui::types::Node>,
                ::std::string::String,
            > {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension
                        .mount(&surface_id)
                        .and_then(|nodes| $crate::__private::v0_2::nodes(&nodes))
                })
            }

            fn handle_event(
                surface_id: ::std::string::String,
                event: $crate::bindings_v0_2::fyx::ui::types::UiEvent,
            ) -> ::std::result::Result<
                $crate::bindings_v0_2::fyx::ui::types::Update,
                ::std::string::String,
            > {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension
                        .handle_event(&surface_id, $crate::__private::v0_2::event(event))
                        .and_then(|update| $crate::__private::v0_2::update(&update))
                })
            }

            fn list_rows(
                surface_id: ::std::string::String,
                list_node: $crate::NodeId,
                start: u32,
                count: u32,
            ) -> ::std::vec::Vec<$crate::bindings_v0_2::fyx::ui::types::ListRow> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    $crate::__private::v0_2::rows(extension.list_rows(
                        &surface_id,
                        list_node,
                        start,
                        count,
                    ))
                })
            }

            fn tick(
                surface_id: ::std::string::String,
            ) -> ::std::result::Result<
                $crate::bindings_v0_2::fyx::ui::types::Update,
                ::std::string::String,
            > {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension
                        .tick(&surface_id)
                        .and_then(|update| $crate::__private::v0_2::update(&update))
                })
            }
        }

        $crate::__fyx_host_shims!(v0_2);

        #[cfg(target_arch = "wasm32")]
        #[unsafe(link_section = "fyx:ui-api-version")]
        #[used]
        static __FYX_UI_API_VERSION: [u8; 6] = $crate::API_VERSION_V0_2_SECTION_BYTES;

        $crate::bindings_v0_2::export!(__FyxUiExtension with_types_in $crate::bindings_v0_2);
    };
    ($extension_type:ty, v0_1) => {
        struct __FyxUiExtension;

        fn __fyx_new_extension() -> ::std::boxed::Box<dyn $crate::Extension> {
            ::std::boxed::Box::new(<$extension_type as $crate::Extension>::new())
        }

        impl $crate::bindings_v0_1::Guest for __FyxUiExtension {
            fn surfaces() -> ::std::vec::Vec<$crate::bindings_v0_1::fyx::ui::types::Surface> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    $crate::__private::v0_1::surfaces(extension.surfaces())
                })
            }

            fn mount(
                surface_id: ::std::string::String,
            ) -> ::std::result::Result<
                ::std::vec::Vec<$crate::bindings_v0_1::fyx::ui::types::Node>,
                ::std::string::String,
            > {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension
                        .mount(&surface_id)
                        .and_then(|nodes| $crate::__private::v0_1::nodes(&nodes))
                })
            }

            fn handle_event(
                surface_id: ::std::string::String,
                event: $crate::bindings_v0_1::fyx::ui::types::UiEvent,
            ) -> ::std::result::Result<
                $crate::bindings_v0_1::fyx::ui::types::Update,
                ::std::string::String,
            > {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    extension
                        .handle_event(&surface_id, $crate::__private::v0_1::event(event))
                        .and_then(|update| $crate::__private::v0_1::update(&update))
                })
            }

            fn list_rows(
                surface_id: ::std::string::String,
                list_node: $crate::NodeId,
                start: u32,
                count: u32,
            ) -> ::std::vec::Vec<$crate::bindings_v0_1::fyx::ui::types::ListRow> {
                $crate::__private::with_extension(__fyx_new_extension, |extension| {
                    $crate::__private::v0_1::rows(extension.list_rows(
                        &surface_id,
                        list_node,
                        start,
                        count,
                    ))
                })
            }
        }

        $crate::__fyx_host_shims!(v0_1);

        #[cfg(target_arch = "wasm32")]
        #[unsafe(link_section = "fyx:ui-api-version")]
        #[used]
        static __FYX_UI_API_VERSION: [u8; 6] = $crate::API_VERSION_V0_1_SECTION_BYTES;

        $crate::bindings_v0_1::export!(__FyxUiExtension with_types_in $crate::bindings_v0_1);
    };
}

/// Defines the `extern "Rust"` functions through which the SDK's host functions reach the chosen
/// world's imports (see `host.rs`). Only the functions an extension calls end up imported.
#[doc(hidden)]
#[macro_export]
macro_rules! __fyx_host_shims {
    ($world:ident) => {
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            #[unsafe(no_mangle)]
            fn __fyx_ui_exported_api_version() -> (u16, u16, u16) {
                $crate::__private::$world::API_VERSION
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_log(level: $crate::LogLevel, message: &str) {
                $crate::__private::$world::log(level, message)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_push_update(
                surface_id: &str,
                update: &$crate::Update,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::push_update(surface_id, update)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_request_tick(
                surface_id: &str,
                delay_ms: u32,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::request_tick(surface_id, delay_ms)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_cancel_tick(surface_id: &str) {
                $crate::__private::$world::cancel_tick(surface_id)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_add_surface(
                surface: &$crate::Surface,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::add_surface(surface)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_remove_surface(
                surface_id: &str,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::remove_surface(surface_id)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_show_surface(
                surface_id: &str,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::show_surface(surface_id)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_hide_surface(
                surface_id: &str,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::hide_surface(surface_id)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_place_decoration(
                id: &str,
                location: &$crate::DecorationLocation,
                nodes: &[$crate::Node],
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::place_decoration(id, location, nodes)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_remove_decoration(
                id: &str,
            ) -> ::std::result::Result<(), $crate::HostError> {
                $crate::__private::$world::remove_decoration(id)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_add_image(
                format: $crate::ImageFormat,
                bytes: &[u8],
            ) -> ::std::result::Result<u32, $crate::HostError> {
                $crate::__private::$world::add_image(format, bytes)
            }

            #[unsafe(no_mangle)]
            fn __fyx_ui_remove_image(image: u32) {
                $crate::__private::$world::remove_image(image)
            }
        };
    };
}

#[doc(hidden)]
pub mod __private {
    use super::*;

    pub use crate::host::v0_3;
    pub use crate::v0_1::glue as v0_1;
    pub use crate::v0_2::glue as v0_2;

    thread_local! {
        static EXTENSION: RefCell<Option<Box<dyn Extension>>> = const { RefCell::new(None) };
    }

    /// The export functions are free functions, so the instance lives in a thread local that is
    /// filled on the first call. WASM components are single-threaded and the host never calls
    /// back into the extension while a call is running, so the borrow cannot be contended.
    pub fn with_extension<R>(
        new_extension: fn() -> Box<dyn Extension>,
        call: impl FnOnce(&mut dyn Extension) -> R,
    ) -> R {
        EXTENSION
            .with_borrow_mut(|extension| call(extension.get_or_insert_with(new_extension).as_mut()))
    }
}

/// Hands out the node ids of one surface. Ids only ever increase, so an id is never reused for a
/// different node while the surface lives, as the `fyx:ui` contract requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdAllocator {
    next: NodeId,
}

impl IdAllocator {
    pub const fn new() -> Self {
        Self::starting_at(1)
    }

    pub const fn starting_at(first: NodeId) -> Self {
        Self { next: first }
    }

    pub fn allocate(&mut self) -> NodeId {
        let id = self.next;
        self.next += 1;
        id
    }

    /// The id the next call to [`IdAllocator::allocate`] returns.
    pub fn peek(&self) -> NodeId {
        self.next
    }
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds a flat node list from nested code. Flex nodes take a closure that builds their
/// children; every node gets a fresh id and its parent's id, and parents precede children.
pub struct TreeBuilder<'a> {
    ids: &'a mut IdAllocator,
    nodes: Vec<Node>,
    parent: Option<NodeId>,
}

impl<'a> TreeBuilder<'a> {
    /// A builder whose top-level nodes have no parent.
    pub fn new(ids: &'a mut IdAllocator) -> Self {
        Self {
            ids,
            nodes: Vec::new(),
            parent: None,
        }
    }

    /// A builder whose top-level nodes are children of `parent`, for inserting into a live tree.
    pub fn under(ids: &'a mut IdAllocator, parent: NodeId) -> Self {
        Self {
            ids,
            nodes: Vec::new(),
            parent: Some(parent),
        }
    }

    /// Adds a node without children and returns its id.
    pub fn node(&mut self, props: impl Into<Props>) -> NodeId {
        let id = self.ids.allocate();
        self.nodes.push(Node {
            id,
            parent: self.parent,
            props: props.into(),
        });
        id
    }

    /// Adds a flex node and the children `build_children` adds, and returns the node's id. Only
    /// flex nodes have children; the host rejects children under any other kind.
    pub fn flex(&mut self, props: FlexProps, build_children: impl FnOnce(&mut Self)) -> NodeId {
        let id = self.node(props);
        let outer_parent = self.parent.replace(id);
        build_children(self);
        self.parent = outer_parent;
        id
    }

    pub fn column(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.flex(FlexProps::column(), build_children)
    }

    pub fn row(&mut self, build_children: impl FnOnce(&mut Self)) -> NodeId {
        self.flex(FlexProps::row(), build_children)
    }

    pub fn text(&mut self, props: impl Into<TextProps>) -> NodeId {
        self.node(props.into())
    }

    pub fn button(&mut self, props: impl Into<ButtonProps>) -> NodeId {
        self.node(props.into())
    }

    pub fn input(&mut self, props: InputProps) -> NodeId {
        self.node(props)
    }

    /// A virtualized list; the host pulls its rows with `list-rows`.
    pub fn list(&mut self, props: ListProps) -> NodeId {
        self.node(props)
    }

    pub fn table(&mut self, props: TableProps) -> NodeId {
        self.node(props)
    }

    pub fn icon(&mut self, props: impl Into<IconProps>) -> NodeId {
        self.node(props.into())
    }

    pub fn markdown(&mut self, source: impl Into<String>) -> NodeId {
        self.node(Props::Markdown(source.into()))
    }

    pub fn diff_view(&mut self, props: DiffViewProps) -> NodeId {
        self.node(props)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn finish(self) -> Vec<Node> {
        self.nodes
    }
}

/// Builds one row of a virtualized list; its ids start at [`ROW_NODE_ID_BASE`]. `build` must add
/// exactly one top-level node, the row's root, and the row must pass the host's own checks
/// (`ui_ext::Tree::from_row`), or the host would reject the whole `list-rows` answer.
pub fn list_row(index: u32, build: impl FnOnce(&mut TreeBuilder)) -> Result<ListRow, TreeError> {
    let mut ids = IdAllocator::starting_at(ROW_NODE_ID_BASE);
    let mut tree = TreeBuilder::new(&mut ids);
    build(&mut tree);
    let row = ListRow {
        index,
        indent: 0,
        nodes: tree.finish(),
    };
    ui_ext::Tree::from_row(
        row.nodes.iter().map(Into::into).collect(),
        TreeLimits::default(),
    )
    .map_err(TreeError::InvalidRow)?;
    Ok(row)
}

/// The row indices of a `list-rows(start, count)` request that exist in a list of `row_count`
/// rows. The host may ask past the end while the list shrinks.
pub fn row_range(start: u32, count: u32, row_count: u32) -> Range<u32> {
    let end = start.saturating_add(count).min(row_count);
    start.min(end)..end
}

/// Answers a `list-rows(start, count)` request for a list of `row_count` rows by calling
/// `build_row` for each row that exists. See [`list_row`]; a row the host would reject is logged
/// and left out, so the other rows still show.
pub fn list_rows(
    start: u32,
    count: u32,
    row_count: u32,
    mut build_row: impl FnMut(u32, &mut TreeBuilder),
) -> Vec<ListRow> {
    row_range(start, count, row_count)
        .filter_map(
            |index| match list_row(index, |tree| build_row(index, tree)) {
                Ok(row) => Some(row),
                Err(error) => {
                    log(
                        LogLevel::Error,
                        &format!("list row {index} left out: {error}"),
                    );
                    None
                }
            },
        )
        .collect()
}

/// An ordered batch of patches, turned into an `update` with [`PatchSet::into_update`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PatchSet {
    patches: Vec<Patch>,
}

impl PatchSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_props(&mut self, id: NodeId, props: impl Into<Props>) -> &mut Self {
        self.patches.push(Patch::SetProps((id, props.into())));
        self
    }

    /// Inserts one node under `node.parent`, before the sibling `before` or at the end.
    pub fn insert(&mut self, node: Node, before: Option<NodeId>) -> &mut Self {
        self.patches.push(Patch::Insert(InsertOp { node, before }));
        self
    }

    /// Inserts a subtree in parents-first order, such as one built with [`TreeBuilder::under`].
    /// Its top-level nodes (those whose parent is outside `nodes`) go before `before`, keeping
    /// their order; deeper nodes are appended to their new parents.
    pub fn insert_subtree(&mut self, nodes: Vec<Node>, before: Option<NodeId>) -> &mut Self {
        let subtree_ids = nodes.iter().map(|node| node.id).collect::<HashSet<_>>();
        for node in nodes {
            let is_top_level = node
                .parent
                .is_none_or(|parent| !subtree_ids.contains(&parent));
            let before = if is_top_level { before } else { None };
            self.insert(node, before);
        }
        self
    }

    /// Removes the node and its whole subtree.
    pub fn remove(&mut self, id: NodeId) -> &mut Self {
        self.patches.push(Patch::Remove(id));
        self
    }

    /// The rows of a list or table changed although its props did not: the host fetches them
    /// again, `rows` only, or all of them for `None`.
    pub fn invalidate_rows(&mut self, target: NodeId, rows: Option<RowRange>) -> &mut Self {
        self.patches
            .push(Patch::InvalidateRows(InvalidateRowsOp { target, rows }));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn len(&self) -> usize {
        self.patches.len()
    }

    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }

    /// `unchanged` when there is nothing to send, `patches` otherwise.
    pub fn into_update(self) -> Update {
        if self.patches.is_empty() {
            Update::Unchanged
        } else {
            Update::Patches(self.patches)
        }
    }
}

impl From<PatchSet> for Update {
    fn from(patches: PatchSet) -> Self {
        patches.into_update()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    NotMounted,
    UnknownNode(NodeId),
    NotAChild {
        parent: NodeId,
        child: NodeId,
    },
    IsRoot(NodeId),
    /// Only flex nodes have children: inserting under `node`, or giving it props of `kind` while
    /// it has children, would be rejected by the host.
    NotAContainer {
        node: NodeId,
        kind: &'static str,
    },
    UpdateTooLarge {
        count: usize,
        max: usize,
    },
    TreeTooLarge {
        count: usize,
        max: usize,
    },
    /// A list row the host would reject, reported by `ui_ext::Tree::from_row`.
    InvalidRow(ui_ext::TreeError),
    /// `invalidate-rows` names a node that is not a list or a table.
    NotAList {
        node: NodeId,
        kind: &'static str,
    },
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotMounted => write!(formatter, "surface is not mounted"),
            Self::UnknownNode(id) => write!(formatter, "node {id} is not in the surface's tree"),
            Self::NotAChild { parent, child } => {
                write!(formatter, "node {child} is not a child of node {parent}")
            }
            Self::IsRoot(id) => write!(formatter, "node {id} is a root of the surface's tree"),
            Self::NotAContainer { node, kind } => write!(
                formatter,
                "node {node} would be a {kind} with children; only flex nodes have children"
            ),
            Self::UpdateTooLarge { count, max } => write!(
                formatter,
                "update would carry {count} nodes or patches, the cap is {max}"
            ),
            Self::TreeTooLarge { count, max } => {
                write!(formatter, "tree would have {count} nodes, the cap is {max}")
            }
            Self::InvalidRow(error) => write!(formatter, "invalid list row: {error}"),
            Self::NotAList { node, kind } => write!(
                formatter,
                "node {node} is a {kind}; only a list or a table has rows to invalidate"
            ),
        }
    }
}

impl std::error::Error for TreeError {}

impl From<TreeError> for String {
    fn from(error: TreeError) -> Self {
        error.to_string()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LiveNode {
    parent: Option<NodeId>,
    props: Props,
    children: Vec<NodeId>,
}

/// The extension-side copy of one surface's tree, mirroring what the host has applied.
///
/// Edits are recorded and queued as patches; [`SurfaceTree::take_update`] hands the queue to the
/// host. A `set_props` that does not change anything is dropped, so re-setting every node after a
/// state change sends only what actually changed.
///
/// The host applies an update atomically, rejects the whole batch if one patch breaks its rules,
/// and does not tell the extension. So every edit is checked against those rules first: children
/// only under flex nodes, at most [`TreeLimits::max_nodes_per_update`] queued patches and at most
/// [`TreeLimits::max_nodes`] nodes. An edit that breaks one returns a [`TreeError`] and leaves the
/// mirror and the queue as they were (ids it allocated stay unused), so the queue is always a
/// batch the host accepts.
#[derive(Debug, Default)]
pub struct SurfaceTree {
    ids: IdAllocator,
    limits: TreeLimits,
    mounted: bool,
    nodes: HashMap<NodeId, LiveNode>,
    roots: Vec<NodeId>,
    pending: PatchSet,
    /// A push failed, so the host is behind the mirror: the next update is a whole `replace`.
    unsynced: bool,
}

impl SurfaceTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// A tree checked against `limits` instead of the host's defaults.
    pub fn with_limits(limits: TreeLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    /// Builds the surface's tree from scratch and returns it for `mount`. Pending patches are
    /// dropped. Ids keep counting up from earlier trees, so a node id from a previous mount never
    /// names a node of the new tree. Fails, keeping the previous tree, if the new one has more
    /// nodes than a tree may hold (`max_nodes`, 16384), the host's cap for a mount and a
    /// `replace`. So a tree grown past 2048 nodes with `insert` can always be mounted again.
    pub fn mount(&mut self, build: impl FnOnce(&mut TreeBuilder)) -> Result<Vec<Node>, TreeError> {
        let mut tree = TreeBuilder::new(&mut self.ids);
        build(&mut tree);
        let nodes = tree.finish();
        self.check_tree_size(nodes.len())?;

        self.nodes.clear();
        self.roots.clear();
        self.pending = PatchSet::new();
        self.mounted = true;
        self.record(&nodes, None);
        Ok(nodes)
    }

    /// Like [`SurfaceTree::mount`], but returns the tree as a `replace` update, for when patching
    /// would be more work than rebuilding.
    pub fn replace(&mut self, build: impl FnOnce(&mut TreeBuilder)) -> Result<Update, TreeError> {
        Ok(Update::Replace(self.mount(build)?))
    }

    pub fn is_mounted(&self) -> bool {
        self.mounted
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn props(&self, id: NodeId) -> Option<&Props> {
        self.nodes.get(&id).map(|node| &node.props)
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id).and_then(|node| node.parent)
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes
            .get(&id)
            .map_or(&[], |node| node.children.as_slice())
    }

    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The whole live tree as a flat, parents-first node list, as the host holds it.
    pub fn to_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut stack = self.roots.iter().rev().copied().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if let Some(node) = self.nodes.get(&id) {
                nodes.push(Node {
                    id,
                    parent: node.parent,
                    props: node.props.clone(),
                });
                stack.extend(node.children.iter().rev().copied());
            }
        }
        nodes
    }

    /// Replaces a node's props. Returns whether they changed; unchanged props queue no patch.
    pub fn set_props(&mut self, id: NodeId, props: impl Into<Props>) -> Result<bool, TreeError> {
        let props = props.into();
        let node = self.node(id)?;
        if node.props == props {
            return Ok(false);
        }
        Self::check_container(id, node, &props)?;
        self.check_update_size(self.pending.len() + 1)?;

        self.node_mut(id)?.props = props.clone();
        self.pending.set_props(id, props);
        Ok(true)
    }

    /// Applies to the mirror what the host does to its own tree when it sends `event`. On
    /// `input-changed` and `input-submitted` the host writes the typed value into the input node
    /// (keeping its placeholder) without a patch, because the text is the user's; other events
    /// change nothing. Call it with every event of this surface before handling it. Then the
    /// mirror holds what the host holds: echoing the typed value back queues nothing (it would
    /// fight the user's typing), and clearing the field afterwards does queue the `set-props`,
    /// which a stale mirror would have dropped as unchanged.
    ///
    /// While a `set-props` or `insert` for the node is still queued, the mirror is left alone:
    /// the host applies the queued patch after the event, so the queued props are what it will
    /// hold.
    pub fn apply_event(&mut self, event: &UiEvent) {
        let target = match event {
            UiEvent::InputChanged((id, _))
            | UiEvent::InputSubmitted((id, _))
            | UiEvent::RowSelected((id, _)) => *id,
            UiEvent::Click(_)
            | UiEvent::RowClick(_)
            | UiEvent::Dismissed(_)
            | UiEvent::Orphaned
            | UiEvent::CanvasClick(_) => return,
        };
        if self.pending.patches().iter().any(|patch| match patch {
            Patch::SetProps((id, _)) => *id == target,
            Patch::Insert(op) => op.node.id == target,
            Patch::Remove(_) | Patch::InvalidateRows(_) => false,
        }) {
            return;
        }
        match event {
            UiEvent::InputChanged((id, value)) | UiEvent::InputSubmitted((id, value)) => {
                if let Some(LiveNode {
                    props: Props::Input(input),
                    ..
                }) = self.nodes.get_mut(id)
                {
                    input.value.clone_from(value);
                }
            }
            // The host moves a modal list's selection itself, like typed text.
            UiEvent::RowSelected((id, index)) => {
                if let Some(LiveNode {
                    props: Props::List(list),
                    ..
                }) = self.nodes.get_mut(id)
                {
                    list.selected = Some(*index);
                }
            }
            UiEvent::Click(_)
            | UiEvent::RowClick(_)
            | UiEvent::Dismissed(_)
            | UiEvent::Orphaned
            | UiEvent::CanvasClick(_) => {}
        }
    }

    /// Records props the host already shows without queuing a patch. [`SurfaceTree::apply_event`]
    /// covers typed input text; this is for other cases where the host and the extension agree on
    /// a change the host made itself.
    pub fn set_props_from_host(
        &mut self,
        id: NodeId,
        props: impl Into<Props>,
    ) -> Result<(), TreeError> {
        let props = props.into();
        Self::check_container(id, self.node(id)?, &props)?;
        self.node_mut(id)?.props = props;
        Ok(())
    }

    /// Builds a subtree under the flex node `parent`, before its child `before` or after its last
    /// child, and returns the ids of the subtree's top-level nodes.
    pub fn insert(
        &mut self,
        parent: NodeId,
        before: Option<NodeId>,
        build: impl FnOnce(&mut TreeBuilder),
    ) -> Result<Vec<NodeId>, TreeError> {
        let position = self.insert_position(parent, before)?;
        let mut tree = TreeBuilder::under(&mut self.ids, parent);
        build(&mut tree);
        let nodes = tree.finish();
        self.check_update_size(self.pending.len() + nodes.len())?;
        self.check_tree_size(self.nodes.len() + nodes.len())?;
        Ok(self.commit_insert(parent, position, before, nodes))
    }

    /// Removes a node and its whole subtree.
    pub fn remove(&mut self, id: NodeId) -> Result<(), TreeError> {
        self.node(id)?;
        self.check_update_size(self.pending.len() + 1)?;
        self.commit_remove(id);
        Ok(())
    }

    /// Replaces node `id` and its subtree with the nodes `build` adds, at the same position, and
    /// returns their ids. The new nodes get new ids, so the host treats them as new: a list
    /// rebuilt this way has all of its rows requested again, which `set_props` cannot force when
    /// the rows changed but the list's props did not. Checked as a whole: on error nothing is
    /// removed.
    pub fn rebuild(
        &mut self,
        id: NodeId,
        build: impl FnOnce(&mut TreeBuilder),
    ) -> Result<Vec<NodeId>, TreeError> {
        let parent = self.node(id)?.parent.ok_or(TreeError::IsRoot(id))?;
        let siblings = self.children(parent);
        let position = siblings
            .iter()
            .position(|&sibling| sibling == id)
            .ok_or(TreeError::NotAChild { parent, child: id })?;
        let next_sibling = siblings.get(position + 1).copied();
        let removed = self.subtree_len(id);

        let mut tree = TreeBuilder::under(&mut self.ids, parent);
        build(&mut tree);
        let nodes = tree.finish();
        self.check_update_size(self.pending.len() + 1 + nodes.len())?;
        self.check_tree_size(self.nodes.len() - removed + nodes.len())?;

        self.commit_remove(id);
        Ok(self.commit_insert(parent, position, next_sibling, nodes))
    }

    /// Removes every child of `parent`.
    pub fn clear_children(&mut self, parent: NodeId) -> Result<(), TreeError> {
        let children = self.node(parent)?.children.clone();
        self.check_update_size(self.pending.len() + children.len())?;
        for child in children {
            self.commit_remove(child);
        }
        Ok(())
    }

    /// Patches queued since the last call, as an update for `handle-event` or [`push_update`].
    /// After a failed [`SurfaceTree::push`] it is the whole tree as a `replace` instead.
    pub fn take_update(&mut self) -> Update {
        let pending = std::mem::take(&mut self.pending);
        if std::mem::take(&mut self.unsynced) {
            return Update::Replace(self.to_nodes());
        }
        pending.into_update()
    }

    /// Pushes the queued update to `surface_id` with [`push_update`]. On `Err` the host did not
    /// take it, so the mirror is ahead of the host: the next [`SurfaceTree::take_update`] (or
    /// push) sends the whole tree as a `replace`.
    pub fn push(&mut self, surface_id: &str) -> Result<(), HostError> {
        let update = self.take_update();
        if update == Update::Unchanged {
            return Ok(());
        }
        push_update(surface_id, &update).inspect_err(|_| self.unsynced = true)
    }

    /// Whether a failed push left the host behind the mirror.
    pub fn is_unsynced(&self) -> bool {
        self.unsynced
    }

    /// Tells the host that the rows of the list or table `target` changed although its props did
    /// not; it fetches `rows` again, or all of them for `None`. Unlike rebuilding the list, this
    /// keeps its scroll position. In an extension exported as 0.1 it queues an unchanged
    /// `set-props` of the list instead, after which a 0.1 host fetches all its rows again.
    pub fn invalidate_rows(
        &mut self,
        target: NodeId,
        rows: Option<RowRange>,
    ) -> Result<(), TreeError> {
        let node = self.node(target)?;
        if !matches!(node.props, Props::List(_) | Props::Table(_)) {
            return Err(TreeError::NotAList {
                node: target,
                kind: ui_ext::Props::from(&node.props).kind_name(),
            });
        }
        self.check_update_size(self.pending.len() + 1)?;
        if exported_api_version() == API_VERSION_V0_1 {
            // 0.1 has no invalidate-rows, but a 0.1 host fetches every row of a list a patch
            // touches again, so an unchanged set-props does the same job.
            let props = node.props.clone();
            self.pending.set_props(target, props);
        } else {
            self.pending.invalidate_rows(target, rows);
        }
        Ok(())
    }

    pub fn pending(&self) -> &PatchSet {
        &self.pending
    }

    fn node(&self, id: NodeId) -> Result<&LiveNode, TreeError> {
        if !self.mounted {
            return Err(TreeError::NotMounted);
        }
        self.nodes.get(&id).ok_or(TreeError::UnknownNode(id))
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut LiveNode, TreeError> {
        if !self.mounted {
            return Err(TreeError::NotMounted);
        }
        self.nodes.get_mut(&id).ok_or(TreeError::UnknownNode(id))
    }

    fn check_container(id: NodeId, node: &LiveNode, props: &Props) -> Result<(), TreeError> {
        if node.children.is_empty() {
            return Ok(());
        }
        let host_props = ui_ext::Props::from(props);
        if host_props.is_container() {
            Ok(())
        } else {
            Err(TreeError::NotAContainer {
                node: id,
                kind: host_props.kind_name(),
            })
        }
    }

    fn check_update_size(&self, count: usize) -> Result<(), TreeError> {
        let max = self.limits.max_nodes_per_update;
        if count > max {
            return Err(TreeError::UpdateTooLarge { count, max });
        }
        Ok(())
    }

    fn check_tree_size(&self, count: usize) -> Result<(), TreeError> {
        let max = self.limits.max_nodes;
        if count > max {
            return Err(TreeError::TreeTooLarge { count, max });
        }
        Ok(())
    }

    fn insert_position(&self, parent: NodeId, before: Option<NodeId>) -> Result<usize, TreeError> {
        let parent_node = self.node(parent)?;
        let host_props = ui_ext::Props::from(&parent_node.props);
        if !host_props.is_container() {
            return Err(TreeError::NotAContainer {
                node: parent,
                kind: host_props.kind_name(),
            });
        }
        let siblings = &parent_node.children;
        match before {
            Some(before) => siblings
                .iter()
                .position(|&sibling| sibling == before)
                .ok_or(TreeError::NotAChild {
                    parent,
                    child: before,
                }),
            None => Ok(siblings.len()),
        }
    }

    fn subtree_len(&self, id: NodeId) -> usize {
        let mut count = 0;
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            count += 1;
            stack.extend_from_slice(self.children(id));
        }
        count
    }

    fn commit_insert(
        &mut self,
        parent: NodeId,
        position: usize,
        before: Option<NodeId>,
        nodes: Vec<Node>,
    ) -> Vec<NodeId> {
        let top_level = nodes
            .iter()
            .filter(|node| node.parent == Some(parent))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        self.record(&nodes, Some((parent, position)));
        self.pending.insert_subtree(nodes, before);
        top_level
    }

    fn commit_remove(&mut self, id: NodeId) {
        let parent = self.nodes.get(&id).and_then(|node| node.parent);
        let siblings = match parent.and_then(|parent| self.nodes.get_mut(&parent)) {
            Some(parent) => &mut parent.children,
            None => &mut self.roots,
        };
        siblings.retain(|&sibling| sibling != id);

        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.nodes.remove(&id) {
                stack.extend(node.children);
            }
        }
        self.pending.remove(id);
    }

    /// Records parents-first `nodes`; top-level ones are placed at `position` among the
    /// children of `parent`, or appended to the roots.
    fn record(&mut self, nodes: &[Node], insert_at: Option<(NodeId, usize)>) {
        let mut position = insert_at.map(|(_, position)| position);
        for node in nodes {
            self.nodes.insert(
                node.id,
                LiveNode {
                    parent: node.parent,
                    props: node.props.clone(),
                    children: Vec::new(),
                },
            );
            let is_top_level = insert_at.map(|(parent, _)| parent) == node.parent;
            let siblings = match node.parent.and_then(|parent| self.nodes.get_mut(&parent)) {
                Some(parent) => &mut parent.children,
                None => &mut self.roots,
            };
            match position.as_mut().filter(|_| is_top_level) {
                Some(position) => {
                    let index = (*position).min(siblings.len());
                    siblings.insert(index, node.id);
                    *position = index + 1;
                }
                None => siblings.push(node.id),
            }
        }
    }
}

/// Every frozen icon in WIT declaration order, written out rather than lifted from an index, so
/// the SDK's always-linked code calls nothing in the generated bindings (which would link the 0.3
/// world description into 0.1 and 0.2 exports too). `icons_are_in_wit_order` checks it.
const ALL_ICONS: [Icon; 101] = [
    Icon::ArrowUp,
    Icon::ArrowDown,
    Icon::ArrowLeft,
    Icon::ArrowRight,
    Icon::ChevronUp,
    Icon::ChevronDown,
    Icon::ChevronLeft,
    Icon::ChevronRight,
    Icon::Expand,
    Icon::Collapse,
    Icon::Menu,
    Icon::Ellipsis,
    Icon::ListTree,
    Icon::ListTodo,
    Icon::Table,
    Icon::FileTree,
    Icon::Plus,
    Icon::Dash,
    Icon::Check,
    Icon::CheckDouble,
    Icon::Close,
    Icon::Search,
    Icon::Filter,
    Icon::Refresh,
    Icon::Undo,
    Icon::Copy,
    Icon::Pencil,
    Icon::Trash,
    Icon::Replace,
    Icon::Download,
    Icon::Send,
    Icon::Share,
    Icon::Link,
    Icon::Pin,
    Icon::Unpin,
    Icon::Play,
    Icon::Pause,
    Icon::Stop,
    Icon::Maximize,
    Icon::Minimize,
    Icon::Split,
    Icon::File,
    Icon::FileCode,
    Icon::FileDiff,
    Icon::FileText,
    Icon::Folder,
    Icon::FolderOpen,
    Icon::Code,
    Icon::Terminal,
    Icon::Diff,
    Icon::Regex,
    Icon::CaseSensitive,
    Icon::WholeWord,
    Icon::Hash,
    Icon::AtSign,
    Icon::GitBranch,
    Icon::GitCommit,
    Icon::PullRequest,
    Icon::Info,
    Icon::Warning,
    Icon::Error,
    Icon::Help,
    Icon::Bell,
    Icon::BellDot,
    Icon::Circle,
    Icon::Dot,
    Icon::Star,
    Icon::StarFilled,
    Icon::Bookmark,
    Icon::ThumbsUp,
    Icon::ThumbsDown,
    Icon::TodoPending,
    Icon::TodoProgress,
    Icon::TodoComplete,
    Icon::Spinner,
    Icon::Blank,
    Icon::Settings,
    Icon::Clock,
    Icon::Debug,
    Icon::Eye,
    Icon::EyeOff,
    Icon::Lock,
    Icon::Unlock,
    Icon::Person,
    Icon::People,
    Icon::Chat,
    Icon::Envelope,
    Icon::Book,
    Icon::Notepad,
    Icon::Image,
    Icon::Globe,
    Icon::Server,
    Icon::Database,
    Icon::Keyboard,
    Icon::Box,
    Icon::Archive,
    Icon::Sparkle,
    Icon::Bolt,
    Icon::Flame,
    Icon::Power,
    Icon::Blocks,
];

impl Icon {
    /// Every frozen icon, in WIT declaration order (101 in `fyx:ui` 0.2 and 0.3).
    pub fn all() -> impl ExactSizeIterator<Item = Icon> {
        ALL_ICONS.into_iter()
    }

    /// The WIT case name, such as `arrow-up`.
    pub fn name(self) -> &'static str {
        ui_ext::v0_2::Icon::ALL[self as usize].name()
    }
}

impl Surface {
    pub fn panel(
        id: impl Into<String>,
        title: impl Into<String>,
        position: DockPosition,
        icon: Icon,
        default_size: u32,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: SurfaceKind::Panel(PanelDescriptor {
                position,
                icon,
                default_size,
            }),
        }
    }

    /// A modal, closed until [`show_surface`] opens it. Needs `ui:modal`.
    pub fn modal(id: impl Into<String>, title: impl Into<String>, width: ModalWidth) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: SurfaceKind::Modal(ModalDescriptor { width }),
        }
    }

    pub fn status_item(
        id: impl Into<String>,
        title: impl Into<String>,
        side: StatusBarSide,
        priority: i32,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            kind: SurfaceKind::StatusItem(StatusItemDescriptor { side, priority }),
        }
    }
}

impl FlexProps {
    /// A column with no gap, no padding and children aligned to the start.
    pub fn column() -> Self {
        Self {
            direction: FlexDirection::Column,
            gap: 0,
            padding: 0,
            grow: false,
            align: Align::Start,
            background: None,
        }
    }

    /// A row with no gap, no padding and children aligned to the start.
    pub fn row() -> Self {
        Self {
            direction: FlexDirection::Row,
            ..Self::column()
        }
    }

    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn padding(mut self, padding: u16) -> Self {
        self.padding = padding;
        self
    }

    pub fn grow(mut self) -> Self {
        self.grow = true;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn background(mut self, background: ColorToken) -> Self {
        self.background = Some(background);
        self
    }
}

impl TextProps {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            color: None,
            bold: false,
            monospace: false,
        }
    }

    pub fn color(mut self, color: ColorToken) -> Self {
        self.color = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }
}

impl From<&str> for TextProps {
    fn from(content: &str) -> Self {
        Self::new(content)
    }
}

impl From<String> for TextProps {
    fn from(content: String) -> Self {
        Self::new(content)
    }
}

impl ButtonProps {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            style: ButtonStyle::Default,
            disabled: false,
            tooltip: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Shown on hover; the host truncates text over 1024 bytes.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl From<&str> for ButtonProps {
    fn from(label: &str) -> Self {
        Self::new(label)
    }
}

impl From<String> for ButtonProps {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

impl InputProps {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            placeholder: String::new(),
            secret: false,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Masks the text on screen (passwords, tokens). The value still reaches the extension.
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }
}

impl ListProps {
    pub fn new(row_count: u32, row_height: u16) -> Self {
        Self {
            row_count,
            row_height,
            selected: None,
        }
    }

    /// The highlighted row, scrolled into view.
    pub fn selected(mut self, selected: Option<u32>) -> Self {
        self.selected = selected;
        self
    }
}

impl TableProps {
    pub fn new(columns: impl IntoIterator<Item = impl Into<String>>, row_count: u32) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            row_count,
        }
    }
}

impl IconProps {
    pub fn new(name: Icon) -> Self {
        Self { name, color: None }
    }

    pub fn color(mut self, color: ColorToken) -> Self {
        self.color = Some(color);
        self
    }
}

impl From<Icon> for IconProps {
    fn from(name: Icon) -> Self {
        Self::new(name)
    }
}

impl ListRow {
    /// Nesting depth, drawn by the host as indentation (tree views); at most 32 levels.
    pub fn with_indent(mut self, indent: u8) -> Self {
        self.indent = indent;
        self
    }
}

impl DiffViewProps {
    pub fn new(old_text: impl Into<String>, new_text: impl Into<String>) -> Self {
        Self {
            old_text: old_text.into(),
            new_text: new_text.into(),
            language: None,
        }
    }

    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }
}

macro_rules! impl_into_props {
    ($($props_type:ident => $variant:ident),* $(,)?) => {
        $(
            impl From<$props_type> for Props {
                fn from(props: $props_type) -> Self {
                    Props::$variant(props)
                }
            }
        )*
    };
}

impl_into_props! {
    FlexProps => Flex,
    TextProps => Text,
    ButtonProps => Button,
    InputProps => Input,
    ListProps => List,
    TableProps => Table,
    IconProps => Icon,
    DiffViewProps => DiffView,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(content: &str) -> Props {
        TextProps::new(content).into()
    }

    fn to_host(nodes: &[Node]) -> Vec<ui_ext::Node> {
        nodes.iter().map(Into::into).collect()
    }

    /// The host's tree after `mount`, built by `ui_ext` itself.
    fn host_tree(nodes: &[Node]) -> ui_ext::Tree {
        ui_ext::Tree::from_nodes(to_host(nodes), TreeLimits::default())
            .expect("the host accepts the mount")
    }

    /// Applies an update with the host's own rules and checks the host now holds the SDK's view.
    fn apply(host: &mut ui_ext::Tree, surface: &mut SurfaceTree) {
        host.apply_update((&surface.take_update()).into())
            .expect("the host accepts the update");
        assert_eq!(host.to_nodes(), to_host(&surface.to_nodes()));
    }

    #[test]
    fn builder_flattens_nested_code_parents_first() {
        let mut ids = IdAllocator::new();
        let mut tree = TreeBuilder::new(&mut ids);
        let mut leaves = Vec::new();
        let root = tree.column(|tree| {
            leaves.push(tree.text("title"));
            tree.row(|tree| {
                leaves.push(tree.button("a"));
                leaves.push(tree.button("b"));
            });
            leaves.push(tree.list(ListProps::new(3, 20)));
        });
        let nodes = tree.finish();

        let shape = nodes
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(3)),
                (5, Some(3)),
                (6, Some(1)),
            ]
        );
        assert_eq!(root, 1);
        assert_eq!(leaves, [2, 4, 5, 6]);
        assert_eq!(nodes[1].props, text("title"));
        assert_eq!(ids.peek(), 7);
    }

    #[test]
    fn builder_under_parent_and_sibling_roots() {
        let mut ids = IdAllocator::starting_at(10);
        let mut tree = TreeBuilder::under(&mut ids, 3);
        tree.text("first");
        tree.column(|tree| {
            tree.text("nested");
        });
        let shape = tree
            .finish()
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(shape, [(10, Some(3)), (11, Some(3)), (12, Some(11))]);
    }

    #[test]
    fn props_builders() {
        let flex = FlexProps::row()
            .gap(4)
            .padding(8)
            .grow()
            .background(ColorToken::Surface);
        assert_eq!(flex.direction, FlexDirection::Row);
        assert_eq!((flex.gap, flex.padding, flex.grow), (4, 8, true));
        assert_eq!(flex.background, Some(ColorToken::Surface));

        let button = ButtonProps::new("Go")
            .style(ButtonStyle::Filled)
            .icon(Icon::Play)
            .disabled(true)
            .tooltip("Run it");
        assert_eq!(
            Props::from(button),
            Props::Button(ButtonProps {
                label: "Go".into(),
                icon: Some(Icon::Play),
                style: ButtonStyle::Filled,
                disabled: true,
                tooltip: Some("Run it".into()),
            })
        );
        assert!(InputProps::new("x").secret().secret);
        assert_eq!(ListProps::new(3, 20).selected(Some(2)).selected, Some(2));
        assert_eq!(IconProps::from(Icon::Check).name, Icon::Check);
        let modal = Surface::modal("picker", "Pick", ModalWidth::Small);
        assert_eq!(
            modal.kind,
            SurfaceKind::Modal(ModalDescriptor {
                width: ModalWidth::Small
            })
        );

        let table = TableProps::new(["File", "Line"], 2);
        assert_eq!(table.columns, ["File", "Line"]);
    }

    #[test]
    fn patch_set_insert_subtree_only_positions_top_level_nodes() {
        let mut ids = IdAllocator::starting_at(20);
        let mut tree = TreeBuilder::under(&mut ids, 1);
        tree.row(|tree| {
            tree.text("child");
        });
        tree.text("second");
        let mut patches = PatchSet::new();
        patches.insert_subtree(tree.finish(), Some(7));

        let befores = patches
            .patches()
            .iter()
            .map(|patch| match patch {
                Patch::Insert(insert) => (insert.node.id, insert.before),
                _ => panic!("expected only inserts"),
            })
            .collect::<Vec<_>>();
        assert_eq!(befores, [(20, Some(7)), (21, None), (22, Some(7))]);
    }

    #[test]
    fn patch_set_into_update() {
        assert_eq!(PatchSet::new().into_update(), Update::Unchanged);

        let mut patches = PatchSet::new();
        patches.set_props(2, TextProps::new("x")).remove(5);
        assert_eq!(patches.len(), 2);
        assert_eq!(
            Update::from(patches),
            Update::Patches(vec![Patch::SetProps((2, text("x"))), Patch::Remove(5),])
        );
    }

    #[test]
    fn surface_tree_skips_unchanged_props() {
        let mut surface = SurfaceTree::new();
        let mut label = 0;
        surface
            .mount(|tree| {
                tree.column(|tree| {
                    label = tree.text("0");
                });
            })
            .expect("mount");

        assert_eq!(surface.set_props(label, TextProps::new("0")), Ok(false));
        assert_eq!(surface.take_update(), Update::Unchanged);

        assert_eq!(surface.set_props(label, TextProps::new("1")), Ok(true));
        assert_eq!(surface.set_props(label, TextProps::new("1")), Ok(false));
        assert_eq!(
            surface.take_update(),
            Update::Patches(vec![Patch::SetProps((label, text("1")))])
        );
        assert_eq!(surface.props(label), Some(&text("1")));
        assert_eq!(surface.take_update(), Update::Unchanged);

        surface
            .set_props_from_host(label, TextProps::new("2"))
            .expect("known node");
        assert_eq!(surface.props(label), Some(&text("2")));
        assert_eq!(surface.take_update(), Update::Unchanged);
    }

    #[test]
    fn surface_tree_rejects_edits_before_mount_and_on_unknown_nodes() {
        let mut surface = SurfaceTree::new();
        assert_eq!(
            surface.set_props(1, TextProps::new("x")),
            Err(TreeError::NotMounted)
        );

        let mut column = 0;
        let mut first = 0;
        surface
            .mount(|tree| {
                column = tree.column(|tree| {
                    first = tree.text("a");
                });
            })
            .expect("mount");
        assert_eq!(surface.remove(99), Err(TreeError::UnknownNode(99)));
        assert_eq!(
            surface.insert(column, Some(column), |tree| {
                tree.text("x");
            }),
            Err(TreeError::NotAChild {
                parent: column,
                child: column,
            })
        );
        assert!(surface.contains(first));
        let message: String = TreeError::UnknownNode(99).into();
        assert_eq!(message, "node 99 is not in the surface's tree");
    }

    #[test]
    fn surface_tree_patches_reproduce_its_tree() {
        let mut surface = SurfaceTree::new();
        let mut column = 0;
        let mut first = 0;
        let mut last = 0;
        let mounted = surface
            .mount(|tree| {
                column = tree.column(|tree| {
                    first = tree.text("first");
                    tree.row(|tree| {
                        tree.button("a");
                        tree.button("b");
                    });
                    last = tree.text("last");
                });
            })
            .expect("mount");
        assert_eq!(mounted, surface.to_nodes());
        let mut host = host_tree(&mounted);

        let inserted = surface
            .insert(column, Some(last), |tree| {
                tree.row(|tree| {
                    tree.text("inserted child");
                });
                tree.text("inserted sibling");
            })
            .expect("insert");
        assert_eq!(inserted.len(), 2);
        surface
            .set_props(first, TextProps::new("first, edited").bold())
            .expect("set props");
        surface
            .insert(column, None, |tree| {
                tree.text("appended");
            })
            .expect("append");
        surface.remove(inserted[0]).expect("remove");
        assert!(!surface.contains(inserted[0] + 1));

        apply(&mut host, &mut surface);

        let order = surface
            .children(column)
            .iter()
            .map(|&id| surface.props(id).cloned())
            .collect::<Vec<_>>();
        assert_eq!(order.len(), 5);
        assert_eq!(
            order[0],
            Some(TextProps::new("first, edited").bold().into())
        );
        assert_eq!(order[2], Some(text("inserted sibling")));
        assert_eq!(order[3], Some(text("last")));
        assert_eq!(order[4], Some(text("appended")));

        let middle = surface.children(column)[2];
        let rebuilt = surface
            .rebuild(middle, |tree| {
                tree.text("rebuilt");
            })
            .expect("rebuild");
        assert!(!surface.contains(middle));
        assert_eq!(surface.children(column)[2], rebuilt[0]);
        assert_eq!(
            surface.rebuild(column, |_| {}),
            Err(TreeError::IsRoot(column))
        );
        apply(&mut host, &mut surface);

        surface.clear_children(column).expect("clear");
        apply(&mut host, &mut surface);
        assert_eq!(surface.len(), 1);
        assert_eq!(surface.roots(), [column]);
    }

    #[test]
    fn surface_tree_never_reuses_ids_across_mounts() {
        let mut surface = SurfaceTree::new();
        let first = surface
            .mount(|tree| {
                tree.text("a");
            })
            .expect("mount");
        let Ok(Update::Replace(second)) = surface.replace(|tree| {
            tree.text("b");
        }) else {
            panic!("expected a replace");
        };
        assert_eq!(first[0].id, 1);
        assert_eq!(second[0].id, 2);
        assert!(!surface.contains(1));
    }

    #[test]
    fn list_rows_clamp_and_use_row_ids() {
        assert_eq!(row_range(0, 10, 4), 0..4);
        assert_eq!(row_range(6, 10, 4), 4..4);
        assert_eq!(row_range(u32::MAX, 10, u32::MAX), u32::MAX..u32::MAX);

        let rows = list_rows(2, 5, 4, |index, tree| {
            tree.row(|tree| {
                tree.text(format!("row {index}"));
            });
        });
        assert_eq!(rows.iter().map(|row| row.index).collect::<Vec<_>>(), [2, 3]);
        let row = &rows[0];
        assert_eq!(row.nodes[0].id, ROW_NODE_ID_BASE);
        assert_eq!(row.nodes[0].parent, None);
        assert_eq!(row.nodes[1].parent, Some(ROW_NODE_ID_BASE));
        assert_eq!(row.nodes[1].props, text("row 2"));
    }

    #[test]
    fn rows_the_host_would_reject_are_left_out() {
        assert_eq!(
            list_row(0, |_| {}),
            Err(TreeError::InvalidRow(ui_ext::TreeError::RowRootCount(0)))
        );
        assert_eq!(
            list_row(0, |tree| {
                tree.text("a");
                tree.text("b");
            }),
            Err(TreeError::InvalidRow(ui_ext::TreeError::RowRootCount(2)))
        );

        let rows = list_rows(0, 3, 3, |index, tree| {
            if index != 1 {
                tree.text(format!("row {index}"));
            }
        });
        assert_eq!(rows.iter().map(|row| row.index).collect::<Vec<_>>(), [0, 2]);
        for row in &rows {
            ui_ext::Tree::from_row(to_host(&row.nodes), TreeLimits::default())
                .expect("the host accepts the row");
        }
    }

    #[test]
    fn host_functions_work_off_wasm() {
        log(LogLevel::Info, "logged to stderr in tests");
        push_update("echo", &Update::Unchanged).expect("off WASM pushes succeed");
        assert_eq!(
            take_pushed_updates(),
            [("echo".to_string(), Update::Unchanged)]
        );
        assert!(take_pushed_updates().is_empty());
    }

    #[test]
    fn icons_are_in_wit_order() {
        let icons = Icon::all().collect::<Vec<_>>();
        assert_eq!(icons.len(), 101);
        for (icon, model) in icons.iter().zip(ui_ext::v0_2::Icon::ALL) {
            // wit-bindgen's `Debug` prints `Icon::ArrowUp`; the model's prints `ArrowUp`.
            let wit = format!("{icon:?}");
            assert_eq!(wit.rsplit("::").next(), Some(format!("{model:?}").as_str()));
            assert_eq!(icon.name(), model.name());
        }
        assert_eq!(Icon::ArrowUp.name(), "arrow-up");
        assert_eq!(Icon::Blocks.name(), "blocks");
    }

    #[test]
    fn api_versions_match_the_wit_packages() {
        assert_eq!(API_VERSION_SECTION_BYTES, [0, 0, 0, 3, 0, 0]);
        assert_eq!(API_VERSION_V0_2_SECTION_BYTES, [0, 0, 0, 2, 0, 0]);
        assert_eq!(API_VERSION_V0_1_SECTION_BYTES, [0, 0, 0, 1, 0, 0]);
        for (version, wit) in [
            (
                API_VERSION,
                include_str!("../../ui_ext/wit/since_v0.3.0/fyx-ui.wit"),
            ),
            (
                API_VERSION_V0_2,
                include_str!("../../ui_ext/wit/since_v0.2.0/fyx-ui.wit"),
            ),
            (
                API_VERSION_V0_1,
                include_str!("../../ui_ext/wit/fyx-ui.wit"),
            ),
        ] {
            let (major, minor, patch) = version;
            assert!(
                wit.contains(&format!("package fyx:ui@{major}.{minor}.{patch};")),
                "{version:?}"
            );
        }
    }

    #[test]
    fn invalidate_rows_only_on_lists() {
        let mut surface = SurfaceTree::new();
        let (mut list, mut label) = (0, 0);
        let mounted = surface
            .mount(|tree| {
                tree.column(|tree| {
                    label = tree.text("label");
                    list = tree.list(ListProps::new(10, 20));
                });
            })
            .expect("mount");
        let mut host = host_tree(&mounted);
        assert_eq!(
            surface.invalidate_rows(label, None),
            Err(TreeError::NotAList {
                node: label,
                kind: "text"
            })
        );
        let range = RowRange { start: 2, count: 3 };
        surface.invalidate_rows(list, Some(range)).expect("a list");
        assert_eq!(
            surface.pending().patches(),
            [Patch::InvalidateRows(InvalidateRowsOp {
                target: list,
                rows: Some(range)
            })]
        );
        // It changes no node, so the host's tree is the same before and after.
        apply(&mut host, &mut surface);
    }

    #[test]
    fn row_selected_is_mirrored_like_typed_text() {
        let mut surface = SurfaceTree::new();
        let mut list = 0;
        surface
            .mount(|tree| {
                tree.column(|tree| {
                    list = tree.list(ListProps::new(10, 20));
                });
            })
            .expect("mount");
        surface.apply_event(&UiEvent::RowSelected((list, 4)));
        assert_eq!(
            surface.props(list),
            Some(&ListProps::new(10, 20).selected(Some(4)).into())
        );
        assert_eq!(
            surface.set_props(list, ListProps::new(10, 20).selected(Some(4))),
            Ok(false),
            "echoing the host's selection queues nothing"
        );
    }

    /// bugs-1 #20: the host applies a queued `set-props` after the event, so the mirror keeps it.
    #[test]
    fn apply_event_keeps_a_queued_set_props() {
        let mut surface = SurfaceTree::new();
        let mut input = 0;
        surface
            .mount(|tree| {
                tree.column(|tree| {
                    input = tree.input(InputProps::new("draft"));
                });
            })
            .expect("mount");
        assert_eq!(surface.set_props(input, InputProps::new("")), Ok(true));
        surface.apply_event(&UiEvent::InputChanged((input, "typed".into())));
        assert_eq!(surface.props(input), Some(&InputProps::new("").into()));
        assert_eq!(
            surface.take_update(),
            Update::Patches(vec![Patch::SetProps((input, InputProps::new("").into()))])
        );

        // Nothing queued: the typed text is mirrored, and clearing it queues the set-props again.
        surface.apply_event(&UiEvent::InputChanged((input, "typed".into())));
        assert_eq!(surface.props(input), Some(&InputProps::new("typed").into()));
        assert_eq!(surface.set_props(input, InputProps::new("")), Ok(true));
    }

    /// bugs-1 #11: a mount is capped like the host's, at `max_nodes`, not at one update's size.
    #[test]
    fn a_tree_grown_past_one_update_can_be_mounted_again() {
        const COUNT: usize = 3000;
        let mut surface = SurfaceTree::new();
        let mut root = 0;
        surface
            .mount(|tree| {
                root = tree.column(|_| {});
            })
            .expect("mount");
        for _ in 0..3 {
            surface
                .insert(root, None, |tree| {
                    for _ in 0..COUNT / 3 {
                        tree.text("row");
                    }
                })
                .expect("insert");
            surface.take_update();
        }
        assert_eq!(surface.len(), COUNT + 1);

        let nodes = surface
            .mount(|tree| {
                tree.column(|tree| {
                    for _ in 0..COUNT {
                        tree.text("row");
                    }
                });
            })
            .expect("a remount of the grown tree");
        assert_eq!(nodes.len(), COUNT + 1);
        assert!(matches!(
            surface.mount(|tree| {
                tree.column(|tree| {
                    for _ in 0..TreeLimits::default().max_nodes {
                        tree.text("row");
                    }
                });
            }),
            Err(TreeError::TreeTooLarge { .. })
        ));
    }

    #[test]
    fn push_records_and_a_failed_push_resends_the_whole_tree() {
        take_host_effects();
        let mut surface = SurfaceTree::new();
        let mut label = 0;
        surface
            .mount(|tree| {
                tree.column(|tree| {
                    label = tree.text("0");
                });
            })
            .expect("mount");
        surface.set_props(label, TextProps::new("1")).expect("set");
        surface.push("status").expect("off WASM pushes succeed");
        assert_eq!(
            take_host_effects(),
            [HostEffect::Push {
                surface_id: "status".into(),
                update: Update::Patches(vec![Patch::SetProps((label, text("1")))]),
            }]
        );
        surface.push("status").expect("nothing to push");
        assert!(take_host_effects().is_empty());

        // What `push` does when the host refuses: the next update is the whole tree.
        surface.unsynced = true;
        assert_eq!(surface.take_update(), Update::Replace(surface.to_nodes()));
        assert!(!surface.is_unsynced());
        assert_eq!(surface.take_update(), Update::Unchanged);
    }

    #[test]
    fn host_functions_record_their_effects() {
        take_host_effects();
        request_tick("clock", 1000).expect("tick");
        cancel_tick("clock");
        let picker = Surface::modal("picker", "Pick", ModalWidth::Medium);
        add_surface(&picker).expect("add");
        show_surface("picker").expect("show");
        let location = DecorationLocation {
            target: DecorationTarget::Path("src/main.rs".into()),
            line: 3,
            placement: DecorationPlacement::Below,
            height_lines: None,
        };
        place_decoration("todo-3", &location, &[]).expect("place");
        assert_eq!(
            take_host_effects(),
            [
                HostEffect::RequestTick {
                    surface_id: "clock".into(),
                    delay_ms: 1000
                },
                HostEffect::CancelTick {
                    surface_id: "clock".into()
                },
                HostEffect::AddSurface(picker),
                HostEffect::ShowSurface("picker".into()),
                HostEffect::PlaceDecoration {
                    id: "todo-3".into(),
                    location,
                    nodes: Vec::new()
                },
            ]
        );
    }

    /// A mounted column holding a text and a row with one button; returns (column, text, row).
    fn small_surface(limits: TreeLimits) -> (SurfaceTree, NodeId, NodeId, NodeId) {
        let mut surface = SurfaceTree::with_limits(limits);
        let (mut text_id, mut row_id) = (0, 0);
        let column = surface
            .mount(|tree| {
                tree.column(|tree| {
                    text_id = tree.text("label");
                    row_id = tree.row(|tree| {
                        tree.button("go");
                    });
                });
            })
            .expect("mount")[0]
            .id;
        (surface, column, text_id, row_id)
    }

    fn snapshot(surface: &SurfaceTree) -> (Vec<Node>, PatchSet) {
        (surface.to_nodes(), surface.pending().clone())
    }

    #[test]
    fn insert_only_under_flex() {
        let (mut surface, column, text_id, _) = small_surface(TreeLimits::default());
        surface
            .set_props(text_id, TextProps::new("queued"))
            .expect("set");
        let before = snapshot(&surface);

        let error = surface.insert(text_id, None, |tree| {
            tree.text("child of a text");
        });
        assert_eq!(
            error,
            Err(TreeError::NotAContainer {
                node: text_id,
                kind: "text",
            })
        );
        assert_eq!(snapshot(&surface), before);

        assert_eq!(
            surface.insert(column, Some(column), |_| {}),
            Err(TreeError::NotAChild {
                parent: column,
                child: column,
            })
        );
        assert_eq!(snapshot(&surface), before);
        assert!(
            surface
                .insert(column, None, |tree| {
                    tree.text("ok");
                })
                .is_ok()
        );
    }

    #[test]
    fn flex_with_children_stays_flex() {
        let (mut surface, column, text_id, row_id) = small_surface(TreeLimits::default());
        let before = snapshot(&surface);

        assert_eq!(
            surface.set_props(row_id, TextProps::new("leaf")),
            Err(TreeError::NotAContainer {
                node: row_id,
                kind: "text",
            })
        );
        assert_eq!(
            surface.set_props_from_host(column, ButtonProps::new("leaf")),
            Err(TreeError::NotAContainer {
                node: column,
                kind: "button",
            })
        );
        assert_eq!(snapshot(&surface), before);

        assert_eq!(
            surface.set_props(row_id, FlexProps::column().gap(2)),
            Ok(true)
        );
        assert_eq!(
            surface.set_props(text_id, ButtonProps::new("leaf")),
            Ok(true)
        );
        surface.clear_children(row_id).expect("clear");
        assert_eq!(
            surface.set_props(row_id, TextProps::new("now a leaf")),
            Ok(true)
        );
    }

    #[test]
    fn updates_stay_under_the_patch_cap() {
        let limits = TreeLimits {
            max_nodes_per_update: 4,
            max_nodes: 100,
            ..TreeLimits::default()
        };
        let (mut surface, column, text_id, row_id) = small_surface(limits);
        for index in 0..4 {
            surface
                .set_props(text_id, TextProps::new(index.to_string()))
                .expect("under the cap");
        }
        let before = snapshot(&surface);
        let too_large = Err(TreeError::UpdateTooLarge { count: 5, max: 4 });
        assert_eq!(surface.set_props(text_id, TextProps::new("5")), too_large);
        assert_eq!(surface.remove(text_id), too_large.clone().map(|_: bool| ()));
        assert_eq!(
            surface
                .insert(column, None, |tree| {
                    tree.text("x");
                })
                .map(|_| ()),
            too_large.map(|_: bool| ())
        );
        assert_eq!(snapshot(&surface), before);

        assert!(matches!(surface.take_update(), Update::Patches(patches) if patches.len() == 4));
        let error = surface.insert(column, None, |tree| {
            tree.row(|tree| {
                tree.text("a");
                tree.text("b");
                tree.text("c");
                tree.text("d");
            });
        });
        assert_eq!(error, Err(TreeError::UpdateTooLarge { count: 5, max: 4 }));
        let error = surface.rebuild(row_id, |tree| {
            tree.row(|tree| {
                tree.text("a");
                tree.text("b");
                tree.text("c");
            });
        });
        assert_eq!(error, Err(TreeError::UpdateTooLarge { count: 5, max: 4 }));
        assert!(surface.contains(row_id));
        assert_eq!(surface.take_update(), Update::Unchanged);

        // A mount is capped by `max_nodes` only, like the host's (bugs-1 #11).
        let mounted = surface.mount(|tree| {
            tree.column(|tree| {
                for _ in 0..4 {
                    tree.text("x");
                }
            });
        });
        assert_eq!(mounted.map(|nodes| nodes.len()), Ok(5));
        assert!(!surface.contains(text_id));
    }

    #[test]
    fn tree_stays_under_the_node_cap() {
        let limits = TreeLimits {
            max_nodes_per_update: 10,
            max_nodes: 5,
            ..TreeLimits::default()
        };
        let (mut surface, column, text_id, _) = small_surface(limits);
        surface
            .insert(column, None, |tree| {
                tree.text("fifth");
            })
            .expect("at the cap");
        let before = snapshot(&surface);
        assert_eq!(
            surface
                .insert(column, None, |tree| {
                    tree.text("sixth");
                })
                .map(|_| ()),
            Err(TreeError::TreeTooLarge { count: 6, max: 5 })
        );
        assert_eq!(
            surface
                .rebuild(text_id, |tree| {
                    tree.text("a");
                    tree.text("b");
                })
                .map(|_| ()),
            Err(TreeError::TreeTooLarge { count: 6, max: 5 })
        );
        assert_eq!(snapshot(&surface), before);
        assert!(
            surface
                .rebuild(text_id, |tree| {
                    tree.text("replacement");
                })
                .is_ok()
        );
    }

    /// What the host does on `input-changed` / `input-submitted` before it sends the event.
    fn host_types(host: &mut ui_ext::Tree, id: NodeId, value: &str) {
        let Some(ui_ext::Props::Input(input)) = host.get(id).map(|node| node.props().clone())
        else {
            panic!("not an input");
        };
        let typed = ui_ext::InputProps {
            value: value.into(),
            ..input
        };
        host.apply_patches(vec![ui_ext::Patch::SetProps(
            id,
            ui_ext::Props::Input(typed),
        )])
        .expect("the host records typed text");
    }

    #[test]
    fn apply_event_keeps_typed_text_in_sync_with_the_host() {
        let mut surface = SurfaceTree::new();
        let (mut input, mut label) = (0, 0);
        let mounted = surface
            .mount(|tree| {
                tree.column(|tree| {
                    input = tree.input(InputProps::new("").placeholder("Name"));
                    label = tree.text("label");
                });
            })
            .expect("mount");
        let mut host = host_tree(&mounted);

        for event in [
            UiEvent::InputChanged((input, "ab".into())),
            UiEvent::InputSubmitted((input, "abc".into())),
        ] {
            let (UiEvent::InputChanged((_, value)) | UiEvent::InputSubmitted((_, value))) = &event
            else {
                unreachable!();
            };
            host_types(&mut host, input, value);
            surface.apply_event(&event);
            assert_eq!(host.to_nodes(), to_host(&surface.to_nodes()));
        }
        assert_eq!(
            surface.props(input),
            Some(&InputProps::new("abc").placeholder("Name").into())
        );

        // Echoing the typed text queues nothing; clearing the field does reach the host.
        let typed = InputProps::new("abc").placeholder("Name");
        assert_eq!(surface.set_props(input, typed), Ok(false));
        let cleared = InputProps::new("").placeholder("Name");
        assert_eq!(surface.set_props(input, cleared), Ok(true));
        apply(&mut host, &mut surface);

        // Events for other nodes, or unknown ones, change nothing.
        let before = surface.to_nodes();
        surface.apply_event(&UiEvent::InputChanged((label, "x".into())));
        surface.apply_event(&UiEvent::InputChanged((999, "x".into())));
        surface.apply_event(&UiEvent::Click(input));
        assert_eq!(surface.to_nodes(), before);
        assert_eq!(surface.take_update(), Update::Unchanged);
    }

    #[test]
    fn without_apply_event_clearing_a_typed_field_is_lost() {
        // The failure `apply_event` prevents (judge integration-1 #6): the mirror still holds ""
        // after the user typed, so clearing the field looks like no change.
        let mut surface = SurfaceTree::new();
        let mut input = 0;
        surface
            .mount(|tree| {
                tree.column(|tree| {
                    input = tree.input(InputProps::new(""));
                });
            })
            .expect("mount");
        assert_eq!(surface.set_props(input, InputProps::new("")), Ok(false));
    }

    struct Echo {
        events: u32,
    }

    impl Extension for Echo {
        fn new() -> Self {
            Self { events: 0 }
        }

        fn surfaces(&self) -> Vec<Surface> {
            vec![Surface::panel(
                "echo",
                "Echo",
                DockPosition::Left,
                Icon::Blocks,
                200,
            )]
        }

        fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
            let mut ids = IdAllocator::new();
            let mut tree = TreeBuilder::new(&mut ids);
            tree.text(surface_id);
            Ok(tree.finish())
        }

        fn handle_event(&mut self, _surface_id: &str, _event: UiEvent) -> Result<Update, String> {
            self.events += 1;
            let mut patches = PatchSet::new();
            patches.set_props(1, TextProps::new(self.events.to_string()));
            Ok(patches.into_update())
        }
    }

    #[test]
    fn dispatch_keeps_one_instance() {
        fn new_echo() -> Box<dyn Extension> {
            Box::new(Echo::new())
        }

        let surfaces = __private::with_extension(new_echo, |extension| extension.surfaces());
        assert_eq!(surfaces[0].id, "echo");
        for expected in ["1", "2"] {
            let update = __private::with_extension(new_echo, |extension| {
                extension.handle_event("echo", UiEvent::Click(1))
            });
            assert_eq!(
                update,
                Ok(Update::Patches(vec![Patch::SetProps((1, text(expected)))]))
            );
        }
        let rows =
            __private::with_extension(new_echo, |extension| extension.list_rows("echo", 1, 0, 10));
        assert!(rows.is_empty());
    }
}
