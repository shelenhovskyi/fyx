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
    conversions::try_collect,
    effects::{EffectReply, EffectReplyError, HostEffect},
    fyx_ui_v0_2, fyx_ui_v0_3, fyx_ui_v0_4,
};

/// The bindings of a component, by the world it targets. The newest version comes first, as in
/// `extension_host`'s `wit::Extension`.
pub(crate) enum UiBindings {
    V0_4(fyx_ui_v0_4::UiExtension),
    V0_3(fyx_ui_v0_3::UiExtension),
    V0_2(fyx_ui_v0_2::UiExtension),
    V0_1(wit_v0_1::UiExtension),
}

/// A guest value the model cannot hold yet (a 0.3 `image` or `canvas` node), as the guest's own
/// error for that call, so it is rejected like a bad reply instead of disabling the extension.
fn guest_value<T>(value: Result<T, model::HostError>) -> Result<T, String> {
    value.map_err(|error| error.message)
}

/// A 0.3 reply update, with its `image` nodes checked against the state after the call
/// ([`crate::effects::ApiState::check_images`]). A bad one is the guest's error, so the renderer
/// resyncs instead of applying it.
fn checked_images(
    store: &Store<UiState>,
    surface_id: &str,
    update: model::Update,
) -> Result<model::Update, String> {
    guest_value(
        store
            .data()
            .api
            .check_images(surface_id, crate::effects::update_props(&update)),
    )
    .map(|()| update)
}

