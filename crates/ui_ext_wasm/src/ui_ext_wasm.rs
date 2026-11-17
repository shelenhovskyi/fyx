//! Host side of the `fyx:ui@0.1.0` world (`crates/ui_ext/wit/fyx-ui.wit`).
//!
//! A UI extension is its own component in its own [`Store`] with its own call queue, so a slow call
//! of the same extension's `zed:extension` component (a download, a language server install) never
//! sits in front of click handling. UI components run on a separate [`Engine`] whose epoch ticks
//! every [`UI_EPOCH_INTERVAL`] instead of Zed's 100 ms, so a UI call yields to the executor that
//! often, and a call that runs past [`UI_CALL_BUDGET`] is interrupted; the trapped instance is
//! then disabled. Epoch callbacks only run while guest code executes, so every call is also raced
//! against a wall-clock deadline that catches a guest blocked in a WASI host call.
//!
//! The renderer never sees wasmtime types: everything crosses as `ui_ext` types.

use std::{
    future::Future,
    ops::RangeInclusive,
    path::{Component as PathComponent, Path},
    pin::{Pin, pin},
    sync::{Arc, Mutex, OnceLock, PoisonError},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow};
use fs::Fs;
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::{BoxFuture, Either},
};
use gpui::{AsyncApp, Task};
use semver::Version;
use serde::Deserialize;
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
        imports: {
            default: trappable,
        },
        exports: {
            default: async,
        },
    });
}

use bindings::fyx::ui::{host as wit_host, types as wit};

/// Compiled code of UI components, kept for the process so a reload of an unchanged component
/// skips most of the compilation. The same scheme as `extension_host`'s cache, sized for UI
/// components.
#[derive(Debug)]
struct IncrementalCompilationCache {
    cache: moka::sync::Cache<Vec<u8>, Vec<u8>>,
}

impl wasmtime::CacheStore for IncrementalCompilationCache {
    fn get(&self, key: &[u8]) -> Option<std::borrow::Cow<'_, [u8]>> {
        self.cache.get(key).map(|value| value.into())
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        self.cache.insert(key.to_vec(), value);
        true
    }
}

fn cache_store() -> Arc<IncrementalCompilationCache> {
    static CACHE_STORE: std::sync::LazyLock<Arc<IncrementalCompilationCache>> =
        std::sync::LazyLock::new(|| {
            Arc::new(IncrementalCompilationCache {
                cache: moka::sync::Cache::builder()
                    .max_capacity(32 * 1024 * 1024)
                    .weigher(|key: &Vec<u8>, value: &Vec<u8>| {
                        (key.len() + value.len()).try_into().unwrap_or(u32::MAX)
                    })
                    .build(),
            })
        });
    CACHE_STORE.clone()
}

/// Three big-endian `u16`s, as in `zed:api-version` (the same parser as `extension_host`'s).
fn parse_version_custom_section(data: &[u8]) -> Option<Version> {
    if data.len() == 6 {
        Some(Version::new(
            u16::from_be_bytes([data[0], data[1]]) as _,
            u16::from_be_bytes([data[2], data[3]]) as _,
            u16::from_be_bytes([data[4], data[5]]) as _,
        ))
    } else {
        None
    }
}

pub const MIN_VERSION: Version = Version::new(0, 1, 0);
pub const MAX_VERSION: Version = Version::new(0, 1, 0);

/// Custom section with the `fyx:ui` version a component targets: three big-endian `u16`s, the same
/// encoding as `zed:api-version`.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";

