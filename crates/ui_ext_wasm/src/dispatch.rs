//! One host for both worlds: [`UiBindings`] holds a component's bindings of its own version, and
//! every call goes through it in `ui_ext::v0_2` types, the latest model, which 0.1 data converts
//! into without loss. The `*_v0_2` methods of [`UiExtension`] are the API the renderer moves to in
//! platform plan 2.3; the 0.1-typed `mount`, `handle_event` and `list_rows` it uses today are thin
//! shims over them, lossless for 0.1 components.

use anyhow::{Context as _, Result};
use futures::FutureExt as _;
use ui_ext::{ApiVersion, Tree, v0_2 as model};
use wasmtime::{Store, component::Component};

use crate::{
    CallBudget, PushedUpdate, Reply, ReplyError, UiExtension, UiRuntime, UiState,
    bindings as wit_v0_1,
    effects::{EffectReply, EffectReplyError, HostEffect},
    fyx_ui_v0_2,
};

/// The bindings of a component, by the world it targets. The newest version comes first, as in
/// `extension_host`'s `wit::Extension`.
pub(crate) enum UiBindings {
    V0_2(fyx_ui_v0_2::UiExtension),
    V0_1(wit_v0_1::UiExtension),
}

impl UiBindings {
    pub(crate) async fn instantiate(
        version: ApiVersion,
        store: &mut Store<UiState>,
        component: &Component,
        runtime: &UiRuntime,
    ) -> wasmtime::Result<Self> {
        Ok(match version {
            ApiVersion::V0_2_0 => Self::V0_2(
                fyx_ui_v0_2::UiExtension::instantiate_async(store, component, &runtime.linker_v0_2)
                    .await?,
            ),
            ApiVersion::V0_1_0 => Self::V0_1(
                wit_v0_1::UiExtension::instantiate_async(store, component, &runtime.linker).await?,
            ),
        })
    }

    pub(crate) async fn call_surfaces(
        &self,
        store: &mut Store<UiState>,
    ) -> wasmtime::Result<Vec<model::Surface>> {
        Ok(match self {
            Self::V0_2(extension) => extension
                .call_surfaces(store)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
            Self::V0_1(extension) => extension
                .call_surfaces(store)
                .await?
                .into_iter()
                .map(|surface| ui_ext::Surface::from(surface).into())
                .collect(),
        })
    }

    async fn call_mount(
        &self,
        store: &mut Store<UiState>,
        surface_id: &str,
    ) -> wasmtime::Result<Result<Vec<model::Node>, String>> {
        Ok(match self {
            Self::V0_2(extension) => extension
                .call_mount(store, surface_id)
                .await?
                .map(|nodes| nodes.into_iter().map(Into::into).collect()),
            Self::V0_1(extension) => extension.call_mount(store, surface_id).await?.map(|nodes| {
                nodes
                    .into_iter()
                    .map(|node| ui_ext::Node::from(node).into())
                    .collect()
            }),
        })
    }

    async fn call_handle_event(
        &self,
        store: &mut Store<UiState>,
        surface_id: &str,
        event: model::UiEvent,
    ) -> wasmtime::Result<Result<model::Update, String>> {
        Ok(match self {
            Self::V0_2(extension) => extension
                .call_handle_event(store, surface_id, &event.into())
                .await?
                .map(Into::into),
            Self::V0_1(extension) => {
                // The host never produces 0.2-only events for a 0.1 surface (0.1 has no modals),
                // so this is defence in depth: the event is dropped, not sent.
                let event = match ui_ext::UiEvent::try_from(event) {
                    Ok(event) => event,
                    Err(error) => {
                        log::debug!(
                            "fyx:ui: dropped an event for 0.1 surface {surface_id:?}: {error}"
                        );
                        return Ok(Ok(model::Update::Unchanged));
                    }
                };
                extension
                    .call_handle_event(store, surface_id, &event.into())
                    .await?
                    .map(|update| ui_ext::Update::from(update).into())
            }
        })
    }

