//! Host side of the `fyx:ui@0.1.0` world (`crates/ui_ext/wit/fyx-ui.wit`).
//!
//! A UI extension is its own component in its own [`Store`] with its own call queue, so a slow call
//! of the same extension's `zed:extension` component (a download, a language server install) never
//! sits in front of click handling. UI components run on a separate [`Engine`] whose epoch ticks
//! every [`UI_EPOCH_INTERVAL`] instead of Zed's 100 ms, so a UI call yields to the executor that
//! often, and a call that runs past [`UI_CALL_BUDGET`] is interrupted; the trapped instance is
//! then disabled.
//!
//! The renderer never sees wasmtime types: everything crosses as `ui_ext` types.

use std::{
    ops::RangeInclusive,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::BoxFuture,
};
use gpui::{AsyncApp, Task};
use semver::Version;
use ui_ext::{Tree, TreeLimits};
use wasmtime::{
    Engine, Store, UpdateDeadline,
    component::{Component, HasData, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit",
        world: "ui-extension",
        exports: {
            default: async,
        },
    });
}

use bindings::fyx::ui::{host as wit_host, types as wit};

pub const MIN_VERSION: Version = Version::new(0, 1, 0);
pub const MAX_VERSION: Version = Version::new(0, 1, 0);

/// Custom section with the `fyx:ui` version a component targets: three big-endian `u16`s, the same
/// encoding as `zed:api-version`.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";

pub const UI_EPOCH_INTERVAL: Duration = Duration::from_millis(10);
pub const UI_CALL_BUDGET: Duration = Duration::from_millis(200);
/// Instantiation and `surfaces` run once per load and may set up state, so they get more time.
const UI_INIT_BUDGET: Duration = Duration::from_secs(2);

/// The `fyx:ui` versions this host can load, on every release channel.
///
/// Unlike [`super::wit::wasm_api_version_range`], which caps stable and preview at Zed's v0.6.0
/// while it stages its own extension API, `fyx:ui` is versioned by its WIT package alone.
pub fn api_version_range() -> RangeInclusive<Version> {
    MIN_VERSION..=MAX_VERSION
}

pub fn is_supported_api_version(version: &Version) -> bool {
    api_version_range().contains(version)
}

/// Reads the [`API_VERSION_SECTION`], if the component has one.
pub fn parse_api_version(extension_id: &str, wasm_bytes: &[u8]) -> Result<Option<Version>> {
    let mut version = None;
    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        if let wasmparser::Payload::CustomSection(section) =
            payload.context("error parsing fyx:ui extension")?
            && section.name() == API_VERSION_SECTION
        {
            version = Some(
                super::parse_wasm_extension_version_custom_section(section.data())
                    .with_context(|| {
                        format!(
                            "extension {extension_id} has an invalid {API_VERSION_SECTION} section: {:?}",
                            section.data()
                        )
                    })?,
            );
        }
    }
    Ok(version)
}

/// Fallback for components built without the SDK: the version of an imported `fyx:ui` interface.
/// A component that never calls the host may import none, so this is not enough on its own.
fn api_version_from_imports(component: &Component, engine: &Engine) -> Option<Version> {
    component
        .component_type()
        .imports(engine)
        .find_map(|(name, _)| {
            let version = name
                .strip_prefix("fyx:ui/host@")
                .or_else(|| name.strip_prefix("fyx:ui/types@"))?;
            Version::parse(version).ok()
        })
}

/// An update the extension pushed on its own (timers, background work), not as the reply to an
/// event. The renderer coalesces these and applies them on its next frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedUpdate {
    pub surface_id: String,
    pub update: ui_ext::Update,
}

pub struct UiState {
    extension_id: Arc<str>,
    table: ResourceTable,
    ctx: WasiCtx,
    pushed_updates: mpsc::UnboundedSender<PushedUpdate>,
    call_started: Instant,
    call_budget: Duration,
    disabled: Option<String>,
}

impl UiState {
    fn begin_call(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
    }
}

impl HasData for UiState {
    type Data<'a> = &'a mut UiState;
}

