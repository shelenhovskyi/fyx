//! The calls the renderer makes into an extension, behind a trait so the renderer can be tested
//! without compiling a WASM component.
//!
//! The renderer works on the 0.2 model (`ui_ext::v0_2`) for every extension; a 0.1 component's
//! results are converted here (`From` impls of the 0.2 model), so the renderer never sees 0.1
//! types. What differs by version is decided from [`UiBackend::api_version`] (migration.md §3).

use anyhow::Result;
use futures::{FutureExt as _, future::LocalBoxFuture};
use semver::Version;
use ui_ext::v0_2::{NodeId, UiEvent, Update, tree::Tree};
use ui_ext_wasm::UiExtension;

/// An update the extension pushed during a call, for any of its surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedUpdate {
    pub surface_id: String,
    pub update: Update,
}

/// The result of a call together with the updates the guest pushed while it ran. Apply `pushed`
/// in order, then `value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply<T> {
    pub pushed: Vec<PushedUpdate>,
    pub value: T,
}

/// A call that ran but whose result was rejected (the extension's own error, or a reply that
/// failed validation), with the pushes it made, which still apply. Reach it with
/// `error.downcast_ref::<ReplyError>()`; other errors (trap, budget, disabled) carry no pushes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyError {
    pub pushed: Vec<PushedUpdate>,
    pub message: String,
}

impl std::fmt::Display for ReplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReplyError {}

impl From<ui_ext_wasm::PushedUpdate> for PushedUpdate {
    fn from(pushed: ui_ext_wasm::PushedUpdate) -> Self {
        Self {
            surface_id: pushed.surface_id,
            update: pushed.update.into(),
        }
    }
}

fn convert_pushes(pushed: Vec<ui_ext_wasm::PushedUpdate>) -> Vec<PushedUpdate> {
    pushed.into_iter().map(Into::into).collect()
}

/// A 0.1 host error, with its pushes converted; the context chain is kept.
fn convert_error(error: anyhow::Error) -> anyhow::Error {
    match error.downcast_ref::<ui_ext_wasm::ReplyError>() {
        Some(reply_error) => anyhow::Error::new(ReplyError {
            pushed: convert_pushes(reply_error.pushed.clone()),
            message: reply_error.message.clone(),
        }),
        None => error,
    }
}

fn convert_reply<T, U>(
    result: Result<ui_ext_wasm::Reply<T>>,
    value: impl FnOnce(T) -> Result<U>,
) -> Result<Reply<U>> {
    let reply = result.map_err(convert_error)?;
    let pushed = convert_pushes(reply.pushed);
    match value(reply.value) {
        Ok(value) => Ok(Reply { pushed, value }),
        Err(error) => Err(anyhow::Error::new(ReplyError {
            pushed,
            message: format!("{error:#}"),
        })),
    }
}

/// A 0.1 tree in the 0.2 model, revalidated (the rules are the same).
fn convert_tree(tree: ui_ext::Tree, row: bool) -> Result<Tree> {
    let limits = tree.limits();
    let nodes = tree.to_nodes().into_iter().map(Into::into).collect();
    Ok(if row {
        Tree::from_row(nodes, limits)?
    } else {
        Tree::from_nodes(nodes, limits)?
    })
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
    /// Rows with their indent level (0.2's `list-row.indent`, clamped; 0 for 0.1).
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
            convert_reply(UiExtension::mount(&this, &surface_id).await, |tree| {
                convert_tree(tree, false)
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
        async move {
            // A 0.1 component has no `row-selected` or `dismissed`; nothing to send.
            let Ok(event) = ui_ext::UiEvent::try_from(event) else {
                return Ok(Reply {
                    pushed: Vec::new(),
                    value: Update::Unchanged,
                });
            };
            convert_reply(
                UiExtension::handle_event(&this, &surface_id, event).await,
                |update| Ok(update.into()),
            )
        }
        .boxed_local()
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
        async move {
            convert_reply(
                UiExtension::list_rows(&this, &surface_id, list_node, start, count).await,
                |rows| {
                    rows.into_iter()
                        .map(|(index, tree)| Ok((index, convert_tree(tree, true)?)))
                        .collect()
                },
            )
        }
        .boxed_local()
    }
}