    async fn call_list_rows(
        &self,
        store: &mut Store<UiState>,
        surface_id: &str,
        list_node: model::NodeId,
        start: u32,
        count: u32,
    ) -> wasmtime::Result<Vec<model::ListRow>> {
        Ok(match self {
            Self::V0_2(extension) => extension
                .call_list_rows(store, surface_id, list_node, start, count)
                .await?
                .into_iter()
                .map(Into::into)
                .collect(),
            Self::V0_1(extension) => extension
                .call_list_rows(store, surface_id, list_node, start, count)
                .await?
                .into_iter()
                .map(|row| {
                    model::ListRow::from(ui_ext::ListRow {
                        index: row.index,
                        nodes: row.nodes.into_iter().map(Into::into).collect(),
                    })
                })
                .collect(),
        })
    }

    /// A 0.1 component cannot request ticks, so it is never ticked; if it were, nothing changes.
    async fn call_tick(
        &self,
        store: &mut Store<UiState>,
        surface_id: &str,
    ) -> wasmtime::Result<Result<model::Update, String>> {
        Ok(match self {
            Self::V0_2(extension) => extension
                .call_tick(store, surface_id)
                .await?
                .map(Into::into),
            Self::V0_1(_) => Ok(model::Update::Unchanged),
        })
    }
}

impl UiState {
    /// The effects of the call that just returned, in call order: a 0.1 component's pushes or a
    /// 0.2 component's effect list (one of them is always empty).
    pub(crate) fn finish_call(&mut self) -> Vec<HostEffect> {
        let pushes = self.pushes.drain(..).map(|pushed| HostEffect::Push {
            surface_id: pushed.surface_id,
            update: pushed.update.into(),
        });
        let mut effects: Vec<HostEffect> = pushes.collect();
        effects.extend(self.api.finish_call());
        effects
    }
}

impl UiExtension {
    /// The world the component targets.
    pub fn ui_api_version(&self) -> ApiVersion {
        self.ui_api_version
    }

    /// The `[fyx_ui].capabilities` the component was loaded with; every capability when it was
    /// loaded without a manifest.
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// What `surfaces` returned right after instantiation, as the 0.2 model. Surfaces added with
    /// `add-surface` during `surfaces` are in [`UiExtension::initial_effects`].
    pub fn surfaces_v0_2(&self) -> &[model::Surface] {
        &self.surfaces_v0_2
    }

    /// The effects of instantiation and `surfaces`, which have no reply to go with. Apply them in
    /// order after registering [`UiExtension::surfaces_v0_2`].
    pub fn initial_effects(&self) -> &[HostEffect] {
        &self.initial_effects
    }

    /// The whole tree of a surface or decoration, validated as a tree (at most 16384 nodes,
    /// parents first, children only under `flex`).
    pub async fn mount_v0_2(&self, surface_id: &str) -> Result<EffectReply<Vec<model::Node>>> {
        let (nodes, _tree, effects) = self.mount_checked(surface_id).await?;
        Ok(EffectReply {
            effects,
            value: nodes,
        })
    }

    /// The extension's reply to one input event; the caller applies it to its tree, which
    /// validates it. The reply to `dismissed` is always `unchanged`: the modal has no tree any
    /// more, so anything else is dropped here with a debug log (its effects still apply).
    pub async fn handle_event_v0_2(
        &self,
        surface_id: &str,
        event: model::UiEvent,
    ) -> Result<EffectReply<model::Update>> {
        let dismissed = matches!(event, model::UiEvent::Dismissed(_));
        let (result, effects) = self
            .call("handle-event", surface_id, CallBudget::Regular, {
                let surface_id = surface_id.to_string();
                move |extension, store| {
                    async move {
                        store.data_mut().api.begin_event(&surface_id, &event);
                        extension.call_handle_event(store, &surface_id, event).await
                    }
                    .boxed()
                }
            })
            .await?;
        let mut update = self.guest_result(result, &effects)?;
        if dismissed && update != model::Update::Unchanged {
            log::debug!(
                "fyx:ui extension {}: ignored a reply to dismissed({surface_id:?}) other than unchanged",
                self.extension_id
            );
            update = model::Update::Unchanged;
        }
        Ok(EffectReply {
            effects,
            value: update,
        })
    }