impl WasiView for UiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wit::Host for UiState {}

impl wit_host::Host for UiState {
    fn push_update(&mut self, surface_id: String, update: wit::Update) {
        let pushed = PushedUpdate {
            surface_id,
            update: update.into(),
        };
        if self.pushed_updates.unbounded_send(pushed).is_err() {
            log::debug!(
                "fyx:ui extension {}: dropped a pushed update, nothing listens for it",
                self.extension_id
            );
        }
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) {
        let level = match level {
            wit_host::LogLevel::Trace => log::Level::Trace,
            wit_host::LogLevel::Debug => log::Level::Debug,
            wit_host::LogLevel::Info => log::Level::Info,
            wit_host::LogLevel::Warn => log::Level::Warn,
            wit_host::LogLevel::Error => log::Level::Error,
        };
        log::log!(target: "fyx_ui", level, "{}: {message}", self.extension_id);
    }
}

struct UiRuntime {
    engine: Engine,
    linker: Linker<UiState>,
}

fn ui_runtime() -> Result<&'static UiRuntime> {
    static RUNTIME: OnceLock<Result<UiRuntime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| build_ui_runtime().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow!("failed to set up the fyx:ui wasm runtime: {error}"))
}

fn build_ui_runtime() -> Result<UiRuntime> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    config.enable_incremental_compilation(super::cache_store())?;
    let engine = Engine::new(&config)?;

    // A plain thread rather than an executor timer: the tick has to keep coming while executor
    // threads are busy, which is exactly when a runaway UI call needs interrupting.
    let weak_engine = engine.weak();
    std::thread::Builder::new()
        .name("fyx-ui-epoch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(UI_EPOCH_INTERVAL);
                let Some(engine) = weak_engine.upgrade() else {
                    break;
                };
                engine.increment_epoch();
            }
        })
        .context("failed to spawn the fyx:ui epoch thread")?;

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    bindings::UiExtension::add_to_linker::<_, UiState>(&mut linker, |state| state)?;
    Ok(UiRuntime { engine, linker })
}

type UiCall = Box<
    dyn Send
        + for<'a> FnOnce(&'a mut bindings::UiExtension, &'a mut Store<UiState>) -> BoxFuture<'a, ()>,
>;

/// A loaded `fyx:ui` component. Calls are serialized on its own queue.
#[derive(Clone)]
pub struct UiExtension {
    extension_id: Arc<str>,
    api_version: Version,
    surfaces: Arc<[ui_ext::Surface]>,
    limits: TreeLimits,
    tx: mpsc::UnboundedSender<UiCall>,
    _task: Arc<Task<Result<(), gpui_tokio::JoinError>>>,
}