pub const UI_EPOCH_INTERVAL: Duration = Duration::from_millis(10);
pub const UI_CALL_BUDGET: Duration = Duration::from_millis(200);
/// Instantiation and `surfaces` run once per load and may set up state, so they get more time.
const UI_INIT_BUDGET: Duration = Duration::from_secs(2);
/// On top of a budget before a call is dropped outright. Long enough that a busy guest is stopped
/// by the epoch interrupt first, which leaves a clearer error.
const UI_DEADLINE_SLACK: Duration = Duration::from_millis(50);
/// Pushes one call may make before it traps and the extension is disabled. Pushes are queued
/// unvalidated until the call returns, so this bounds what a looping guest can pile up.
pub const MAX_PUSHES_PER_CALL: usize = 64;
/// Patches plus nodes all of one call's pushes may carry together. Each push is also capped by
/// `TreeLimits` when it is applied, but pushes queue unapplied until the call returns, so their
/// total is capped here: past it the call traps and the extension is disabled.
pub const MAX_PUSHED_ITEMS_PER_CALL: usize = 4096;
/// Linear memory one UI component may grow to. A UI component holds a tree mirror and some state,
/// not data sets. This also bounds every string and list a call can make the host copy out of the
/// guest (arguments of `push-update` and `log`, replies), since they come from that memory.
pub const UI_MEMORY_LIMIT: usize = 32 << 20;
/// Table elements per table (function references; a Rust component uses a few thousand).
pub const UI_TABLE_ELEMENTS_LIMIT: usize = 64 * 1024;
/// Core instances, tables and memories per component (a component and its adapters use a handful).
pub const UI_INSTANCES_LIMIT: usize = 32;
pub const UI_TABLES_LIMIT: usize = 32;
pub const UI_MEMORIES_LIMIT: usize = 8;

/// The `fyx:ui` versions this host can load, on every release channel.
///
/// Unlike `extension_host`'s `wasm_api_version_range`, which caps stable and preview at Zed's v0.6.0
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
            let data = section.data();
            // The linker concatenates equally named custom sections, so a guest that emits the
            // section next to `register_extension!` ends up with a multiple of 6 bytes.
            if data.len() > 6 && data.len() % 6 == 0 {
                anyhow::bail!(
                    "extension {extension_id} has {} copies of the {API_VERSION_SECTION} section \
                     ({} bytes); emit it only via ui_ext_sdk::register_extension!",
                    data.len() / 6,
                    data.len(),
                );
            }
            version = Some(parse_version_custom_section(data).with_context(|| {
                format!(
                    "extension {extension_id} has an invalid {API_VERSION_SECTION} section: {:?}",
                    section.data()
                )
            })?);
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

/// Capability a manifest must declare for each panel surface.
pub const CAPABILITY_PANEL: &str = "ui:panel";
/// Capability a manifest must declare for each status item surface.
pub const CAPABILITY_STATUS_BAR: &str = "ui:status-bar";

/// The `[fyx_ui]` table of an extension's `extension.toml`. Upstream's `ExtensionManifest` ignores
/// unknown tables, so the same file stays installable in Zed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiManifest {
    /// The `fyx:ui` version the component implements.
    pub api_version: Version,
    /// Path of the component inside the extension directory, separate from `extension.wasm`.
    pub component: String,
    /// Capabilities the surfaces need: [`CAPABILITY_PANEL`], [`CAPABILITY_STATUS_BAR`].
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Deserialize)]
struct ExtensionToml {
    id: Arc<str>,
    fyx_ui: Option<UiManifest>,
}

impl UiManifest {
    /// Reads `extension.toml` in `extension_dir`. `None` when it has no `[fyx_ui]` table.
    pub async fn read(fs: &dyn Fs, extension_dir: &Path) -> Result<Option<(Arc<str>, UiManifest)>> {
        let path = extension_dir.join("extension.toml");
        let text = fs
            .load(&path)
            .await
            .with_context(|| format!("reading {path:?}"))?;
        let toml: ExtensionToml =
            toml::from_str(&text).with_context(|| format!("parsing {path:?}"))?;
        let Some(manifest) = toml.fyx_ui else {
            return Ok(None);
        };
        let id = toml.id;
        anyhow::ensure!(
            is_supported_api_version(&manifest.api_version),
            "extension {id} targets fyx:ui {}, this build supports {MIN_VERSION}..={MAX_VERSION}",
            manifest.api_version,
        );
        anyhow::ensure!(
            !manifest.component.is_empty()
                && Path::new(&manifest.component)
                    .components()
                    .all(|component| matches!(component, PathComponent::Normal(_))),
            "extension {id}: [fyx_ui].component must be a relative path inside the extension, got {:?}",
            manifest.component,
        );
        Ok(Some((id, manifest)))
    }

