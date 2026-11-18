//! The calls the renderer makes into an extension, behind a trait so the renderer can be tested
//! without compiling a WASM component.

use anyhow::Result;
use futures::{FutureExt as _, future::LocalBoxFuture};
use ui_ext::{NodeId, Tree, UiEvent, Update};
use ui_ext_wasm::UiExtension;

pub use ui_ext_wasm::{PushedUpdate, Reply, ReplyError};

/// One loaded UI extension. Every call is asynchronous and runs off the frame; the renderer never
/// awaits one inside `render()`.
///
/// Every reply carries the updates the extension pushed during the call (for any of its
/// surfaces), in emission order; they are applied before the reply's value.
pub trait UiBackend: 'static {
    fn extension_id(&self) -> &str;
    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>>;
    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Reply<Update>>>;
    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<(u32, Tree)>>>>;
    /// Recent calls, `log` output and capability checks, for the Extension Inspector.
    fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        None
    }
}

impl UiBackend for UiExtension {
    fn extension_id(&self) -> &str {
        UiExtension::extension_id(self)
    }

    fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        Some(UiExtension::diagnostics(self))
    }

    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { UiExtension::mount(&this, &surface_id).await }.boxed_local()
    }

    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { UiExtension::handle_event(&this, &surface_id, event).await }.boxed_local()
    }

    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<(u32, Tree)>>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { UiExtension::list_rows(&this, &surface_id, list_node, start, count).await }
            .boxed_local()
    }
}