    /// Rows `start..start + count` of a virtualized list, each validated as a single-root tree.
    /// `indent` is passed on as the extension sent it; [`model::ListRow::indent_levels`] is what
    /// to draw.
    pub async fn list_rows_v0_2(
        &self,
        surface_id: &str,
        list_node: model::NodeId,
        start: u32,
        count: u32,
    ) -> Result<EffectReply<Vec<model::ListRow>>> {
        let (rows, effects) = self
            .list_rows_checked(surface_id, list_node, start, count)
            .await?;
        Ok(EffectReply {
            effects,
            value: rows.into_iter().map(|(row, _tree)| row).collect(),
        })
    }

    /// The tick `request-tick` asked for. The update applies to `surface_id`, like an event's.
    /// A 0.1 component is never ticked: it answers `unchanged` without a call into it.
    pub async fn tick(&self, surface_id: &str) -> Result<EffectReply<model::Update>> {
        let (result, effects) = self
            .call("tick", surface_id, CallBudget::Regular, {
                let surface_id = surface_id.to_string();
                move |extension, store| {
                    async move { extension.call_tick(store, &surface_id).await }.boxed()
                }
            })
            .await?;
        let update = self.guest_result(result, &effects)?;
        Ok(EffectReply {
            effects,
            value: update,
        })
    }

    /// Tells the host that a modal is no longer shown although no `dismissed` event or
    /// `hide-surface` said so: the renderer dropped its `ShowSurface` effect (no workspace window).
    /// Without it, the next `show-surface` of that modal would do nothing.
    pub fn modal_closed(&self, surface_id: &str) {
        let surface_id = surface_id.to_string();
        self.tx
            .unbounded_send(Box::new(move |_, store| {
                async move { store.data_mut().api.modal_closed(&surface_id) }.boxed()
            }))
            .ok();
    }

