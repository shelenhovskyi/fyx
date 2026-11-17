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
    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Update>>;
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

    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Update>> {
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
    ) -> LocalBoxFuture<'static, Result<Vec<(u32, Tree)>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { UiExtension::list_rows(&this, &surface_id, list_node, start, count).await }
            .boxed_local()
    }
}
