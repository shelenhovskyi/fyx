//! The calls the renderer makes into an extension, behind a trait so the renderer can be tested
//! without compiling a WASM component.

use anyhow::Result;
use extension_host::wasm_host::fyx_ui::UiExtension;
use futures::{FutureExt as _, future::LocalBoxFuture};
use ui_ext::{NodeId, Tree, UiEvent, Update};

/// One loaded UI extension. Every call is asynchronous and runs off the frame; the renderer never
/// awaits one inside `render()`.
pub trait UiBackend: 'static {
    fn extension_id(&self) -> &str;
    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Tree>>;
    /// The updates to apply for one event, in order: anything the extension pushed while handling
    /// it, then the reply itself, last.
    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Vec<Update>>>;
    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Vec<(u32, Tree)>>>;
}

impl UiBackend for UiExtension {
    fn extension_id(&self) -> &str {
        UiExtension::extension_id(self)
    }

    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Tree>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { UiExtension::mount(&this, &surface_id).await }.boxed_local()
    }

    // The one place to adapt when W2's `handle_event` starts returning the pushes made during the
    // call together with the reply (integrator decision 3). Until then pushes arrive on the
    // separate channel and the reply is the only update here.
    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Vec<Update>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move {
            let reply = UiExtension::handle_event(&this, &surface_id, event).await?;
            Ok(vec![reply])
        }
        .boxed_local()
    }

    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Vec<(u32, Tree)>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { UiExtension::list_rows(&this, &surface_id, list_node, start, count).await }
            .boxed_local()
    }
}