    #[cfg(test)]
    pub(crate) async fn with_state<T: 'static + Send>(
        &self,
        f: impl 'static + Send + FnOnce(&mut UiState) -> T,
    ) -> T {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.tx
            .unbounded_send(Box::new(move |_, store| {
                async move {
                    tx.send(f(store.data_mut())).ok();
                }
                .boxed()
            }))
            .ok();
        rx.await.expect("the queue runs")
    }

    async fn mount_checked(
        &self,
        surface_id: &str,
    ) -> Result<(Vec<model::Node>, Tree, Vec<HostEffect>)> {
        let (result, effects) = self
            .call(
                "mount",
                surface_id,
                CallBudget::Mount(surface_id.to_string()),
                {
                    let surface_id = surface_id.to_string();
                    move |extension, store| {
                        async move { extension.call_mount(store, &surface_id).await }.boxed()
                    }
                },
            )
            .await?;
        let nodes = self.guest_result(result, &effects)?;
        // Validated as a 0.2 tree; the 0.1 copy is for the 0.1 shims, until plan 2.3 drops them.
        let tree = model::tree::Tree::from_nodes(nodes.clone(), self.limits)
            .map_err(anyhow::Error::from)
            .and_then(|_| {
                Ok(Tree::from_nodes(
                    nodes.iter().map(shim::node).collect(),
                    self.limits,
                )?)
            })
            .with_context(|| format!("invalid tree from mount({surface_id:?})"));
        match tree {
            Ok(tree) => Ok((nodes, tree, effects)),
            Err(error) => Err(self.rejected(error, effects)),
        }
    }

    async fn list_rows_checked(
        &self,
        surface_id: &str,
        list_node: model::NodeId,
        start: u32,
        count: u32,
    ) -> Result<(Vec<(model::ListRow, Tree)>, Vec<HostEffect>)> {
        let (rows, effects) = self
            .call("list-rows", surface_id, CallBudget::Regular, {
                let surface_id = surface_id.to_string();
                move |extension, store| {
                    async move {
                        extension
                            .call_list_rows(store, &surface_id, list_node, start, count)
                            .await
                    }
                    .boxed()
                }
            })
            .await?;
        let requested = start..start.saturating_add(count);
        let rows = rows
            .into_iter()
            .map(|row| {
                anyhow::ensure!(
                    requested.contains(&row.index),
                    "list-rows({surface_id:?}, {list_node}) returned row {} outside {requested:?}",
                    row.index,
                );
                model::tree::Tree::from_row(row.nodes.clone(), self.limits).with_context(|| {
                    format!(
                        "invalid row {} from list-rows({surface_id:?}, {list_node})",
                        row.index
                    )
                })?;
                let tree = Tree::from_row(row.nodes.iter().map(shim::node).collect(), self.limits)
                    .with_context(|| {
                        format!(
                            "invalid row {} from list-rows({surface_id:?}, {list_node})",
                            row.index
                        )
                    })?;
                Ok((row, tree))
            })
            .collect::<Result<Vec<_>>>();
        match rows {
            Ok(rows) => Ok((rows, effects)),
            Err(error) => Err(self.rejected(error, effects)),
        }
    }

    /// Turns an error the guest reported into an [`EffectReplyError`] carrying the call's effects.
    fn guest_result<T>(&self, result: Result<T, String>, effects: &[HostEffect]) -> Result<T> {
        result.map_err(|message| {
            EffectReplyError {
                effects: effects.to_vec(),
                message: format!("from fyx:ui extension {}: {message}", self.extension_id),
            }
            .into()
        })
    }

    /// A reply that failed validation, as an [`EffectReplyError`] carrying the call's effects.
    fn rejected(&self, error: anyhow::Error, effects: Vec<HostEffect>) -> anyhow::Error {
        EffectReplyError {
            effects,
            message: format!("fyx:ui extension {}: {error:#}", self.extension_id),
        }
        .into()
    }

    /// The initial tree of a surface, validated (0.1 model).
    pub async fn mount(&self, surface_id: &str) -> Result<Reply<Tree>> {
        let (_nodes, tree, effects) = self.mount_checked(surface_id).await.map_err(shim::error)?;
        Ok(Reply {
            pushed: shim::pushes(effects),
            value: tree,
        })
    }

    /// The extension's reply to one input event (0.1 model). The caller applies it to its tree,
    /// which validates it.
    pub async fn handle_event(
        &self,
        surface_id: &str,
        event: ui_ext::UiEvent,
    ) -> Result<Reply<ui_ext::Update>> {
        let reply = self
            .handle_event_v0_2(surface_id, event.into())
            .await
            .map_err(shim::error)?;
        Ok(Reply {
            pushed: shim::pushes(reply.effects),
            value: shim::update(reply.value),
        })
    }

    /// Rows `start..start + count` of a virtualized list, each validated as a single-root tree
    /// (0.1 model).
    pub async fn list_rows(
        &self,
        surface_id: &str,
        list_node: ui_ext::NodeId,
        start: u32,
        count: u32,
    ) -> Result<Reply<Vec<(u32, Tree)>>> {
        let (rows, effects) = self
            .list_rows_checked(surface_id, list_node, start, count)
            .await
            .map_err(shim::error)?;
        Ok(Reply {
            pushed: shim::pushes(effects),
            value: rows
                .into_iter()
                .map(|(row, tree)| (row.index, tree))
                .collect(),
        })
    }
}

/// The 0.2 model seen through the 0.1 types the renderer uses until plan 2.3. Lossless for a 0.1
/// component, whose data came from the 0.1 types. For a 0.2 component (loaded only in dev builds)
/// it drops what 0.1 cannot express: tooltips, `secret`, `selected`, `indent`, `invalidate-rows`,
/// modals and every effect but pushes; frozen icons become their snake_case name, which is
/// upstream's name for most of them.
pub(crate) mod shim {
    use super::*;