    /// Every surface needs the capability of its kind.
    fn check_surfaces(&self, extension_id: &str, surfaces: &[ui_ext::Surface]) -> Result<()> {
        for surface in surfaces {
            let required = match surface.kind {
                ui_ext::SurfaceKind::Panel(_) => CAPABILITY_PANEL,
                ui_ext::SurfaceKind::StatusItem(_) => CAPABILITY_STATUS_BAR,
            };
            anyhow::ensure!(
                self.capabilities
                    .iter()
                    .any(|capability| capability == required),
                "extension {extension_id}: surface {:?} needs {required:?} in [fyx_ui].capabilities",
                surface.id,
            );
        }
        Ok(())
    }
}

/// A call that ran to completion but whose result was rejected: the extension reported an error, or
/// its reply failed validation. It carries the pushes the call made, in order, because the guest
/// already counts them as applied: apply them before showing the error, so later replies still fit.
///
/// Reach it with `error.downcast_ref::<ReplyError>()` on the `anyhow::Error` a call returns. Errors
/// of any other type (trap, time budget, disabled extension) carry no pushes.
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

/// An update the extension sent through `host.push-update`, for any of its surfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushedUpdate {
    pub surface_id: String,
    pub update: ui_ext::Update,
}

/// The result of a call together with the updates the guest pushed while it ran.
///
/// Apply `pushed` in order, then `value`: the guest emitted them in that order, and a reply may
/// depend on nodes a push inserted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply<T> {
    pub pushed: Vec<PushedUpdate>,
    pub value: T,
}

pub struct UiState {
    extension_id: Arc<str>,
    table: ResourceTable,
    ctx: WasiCtx,
    /// Pushes of the running call, returned with its reply.
    pushes: Vec<PushedUpdate>,
    /// Patches plus nodes in `pushes`.
    pushed_items: usize,
    limits: wasmtime::StoreLimits,
    call_started: Instant,
    call_budget: Duration,
    disabled: Option<String>,
}

impl UiState {
    fn begin_call(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
        self.pushes.clear();
        self.pushed_items = 0;
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
    fn push_update(&mut self, surface_id: String, update: wit::Update) -> wasmtime::Result<()> {
        if self.pushes.len() >= MAX_PUSHES_PER_CALL {
            wasmtime::bail!("pushed more than {MAX_PUSHES_PER_CALL} updates in one call");
        }
        let update: ui_ext::Update = update.into();
        self.pushed_items += match &update {
            ui_ext::Update::Unchanged => 0,
            ui_ext::Update::Replace(nodes) => nodes.len(),
            ui_ext::Update::Patches(patches) => patches.len(),
        };
        if self.pushed_items > MAX_PUSHED_ITEMS_PER_CALL {
            wasmtime::bail!(
                "pushed more than {MAX_PUSHED_ITEMS_PER_CALL} patches and nodes in one call"
            );
        }
        self.pushes.push(PushedUpdate { surface_id, update });
        Ok(())
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) -> wasmtime::Result<()> {
        let level = match level {
            wit_host::LogLevel::Trace => log::Level::Trace,
            wit_host::LogLevel::Debug => log::Level::Debug,
            wit_host::LogLevel::Info => log::Level::Info,
            wit_host::LogLevel::Warn => log::Level::Warn,
            wit_host::LogLevel::Error => log::Level::Error,
        };
        log::log!(target: "fyx_ui", level, "{}: {message}", self.extension_id);
        Ok(())
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

/// Resource caps of every UI component's store. Growing past one traps (rather than failing the
/// guest's allocation quietly), which disables the extension with an explicit error.
fn ui_store_limits() -> wasmtime::StoreLimits {
    wasmtime::StoreLimitsBuilder::new()
        .memory_size(UI_MEMORY_LIMIT)
        .table_elements(UI_TABLE_ELEMENTS_LIMIT)
        .instances(UI_INSTANCES_LIMIT)
        .tables(UI_TABLES_LIMIT)
        .memories(UI_MEMORIES_LIMIT)
        .trap_on_grow_failure(true)
        .build()
}

fn build_ui_runtime() -> Result<UiRuntime> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    config.enable_incremental_compilation(cache_store())?;
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
                wake_deadlines();
            }
        })
        .context("failed to spawn the fyx:ui epoch thread")?;

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    bindings::UiExtension::add_to_linker::<_, UiState>(&mut linker, |state| state)?;
    Ok(UiRuntime { engine, linker })
}

