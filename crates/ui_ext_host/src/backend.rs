//! The calls the renderer makes into an extension, behind a trait so the renderer can be tested
//! without compiling a WASM component.
//!
//! The renderer works on the 0.2 model (`ui_ext::v0_2`) for every extension. `ui_ext_wasm` hands
//! a 0.1 component's results over in that model too (the 2.1 `From` impls), and both versions'
//! side effects as [`HostEffect`]s, in emission order. What differs by version is decided from
//! [`UiBackend::api_version`] (migration.md §3).

use anyhow::Result;
use futures::{FutureExt as _, future::LocalBoxFuture};
use semver::Version;
use ui_ext::{
    TreeLimits,
    v0_2::{ListRow, NodeId, UiEvent, Update, tree::Tree},
};
use ui_ext_wasm::{EffectReply, EffectReplyError, UiExtension};

pub use ui_ext_wasm::HostEffect;

/// The result of a call together with the effects the guest made while it ran: apply `effects`
/// in order, then `value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply<T> {
    pub effects: Vec<HostEffect>,
    pub value: T,
}

/// A call that ran but whose result was rejected (the extension's own error, or a reply that
/// failed validation), with the effects it made, which still apply. Reach it with
/// `error.downcast_ref::<ReplyError>()`; other errors (trap, budget, disabled) carry none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyError {
    pub effects: Vec<HostEffect>,
    pub message: String,
}

impl std::fmt::Display for ReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReplyError {}

/// One list or table row, validated, with its indent level as drawn (clamped; 0 for 0.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub index: u32,
    pub indent: u8,
    pub tree: Tree,
}

/// `ui_ext_wasm`'s rejection, with its context chain, as the renderer's.
fn convert_error(error: anyhow::Error) -> anyhow::Error {
    let rejected = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<EffectReplyError>())
        .cloned();
    match rejected {
        Some(EffectReplyError { effects, message }) => {
            anyhow::Error::new(ReplyError { effects, message })
        }
        None => error,
    }
}

fn convert_reply<T, U>(
    result: Result<EffectReply<T>>,
    value: impl FnOnce(T) -> Result<U>,
) -> Result<Reply<U>> {
    let reply = result.map_err(convert_error)?;
    match value(reply.value) {
        Ok(value) => Ok(Reply {
            effects: reply.effects,
            value,
        }),
        Err(error) => Err(anyhow::Error::new(ReplyError {
            effects: reply.effects,
            message: format!("{error:#}"),
        })),
    }
}

/// What a stopped surface keeps in place of its instance: every call fails, so nothing reaches a
/// dead instance and its `Store` is freed (triage-3, ASYNC 7). The last diagnostics stay, for
/// the Extension Inspector.
pub(crate) struct DeadBackend {
    extension_id: String,
    version: Version,
    diagnostics: Option<ui_ext_wasm::diagnostics::Diagnostics>,
}

impl DeadBackend {
    pub(crate) fn of(backend: &dyn UiBackend) -> Self {
        Self {
            extension_id: backend.extension_id().to_string(),
            version: backend.api_version(),
            diagnostics: backend.diagnostics(),
        }
    }

    fn error<T: 'static>(&self) -> LocalBoxFuture<'static, Result<T>> {
        let error = anyhow::anyhow!("{} is not running", self.extension_id);
        async move { Err(error) }.boxed_local()
    }
}

impl UiBackend for DeadBackend {
    fn extension_id(&self) -> &str {
        &self.extension_id
    }

    fn api_version(&self) -> Version {
        self.version.clone()
    }

    fn mount(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        self.error()
    }

    fn handle_event(&self, _: &str, _: UiEvent) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        self.error()
    }

    fn list_rows(
        &self,
        _: &str,
        _: NodeId,
        _: u32,
        _: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<Row>>>> {
        self.error()
    }

    fn tick(&self, _: &str) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        self.error()
    }

    fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        self.diagnostics.clone()
    }
}

/// One loaded UI extension. Every call is asynchronous and runs off the frame; the renderer never
/// awaits one inside `render()`.
pub trait UiBackend: 'static {
    fn extension_id(&self) -> &str;
    /// The `fyx:ui` version the component targets.
    fn api_version(&self) -> Version {
        Version::new(0, 1, 0)
    }
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
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<Row>>>>;
    /// The tick `request-tick` asked for (0.2). Backends without ticks answer `unchanged`.
    fn tick(&self, _surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        async {
            Ok(Reply {
                effects: Vec::new(),
                value: Update::Unchanged,
            })
        }
        .boxed_local()
    }
    /// The renderer dropped a `ShowSurface` (no window to show the modal in).
    fn modal_closed(&self, _surface_id: &str) {}
    /// Recent calls, `log` output and capability checks, for the Extension Inspector.
    fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        None
    }
}

impl UiBackend for UiExtension {
    fn extension_id(&self) -> &str {
        UiExtension::extension_id(self)
    }

    fn api_version(&self) -> Version {
        UiExtension::api_version(self).clone()
    }

    fn diagnostics(&self) -> Option<ui_ext_wasm::diagnostics::Diagnostics> {
        Some(UiExtension::diagnostics(self))
    }

    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move {
            convert_reply(this.mount_v0_2(&surface_id).await, |nodes| {
                Ok(Tree::from_nodes(nodes, TreeLimits::default())?)
            })
        }
        .boxed_local()
    }

    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { convert_reply(this.handle_event_v0_2(&surface_id, event).await, Ok) }
            .boxed_local()
    }

    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<Row>>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move {
            convert_reply(
                this.list_rows_v0_2(&surface_id, list_node, start, count)
                    .await,
                |rows: Vec<ListRow>| {
                    rows.into_iter()
                        .map(|row| {
                            let indent = row.indent_levels();
                            Ok(Row {
                                index: row.index,
                                indent,
                                tree: Tree::from_row(row.nodes, TreeLimits::default())?,
                            })
                        })
                        .collect()
                },
            )
        }
        .boxed_local()
    }

    fn tick(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        let this = self.clone();
        let surface_id = surface_id.to_string();
        async move { convert_reply(UiExtension::tick(&this, &surface_id).await, Ok) }.boxed_local()
    }

    fn modal_closed(&self, surface_id: &str) {
        UiExtension::modal_closed(self, surface_id)
    }
}