impl UiExtension {
    /// Compiles and instantiates a component and asks it for its surfaces. The receiver yields the
    /// updates the extension pushes through `host.push-update`.
    pub fn load(
        extension_id: Arc<str>,
        wasm_bytes: Vec<u8>,
        cx: &AsyncApp,
    ) -> Task<Result<(Self, mpsc::UnboundedReceiver<PushedUpdate>)>> {
        let compile_task = cx.background_executor().spawn({
            let extension_id = extension_id.clone();
            async move {
                let runtime = ui_runtime()?;
                let section_version = parse_api_version(&extension_id, &wasm_bytes)?;
                let component = Component::from_binary(&runtime.engine, &wasm_bytes)
                    .map_err(anyhow::Error::from)
                    .context("failed to compile fyx:ui component")?;
                let api_version = section_version
                    .or_else(|| api_version_from_imports(&component, &runtime.engine))
                    .with_context(|| {
                        format!(
                            "extension {extension_id} is not a fyx:ui component: it has no \
                             {API_VERSION_SECTION} section and imports no fyx:ui interface"
                        )
                    })?;
                anyhow::ensure!(
                    is_supported_api_version(&api_version),
                    "extension {extension_id} targets fyx:ui {api_version}, this build supports {}..={}",
                    MIN_VERSION,
                    MAX_VERSION,
                );
                anyhow::Ok((api_version, component))
            }
        });

        cx.spawn(async move |cx| {
            let (api_version, component) = compile_task.await?;
            let (pushed_tx, pushed_rx) = mpsc::unbounded();

            let instantiate = {
                let extension_id = extension_id.clone();
                async move {
                    let runtime = ui_runtime()?;
                    let mut wasi = WasiCtxBuilder::new();
                    wasi.inherit_stderr();
                    let mut store = Store::new(
                        &runtime.engine,
                        UiState {
                            extension_id,
                            table: ResourceTable::new(),
                            ctx: wasi.build(),
                            pushed_updates: pushed_tx,
                            call_started: Instant::now(),
                            call_budget: UI_INIT_BUDGET,
                            disabled: None,
                        },
                    );
                    store.epoch_deadline_callback(|context| {
                        let state = context.data();
                        if state.call_started.elapsed() > state.call_budget {
                            Ok(UpdateDeadline::Interrupt)
                        } else {
                            Ok(UpdateDeadline::Yield(1))
                        }
                    });
                    store.set_epoch_deadline(1);

                    let extension = bindings::UiExtension::instantiate_async(
                        &mut store,
                        &component,
                        &runtime.linker,
                    )
                    .await
                    .map_err(anyhow::Error::from)
                    .context("failed to instantiate fyx:ui component")?;
                    store.data_mut().begin_call(UI_INIT_BUDGET);
                    let surfaces = extension
                        .call_surfaces(&mut store)
                        .await
                        .map_err(anyhow::Error::from)
                        .context("fyx:ui surfaces() failed")?;
                    let surfaces: Arc<[ui_ext::Surface]> =
                        surfaces.into_iter().map(Into::into).collect();
                    anyhow::Ok((extension, store, surfaces))
                }
            };
            let (mut extension, mut store, surfaces) =
                gpui_tokio::Tokio::spawn(cx, instantiate).await??;

            let (tx, mut rx) = mpsc::unbounded::<UiCall>();
            // Its own task on tokio: WASI calls need a tokio runtime, and this queue is shared
            // with nothing else.
            let task = gpui_tokio::Tokio::spawn(cx, async move {
                while let Some(call) = rx.next().await {
                    call(&mut extension, &mut store).await;
                }
            });

            Ok((
                Self {
                    extension_id,
                    api_version,
                    surfaces,
                    limits: TreeLimits::default(),
                    tx,
                    _task: Arc::new(task),
                },
                pushed_rx,
            ))
        })
    }

    pub fn extension_id(&self) -> &Arc<str> {
        &self.extension_id
    }

    pub fn api_version(&self) -> &Version {
        &self.api_version
    }