/// Wakers of pending [`Deadline`]s, woken on every epoch tick.
static DEADLINE_WAKERS: Mutex<Vec<Waker>> = Mutex::new(Vec::new());

fn wake_deadlines() {
    let wakers = std::mem::take(
        &mut *DEADLINE_WAKERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    for waker in wakers {
        waker.wake();
    }
}

/// Completes once `Instant::now()` passes the deadline, at the resolution of the epoch thread.
/// It needs no executor timer, so it works on tokio and in GPUI tests alike.
struct Deadline(Instant);

impl Future for Deadline {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if Instant::now() >= self.0 {
            return Poll::Ready(());
        }
        DEADLINE_WAKERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(cx.waker().clone());
        Poll::Pending
    }
}

/// Runs a guest call with `budget`. The epoch callback interrupts a guest that keeps executing;
/// this drops the call when the guest is blocked in a host call instead (`wasi:io/poll` behind
/// `thread::sleep`, for example), where no epoch callback runs.
async fn with_budget<T>(budget: Duration, call: impl Future<Output = Result<T>>) -> Result<T> {
    let deadline = Deadline(Instant::now() + budget + UI_DEADLINE_SLACK);
    match futures::future::select(pin!(call), deadline).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => Err(anyhow!(
            "exceeded its time budget of {budget:?} while blocked in a host call"
        )),
    }
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
    /// Compiles and instantiates a component and asks it for its surfaces.
    ///
    /// Pushes made during a call come back in that call's [`Reply`], or in its [`ReplyError`]. The
    /// receiver yields only the pushes made during instantiation and `surfaces`, which have no reply
    /// to go with; it ends after them.
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
                            pushes: Vec::new(),
                            pushed_items: 0,
                            limits: ui_store_limits(),
                            call_started: Instant::now(),
                            call_budget: UI_INIT_BUDGET,
                            disabled: None,
                        },
                    );
                    store.limiter(|state| &mut state.limits);
                    store.epoch_deadline_callback(|context| {
                        let state = context.data();
                        if state.call_started.elapsed() > state.call_budget {
                            Ok(UpdateDeadline::Interrupt)
                        } else {
                            Ok(UpdateDeadline::Yield(1))
                        }
                    });
                    store.set_epoch_deadline(1);

                    let extension = with_budget(UI_INIT_BUDGET, async {
                        bindings::UiExtension::instantiate_async(
                            &mut store,
                            &component,
                            &runtime.linker,
                        )
                        .await
                        .map_err(anyhow::Error::from)
                    })
                    .await
                    .context("failed to instantiate fyx:ui component")?;
                    store.data_mut().begin_call(UI_INIT_BUDGET);
                    let surfaces = with_budget(UI_INIT_BUDGET, async {
                        extension
                            .call_surfaces(&mut store)
                            .await
                            .map_err(anyhow::Error::from)
                    })
                    .await
                    .context("fyx:ui surfaces() failed")?;
                    let surfaces: Arc<[ui_ext::Surface]> =
                        surfaces.into_iter().map(Into::into).collect();
                    // Nothing replies to instantiation, so its pushes go to the channel; the
                    // receiver is still in hand here, so sending cannot fail.
                    for pushed in store.data_mut().pushes.drain(..) {
                        pushed_tx.unbounded_send(pushed).ok();
                    }
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

    /// Loads the UI component of the extension in `extension_dir`, as its `extension.toml`'s
    /// `[fyx_ui]` table describes. `None` when the extension has no such table.
    pub async fn load_from_dir(
        extension_dir: &Path,
        fs: Arc<dyn Fs>,
        cx: &AsyncApp,
    ) -> Result<Option<(Self, mpsc::UnboundedReceiver<PushedUpdate>)>> {
        let Some((extension_id, manifest)) = UiManifest::read(fs.as_ref(), extension_dir).await?
        else {
            return Ok(None);
        };
        let component_path = extension_dir.join(&manifest.component);
        let wasm_bytes = fs
            .load_bytes(&component_path)
            .await
            .with_context(|| format!("reading {component_path:?}"))?;
        let (extension, pushed) = Self::load(extension_id.clone(), wasm_bytes, cx)
            .await
            .with_context(|| format!("loading fyx:ui extension {extension_id}"))?;
        anyhow::ensure!(
            extension.api_version == manifest.api_version,
            "extension {extension_id}: extension.toml says fyx:ui {}, the component targets {}",
            manifest.api_version,
            extension.api_version,
        );
        manifest.check_surfaces(&extension_id, extension.surfaces())?;
        Ok(Some((extension, pushed)))
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
    pub async fn mount(&self, surface_id: &str) -> Result<Reply<Tree>> {
        let (result, pushed) = self
            .call({
                let surface_id = surface_id.to_string();
                move |extension, store| {
                    async move { extension.call_mount(store, &surface_id).await }.boxed()
                }
            })
            .await?;
        let nodes = self.guest_result(result, &pushed)?;
        let tree = Tree::from_nodes(nodes.into_iter().map(Into::into).collect(), self.limits)
            .with_context(|| format!("invalid tree from mount({surface_id:?})"));
        self.reply(tree, pushed)
    }

    /// The extension's reply to one input event. The caller applies it to its tree, which
    /// validates it.
    pub async fn handle_event(
        &self,
        surface_id: &str,
        event: ui_ext::UiEvent,
    ) -> Result<Reply<ui_ext::Update>> {
        let (result, pushed) = self
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
            .await?;
        let update = self.guest_result(result, &pushed)?;
        self.reply(Ok(update.into()), pushed)
    }

    /// Rows `start..start + count` of a virtualized list, each validated as a single-root tree.
    pub async fn list_rows(
        &self,
        surface_id: &str,
        list_node: ui_ext::NodeId,
        start: u32,
        count: u32,
    ) -> Result<Reply<Vec<(u32, Tree)>>> {
        let (rows, pushed) = self
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
        let rows = rows
            .into_iter()
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
            .collect();
        self.reply(rows, pushed)
    }

    /// Turns an error the guest reported into a [`ReplyError`] carrying the call's pushes.
    fn guest_result<T>(
        &self,
        result: std::result::Result<T, String>,
        pushed: &[PushedUpdate],
    ) -> Result<T> {
        result.map_err(|message| {
            ReplyError {
                pushed: pushed.to_vec(),
                message: format!("from fyx:ui extension {}: {message}", self.extension_id),
            }
            .into()
        })
    }

    /// Pairs a validated value with the call's pushes; a validation error becomes a
    /// [`ReplyError`] carrying them.
    fn reply<T>(&self, value: Result<T>, pushed: Vec<PushedUpdate>) -> Result<Reply<T>> {
        match value {
            Ok(value) => Ok(Reply { pushed, value }),
            Err(error) if error.is::<ReplyError>() => Err(error),
            Err(error) => Err(ReplyError {
                pushed,
                message: format!("fyx:ui extension {}: {error:#}", self.extension_id),
            }
            .into()),
        }
    }

    /// Runs one guest call on the extension's queue and returns its result with the updates the
    /// guest pushed during it. A failed call (trap, budget) disables the extension and drops them.
    async fn call<T, F>(&self, f: F) -> Result<(T, Vec<PushedUpdate>)>
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
                        let result = with_budget(UI_CALL_BUDGET, async {
                            f(extension, store).await.map_err(anyhow::Error::from)
                        })
                        .await;
                        let pushes = std::mem::take(&mut store.data_mut().pushes);
                        // A trap or a dropped call, the time budget included, leaves the instance
                        // unusable.
                        if let Err(error) = &result {
                            store.data_mut().disabled = Some(format!("{error:#}"));
                        }
                        result.map(|value| (value, pushes))
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;