    fn icon(icon: &model::IconRef) -> String {
        match icon {
            model::IconRef::Frozen(icon) => icon.name().replace('-', "_"),
            model::IconRef::Upstream(name) => name.clone(),
        }
    }

    fn props(props: &model::Props) -> ui_ext::Props {
        match props {
            model::Props::Flex(props) => ui_ext::Props::Flex(props.clone()),
            model::Props::Text(props) => ui_ext::Props::Text(props.clone()),
            model::Props::Button(props) => ui_ext::Props::Button(ui_ext::ButtonProps {
                label: props.label.clone(),
                icon: props.icon.as_ref().map(icon),
                style: props.style,
                disabled: props.disabled,
            }),
            model::Props::Input(props) => ui_ext::Props::Input(ui_ext::InputProps {
                value: props.value.clone(),
                placeholder: props.placeholder.clone(),
            }),
            model::Props::List(props) => ui_ext::Props::List(ui_ext::ListProps {
                row_count: props.row_count,
                row_height: props.row_height,
            }),
            model::Props::Table(props) => ui_ext::Props::Table(props.clone()),
            model::Props::Icon(props) => ui_ext::Props::Icon(ui_ext::IconProps {
                name: icon(&props.name),
                color: props.color,
            }),
            model::Props::Markdown(text) => ui_ext::Props::Markdown(text.clone()),
            model::Props::DiffView(props) => ui_ext::Props::DiffView(props.clone()),
        }
    }

    pub(crate) fn node(node: &model::Node) -> ui_ext::Node {
        ui_ext::Node {
            id: node.id,
            parent: node.parent,
            props: props(&node.props),
        }
    }

    pub(crate) fn update(update: model::Update) -> ui_ext::Update {
        match update {
            model::Update::Unchanged => ui_ext::Update::Unchanged,
            model::Update::Replace(nodes) => {
                ui_ext::Update::Replace(nodes.iter().map(node).collect())
            }
            model::Update::Patches(patches) => ui_ext::Update::Patches(
                patches
                    .into_iter()
                    .filter_map(|patch| match patch {
                        model::Patch::SetProps(id, new) => {
                            Some(ui_ext::Patch::SetProps(id, props(&new)))
                        }
                        model::Patch::Insert(op) => Some(ui_ext::Patch::Insert(ui_ext::InsertOp {
                            node: node(&op.node),
                            before: op.before,
                        })),
                        model::Patch::Remove(id) => Some(ui_ext::Patch::Remove(id)),
                        model::Patch::InvalidateRows(_) => None,
                    })
                    .collect(),
            ),
        }
    }

    pub(crate) fn surface(surface: &model::Surface) -> Option<ui_ext::Surface> {
        let kind = match &surface.kind {
            model::SurfaceKind::Panel(panel) => {
                ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
                    position: panel.position,
                    icon: icon(&panel.icon),
                    default_size: panel.default_size,
                })
            }
            model::SurfaceKind::StatusItem(item) => ui_ext::SurfaceKind::StatusItem(item.clone()),
            model::SurfaceKind::Modal(_) => return None,
        };
        Some(ui_ext::Surface {
            id: surface.id.clone(),
            title: surface.title.clone(),
            kind,
        })
    }

    pub(crate) fn pushes(effects: Vec<HostEffect>) -> Vec<PushedUpdate> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                HostEffect::Push { surface_id, update } => Some(PushedUpdate {
                    surface_id,
                    update: self::update(update),
                }),
                other => {
                    log::debug!("fyx:ui: the 0.1 renderer drops {other:?}");
                    None
                }
            })
            .collect()
    }

    /// An [`EffectReplyError`] as the [`ReplyError`] the 0.1 renderer looks for.
    pub(crate) fn error(error: anyhow::Error) -> anyhow::Error {
        match error.downcast::<EffectReplyError>() {
            Ok(EffectReplyError { effects, message }) => ReplyError {
                pushed: pushes(effects),
                message,
            }
            .into(),
            Err(error) => error,
        }
    }
}