    /// What `surfaces` returned right after instantiation.
    pub fn surfaces(&self) -> &[ui_ext::Surface] {
        &self.surfaces
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    /// The initial tree of a surface, validated.
    pub async fn mount(&self, surface_id: &str) -> Result<Tree> {
        let nodes = self
            .call({
                let surface_id = surface_id.to_string();
                move |extension, store| {
                    async move { extension.call_mount(store, &surface_id).await }.boxed()
                }
            })
            .await?
            .map_err(|message| self.extension_error(message))?;
        Tree::from_nodes(nodes.into_iter().map(Into::into).collect(), self.limits)
            .with_context(|| format!("invalid tree from mount({surface_id:?})"))
    }

    /// The extension's reply to one input event. The caller applies it to its tree, which
    /// validates it.
    pub async fn handle_event(
        &self,
        surface_id: &str,
        event: ui_ext::UiEvent,
    ) -> Result<ui_ext::Update> {
        let update = self
            .call({
                let surface_id = surface_id.to_string();
                let event = wit::UiEvent::from(event);
                move |extension, store| {
                    async move {
                        extension
                            .call_handle_event(store, &surface_id, &event)
                            .await
                    }
                    .boxed()
                }
            })
            .await?
            .map_err(|message| self.extension_error(message))?;
        Ok(update.into())
    }

    /// Rows `start..start + count` of a virtualized list, each validated as a single-root tree.
    pub async fn list_rows(
        &self,
        surface_id: &str,
        list_node: ui_ext::NodeId,
        start: u32,
        count: u32,
    ) -> Result<Vec<(u32, Tree)>> {
        let rows = self
            .call({
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
        rows.into_iter()
            .map(|row| {
                anyhow::ensure!(
                    requested.contains(&row.index),
                    "list-rows({surface_id:?}, {list_node}) returned row {} outside {requested:?}",
                    row.index,
                );
                let nodes = row.nodes.into_iter().map(Into::into).collect();
                let tree = Tree::from_row(nodes, self.limits).with_context(|| {
                    format!(
                        "invalid row {} from list-rows({surface_id:?}, {list_node})",
                        row.index
                    )
                })?;
                Ok((row.index, tree))
            })
            .collect()
    }

    async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: 'static + Send,
        F: 'static
            + Send
            + for<'a> FnOnce(
                &'a mut bindings::UiExtension,
                &'a mut Store<UiState>,
            ) -> BoxFuture<'a, wasmtime::Result<T>>,
    {
        let (return_tx, return_rx) = oneshot::channel();
        self.tx
            .unbounded_send(Box::new(move |extension, store| {
                async move {
                    let result = if let Some(reason) = &store.data().disabled {
                        Err(anyhow!("disabled after an earlier failure: {reason}"))
                    } else {
                        store.data_mut().begin_call(UI_CALL_BUDGET);
                        store.set_epoch_deadline(1);
                        let result = f(extension, store).await.map_err(anyhow::Error::from);
                        // A trap, the time budget included, leaves the instance unusable.
                        if let Err(error) = &result {
                            store.data_mut().disabled = Some(format!("{error:#}"));
                        }
                        result
                    };
                    // The caller may have stopped waiting; then nobody needs the result.
                    return_tx.send(result).ok();
                }
                .boxed()
            }))
            .map_err(|_| anyhow!("fyx:ui extension {} has shut down", self.extension_id))?;
        return_rx
            .await
            .with_context(|| format!("fyx:ui extension {} dropped a call", self.extension_id))?
            .with_context(|| format!("fyx:ui extension {}", self.extension_id))
    }

    fn extension_error(&self, message: String) -> anyhow::Error {
        anyhow!("from fyx:ui extension {}: {message}", self.extension_id)
    }
}

impl From<wit::ColorToken> for ui_ext::ColorToken {
    fn from(value: wit::ColorToken) -> Self {
        match value {
            wit::ColorToken::Text => Self::Text,
            wit::ColorToken::TextMuted => Self::TextMuted,
            wit::ColorToken::TextAccent => Self::TextAccent,
            wit::ColorToken::Error => Self::Error,
            wit::ColorToken::Warning => Self::Warning,
            wit::ColorToken::Success => Self::Success,
            wit::ColorToken::Border => Self::Border,
            wit::ColorToken::Surface => Self::Surface,
            wit::ColorToken::ElevatedSurface => Self::ElevatedSurface,
        }
    }
}

impl From<wit::Props> for ui_ext::Props {
    fn from(value: wit::Props) -> Self {
        match value {
            wit::Props::Flex(props) => Self::Flex(ui_ext::FlexProps {
                direction: match props.direction {
                    wit::FlexDirection::Row => ui_ext::FlexDirection::Row,
                    wit::FlexDirection::Column => ui_ext::FlexDirection::Column,
                },
                gap: props.gap,
                padding: props.padding,
                grow: props.grow,
                align: match props.align {
                    wit::Align::Start => ui_ext::Align::Start,
                    wit::Align::Center => ui_ext::Align::Center,
                    wit::Align::End => ui_ext::Align::End,
                    wit::Align::Stretch => ui_ext::Align::Stretch,
                },
                background: props.background.map(Into::into),
            }),
            wit::Props::Text(props) => Self::Text(ui_ext::TextProps {
                content: props.content,
                color: props.color.map(Into::into),
                bold: props.bold,
                monospace: props.monospace,
            }),
            wit::Props::Button(props) => Self::Button(ui_ext::ButtonProps {
                label: props.label,
                icon: props.icon,
                style: match props.style {
                    wit::ButtonStyle::Default => ui_ext::ButtonStyle::Default,
                    wit::ButtonStyle::Filled => ui_ext::ButtonStyle::Filled,
                    wit::ButtonStyle::Subtle => ui_ext::ButtonStyle::Subtle,
                    wit::ButtonStyle::Danger => ui_ext::ButtonStyle::Danger,
                },
                disabled: props.disabled,
            }),
            wit::Props::Input(props) => Self::Input(ui_ext::InputProps {
                value: props.value,
                placeholder: props.placeholder,
            }),
            wit::Props::List(props) => Self::List(ui_ext::ListProps {
                row_count: props.row_count,
                row_height: props.row_height,
            }),
            wit::Props::Table(props) => Self::Table(ui_ext::TableProps {
                columns: props.columns,
                row_count: props.row_count,
            }),
            wit::Props::Icon(props) => Self::Icon(ui_ext::IconProps {
                name: props.name,
                color: props.color.map(Into::into),
            }),
            wit::Props::Markdown(text) => Self::Markdown(text),
            wit::Props::DiffView(props) => Self::DiffView(ui_ext::DiffViewProps {
                old_text: props.old_text,
                new_text: props.new_text,
                language: props.language,
            }),
        }
    }
}

impl From<wit::Node> for ui_ext::Node {
    fn from(value: wit::Node) -> Self {
        Self {
            id: value.id,
            parent: value.parent,
            props: value.props.into(),
        }
    }
}

impl From<wit::Patch> for ui_ext::Patch {
    fn from(value: wit::Patch) -> Self {
        match value {
            wit::Patch::SetProps((id, props)) => Self::SetProps(id, props.into()),
            wit::Patch::Insert(op) => Self::Insert(ui_ext::InsertOp {
                node: op.node.into(),
                before: op.before,
            }),
            wit::Patch::Remove(id) => Self::Remove(id),
        }
    }
}

impl From<wit::Update> for ui_ext::Update {
    fn from(value: wit::Update) -> Self {
        match value {
            wit::Update::Unchanged => Self::Unchanged,
            wit::Update::Replace(nodes) => {
                Self::Replace(nodes.into_iter().map(Into::into).collect())
            }
            wit::Update::Patches(patches) => {
                Self::Patches(patches.into_iter().map(Into::into).collect())
            }
        }
    }
}

impl From<wit::Surface> for ui_ext::Surface {
    fn from(value: wit::Surface) -> Self {
        Self {
            id: value.id,
            title: value.title,
            kind: match value.kind {
                wit::SurfaceKind::Panel(panel) => {
                    ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
                        position: match panel.position {
                            wit::DockPosition::Left => ui_ext::DockPosition::Left,
                            wit::DockPosition::Right => ui_ext::DockPosition::Right,
                            wit::DockPosition::Bottom => ui_ext::DockPosition::Bottom,
                        },
                        icon: panel.icon,
                        default_size: panel.default_size,
                    })
                }
                wit::SurfaceKind::StatusItem(item) => {
                    ui_ext::SurfaceKind::StatusItem(ui_ext::StatusItemDescriptor {
                        side: match item.side {
                            wit::StatusBarSide::Left => ui_ext::StatusBarSide::Left,
                            wit::StatusBarSide::Right => ui_ext::StatusBarSide::Right,
                        },
                        priority: item.priority,
                    })
                }
            },
        }
    }
}

impl From<ui_ext::UiEvent> for wit::UiEvent {
    fn from(value: ui_ext::UiEvent) -> Self {
        match value {
            ui_ext::UiEvent::Click(id) => Self::Click(id),
            ui_ext::UiEvent::InputChanged(id, text) => Self::InputChanged((id, text)),
            ui_ext::UiEvent::InputSubmitted(id, text) => Self::InputSubmitted((id, text)),
            ui_ext::UiEvent::RowClick(id, row) => Self::RowClick((id, row)),
        }
    }
}

#[cfg(test)]
mod tests;