impl UiBindings {
    pub(crate) async fn instantiate(
        version: ApiVersion,
        store: &mut Store<UiState>,
        component: &Component,
        runtime: &UiRuntime,
    ) -> wasmtime::Result<Self> {
        Ok(match version {
            ApiVersion::V0_4_0 => {
                // Every staged 0.4 interface is built; one the WIT stages later traps when
                // called until it is.
                let mut linker = runtime.linker_v0_4.clone();
                let engine = store.engine().clone();
                fyx_ui_v0_3::define_unbuilt_as_traps(
                    fyx_ui_v0_4::LINKED_INTERFACES,
                    &mut linker,
                    component,
                    &engine,
                )?;
                Self::V0_4(
                    fyx_ui_v0_4::UiExtension::instantiate_async(store, component, &linker).await?,
                )
            }
            ApiVersion::V0_3_0 => {
                // The 0.3 interfaces this build does not implement yet trap when called (they are
                // staged with the WIT, plan 4.x); a component that never calls them works.
                let mut linker = runtime.linker_v0_3.clone();
                let engine = store.engine().clone();
                fyx_ui_v0_3::define_unbuilt_as_traps(
                    fyx_ui_v0_3::LINKED_INTERFACES,
                    &mut linker,
                    component,
                    &engine,
                )?;
                Self::V0_3(
                    fyx_ui_v0_3::UiExtension::instantiate_async(store, component, &linker).await?,
                )
            }
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
        let surfaces = match self {
            Self::V0_4(extension) => try_collect(extension.call_surfaces(store).await?),
            Self::V0_3(extension) => try_collect(extension.call_surfaces(store).await?),
            Self::V0_2(extension) => try_collect(extension.call_surfaces(store).await?),
            Self::V0_1(extension) => {
                return Ok(extension
                    .call_surfaces(store)
                    .await?
                    .into_iter()
                    .map(|surface| ui_ext::Surface::from(surface).into())
                    .collect());
            }
        };
        match surfaces {
            Ok(surfaces) => Ok(surfaces),
            Err(error) => wasmtime::bail!("{}", error.message),
        }
    }

    async fn call_mount(
        &self,
        store: &mut Store<UiState>,
        surface_id: &str,
    ) -> wasmtime::Result<Result<Vec<model::Node>, String>> {
        Ok(match self {
            Self::V0_4(extension) => extension
                .call_mount(&mut *store, surface_id)
                .await?
                .and_then(|nodes| guest_value(try_collect::<_, model::Node>(nodes)))
                .and_then(|nodes| {
                    guest_value(
                        store
                            .data()
                            .api
                            .check_images(surface_id, nodes.iter().map(|node| &node.props)),
                    )
                    .map(|()| nodes)
                }),
            Self::V0_3(extension) => extension
                .call_mount(&mut *store, surface_id)
                .await?
                .and_then(|nodes| guest_value(try_collect::<_, model::Node>(nodes)))
                .and_then(|nodes| {
                    guest_value(
                        store
                            .data()
                            .api
                            .check_images(surface_id, nodes.iter().map(|node| &node.props)),
                    )
                    .map(|()| nodes)
                }),
            Self::V0_2(extension) => extension
                .call_mount(store, surface_id)
                .await?
                .and_then(|nodes| guest_value(try_collect(nodes))),
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
            Self::V0_4(extension) => extension
                .call_handle_event(&mut *store, surface_id, &event.into())
                .await?
                .and_then(|update| guest_value(update.try_into()))
                .and_then(|update| checked_images(store, surface_id, update)),
            // A 0.2 surface has no canvas, so the host never sends it `canvas-click`: defence in
            // depth, like 0.1's below.
            Self::V0_3(extension) => extension
                .call_handle_event(&mut *store, surface_id, &event.into())
                .await?
                .and_then(|update| guest_value(update.try_into()))
                .and_then(|update| checked_images(store, surface_id, update)),
            // A 0.2 surface has no canvas, so the host never sends it `canvas-click`: defence in
            // depth, like 0.1's below.
            Self::V0_2(_) if matches!(event, model::UiEvent::CanvasClick(_)) => {
                Ok(model::Update::Unchanged)
            }
            Self::V0_2(extension) => extension
                .call_handle_event(store, surface_id, &event.into())
                .await?
                .and_then(|update| guest_value(update.try_into())),
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
    ) -> wasmtime::Result<Result<Vec<model::ListRow>, String>> {
        Ok(match self {
            Self::V0_4(extension) => guest_value(try_collect::<_, model::ListRow>(
                extension
                    .call_list_rows(&mut *store, surface_id, list_node, start, count)
                    .await?,
            ))
            .and_then(|rows| {
                guest_value(
                    store.data().api.check_images(
                        surface_id,
                        rows.iter()
                            .flat_map(|row| row.nodes.iter().map(|node| &node.props)),
                    ),
                )
                .map(|()| rows)
            }),
            Self::V0_3(extension) => guest_value(try_collect::<_, model::ListRow>(
                extension
                    .call_list_rows(&mut *store, surface_id, list_node, start, count)
                    .await?,
            ))
            .and_then(|rows| {
                guest_value(
                    store.data().api.check_images(
                        surface_id,
                        rows.iter()
                            .flat_map(|row| row.nodes.iter().map(|node| &node.props)),
                    ),
                )
                .map(|()| rows)
            }),
            Self::V0_2(extension) => guest_value(try_collect(
                extension
                    .call_list_rows(store, surface_id, list_node, start, count)
                    .await?,
            )),
            Self::V0_1(extension) => Ok(extension
                .call_list_rows(store, surface_id, list_node, start, count)
                .await?
                .into_iter()
                .map(|row| {
                    model::ListRow::from(ui_ext::ListRow {
                        index: row.index,
                        nodes: row.nodes.into_iter().map(Into::into).collect(),
                    })
                })
                .collect()),
        })
    }

    /// Only a 0.3 component starts work outside a call, so only it gets `handle-io`.
    async fn call_handle_io(
        &self,
        store: &mut Store<UiState>,
        event: crate::network::IoEvent,
    ) -> wasmtime::Result<Result<(), String>> {
        Ok(match self {
            Self::V0_4(extension) => match fyx_ui_v0_4::io_event(event) {
                Some(event) => extension.call_handle_io(store, &event).await?,
                None => Ok(()),
            },
            Self::V0_3(extension) => match fyx_ui_v0_3::io_event(event) {
                Some(event) => extension.call_handle_io(store, &event).await?,
                None => Ok(()),
            },
            Self::V0_2(_) | Self::V0_1(_) => Ok(()),
        })
    }

    /// A 0.1 component cannot request ticks, so it is never ticked; if it were, nothing changes.
    async fn call_tick(
        &self,
        store: &mut Store<UiState>,
        surface_id: &str,
    ) -> wasmtime::Result<Result<model::Update, String>> {
        Ok(match self {
            Self::V0_4(extension) => extension
                .call_tick(&mut *store, surface_id)
                .await?
                .and_then(|update| guest_value(update.try_into()))
                .and_then(|update| checked_images(store, surface_id, update)),
            Self::V0_3(extension) => extension
                .call_tick(&mut *store, surface_id)
                .await?
                .and_then(|update| guest_value(update.try_into()))
                .and_then(|update| checked_images(store, surface_id, update)),
            Self::V0_2(extension) => extension
                .call_tick(store, surface_id)
                .await?
                .and_then(|update| guest_value(update.try_into())),
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
        let (nodes, effects) = self.mount_checked(surface_id).await?;
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
            value: rows,
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

    /// The outcomes of this instance's work outside a call (0.3 `network.fetch`; design.md §0.3),
    /// once: the caller hands each to [`UiExtension::handle_io`], in order. At most
    /// [`crate::network::MAX_QUEUED_IO_EVENTS`] wait; past that, finished requests are held.
    pub fn take_io_events(
        &self,
    ) -> Option<futures::channel::mpsc::Receiver<crate::network::IoEvent>> {
        self.network.take_events()
    }

    /// Delivers one outcome through the 0.3 `handle-io` export, queued behind calls already
    /// queued; its effects come back like any call's. A request whose capability was revoked
    /// while it ran is delivered as `denied`.
    pub async fn handle_io(&self, event: crate::network::IoEvent) -> Result<EffectReply<()>> {
        let network = self.network.clone();
        let (result, effects) = self
            .call(
                "handle-io",
                "",
                CallBudget::Regular,
                move |extension, store| {
                    async move {
                        let event = match event {
                            crate::network::IoEvent::FetchDone(id, result) => {
                                store.data_mut().network.delivered(id);
                                let result = if network.granted() {
                                    result
                                } else {
                                    Err(crate::network::FetchError::Denied)
                                };
                                crate::network::IoEvent::FetchDone(id, result)
                            }
                            crate::network::IoEvent::ProcessExited(exit) => {
                                store.data_mut().process.delivered(exit.process);
                                crate::network::IoEvent::ProcessExited(exit)
                            }
                            crate::network::IoEvent::SpawnFailed(id, error) => {
                                store.data_mut().process.delivered(id);
                                crate::network::IoEvent::SpawnFailed(id, error)
                            }
                            // An edit that was applied stays applied, so its outcome is not
                            // turned into `denied` by a later revocation.
                            crate::network::IoEvent::EditDone(id, result) => {
                                store.data_mut().edit.delivered(id);
                                crate::network::IoEvent::EditDone(id, result)
                            }
                            crate::network::IoEvent::Terminal(event) => {
                                store.data_mut().terminals.delivered(&event);
                                crate::network::IoEvent::Terminal(event)
                            }
                            // A second answer, or one for a card never asked, is dropped.
                            crate::network::IoEvent::ApprovalAnswered(id, answer) => {
                                if !store.data_mut().api.approval_answered(id) {
                                    return Ok(Ok(()));
                                }
                                crate::network::IoEvent::ApprovalAnswered(id, answer)
                            }
                            other => other,
                        };
                        extension.call_handle_io(store, event).await
                    }
                    .boxed()
                },
            )
            .await?;
        self.guest_result(result, &effects)?;
        Ok(EffectReply { effects, value: () })
    }

    /// The user answered approval card `id` ([`HostEffect::AskApproval`]), or it closed
    /// unanswered (`None`: the window closed, the extension was disabled). The answer reaches the
    /// guest through the io channel, like any outcome, so [`Self::take_io_events`]' consumer
    /// delivers it with [`Self::handle_io`]. Only the first answer of an open card is delivered.
    pub async fn answer_approval(
        &self,
        id: u64,
        answer: Option<ui_ext::approvals::ApprovalOption>,
    ) {
        self.network
            .deliver(crate::network::IoEvent::ApprovalAnswered(id, answer))
            .await;
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

    /// Tells the host that the renderer dropped decoration `id` on its own, because its tree
    /// failed to apply (integration-5 M2): it stops counting toward the 512 live decorations, and
    /// its notifications and progresses are forgotten, as after `remove-decoration`. The guest
    /// gets no event; a later `place-decoration` with the id places it again.
    pub fn decoration_closed(&self, id: &str) {
        let id = id.to_string();
        self.tx
            .unbounded_send(Box::new(move |_, store| {
                async move { store.data_mut().api.decoration_closed(&id) }.boxed()
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
    ) -> Result<(Vec<model::Node>, Vec<HostEffect>)> {
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
        // Validated as a 0.2 tree. Only the 0.1 API builds the 0.1 copy: on the 0.2 path it was
        // a second tree, built on the main thread and dropped, on every mount.
        match model::tree::Tree::from_nodes(nodes.clone(), self.limits)
            .with_context(|| format!("invalid tree from mount({surface_id:?})"))
        {
            Ok(_) => Ok((nodes, effects)),
            Err(error) => Err(self.rejected(error, effects)),
        }
    }

    async fn list_rows_checked(
        &self,
        surface_id: &str,
        list_node: model::NodeId,
        start: u32,
        count: u32,
    ) -> Result<(Vec<model::ListRow>, Vec<HostEffect>)> {
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
        let rows = self.guest_result(rows, &effects)?;
        let requested = start..start.saturating_add(count);
        // Each row validated as a 0.2 tree; the 0.1 copies are built by the 0.1 API only, not
        // for every visible row on every scroll.
        let checked = rows.iter().try_for_each(|row| {
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
            Ok(())
        });
        match checked {
            Ok(()) => Ok((rows, effects)),
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
        let (nodes, effects) = self.mount_checked(surface_id).await.map_err(shim::error)?;
        let tree = Tree::from_nodes(nodes.iter().map(shim::node).collect(), self.limits)
            .with_context(|| format!("invalid tree from mount({surface_id:?})"));
        match tree {
            Ok(tree) => Ok(Reply {
                pushed: shim::pushes(effects),
                value: tree,
            }),
            Err(error) => Err(shim::error(self.rejected(error, effects))),
        }
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
        let trees = rows
            .iter()
            .map(|row| {
                Tree::from_row(row.nodes.iter().map(shim::node).collect(), self.limits)
                    .map(|tree| (row.index, tree))
                    .with_context(|| {
                        format!(
                            "invalid row {} from list-rows({surface_id:?}, {list_node})",
                            row.index
                        )
                    })
            })
            .collect::<Result<Vec<_>>>();
        match trees {
            Ok(trees) => Ok(Reply {
                pushed: shim::pushes(effects),
                value: trees,
            }),
            Err(error) => Err(shim::error(self.rejected(error, effects))),
        }
    }
}

/// The 0.2 model seen through the 0.1 types the renderer uses until plan 2.3. Lossless for a 0.1
/// component, whose data came from the 0.1 types. For a 0.2 component (loaded only in dev builds)
/// it drops what 0.1 cannot express: tooltips, `secret`, `selected`, `indent`, `invalidate-rows`,
/// modals and every effect but pushes; frozen icons become their snake_case name, which is
/// upstream's name for most of them; a 0.3 image becomes its alt text.
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
            // 0.3 only; 0.1 has no image, so its alt text stands in.
            model::Props::Image(props) => ui_ext::Props::Text(ui_ext::TextProps {
                content: props.alt.clone(),
                ..ui_ext::TextProps::default()
            }),
            // 0.3 only; so does a canvas's description.
            model::Props::Canvas(props) => ui_ext::Props::Text(ui_ext::TextProps {
                content: props.description.clone(),
                ..ui_ext::TextProps::default()
            }),
            // 0.4 only; a terminal has no text to stand in, so it is named.
            model::Props::Terminal(props) => ui_ext::Props::Text(ui_ext::TextProps {
                content: format!("[terminal {}]", props.handle),
                ..ui_ext::TextProps::default()
            }),
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
