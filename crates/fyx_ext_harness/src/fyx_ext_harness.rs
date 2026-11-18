//! Runs a Fyx UI extension component in wasmtime without the editor, so extension tests can mount
//! surfaces, send events, run ticks and assert the trees headless (platform spec decision 10,
//! plan tasks 1.8 and 2.4; the spec's `fyx_sdk::testing`).
//!
//! Apache-2.0, like the SDK, so extensions can use it as a dev-dependency, and host-only, so the
//! SDK itself never pulls in wasmtime.
//!
//! It runs components of `fyx:ui` 0.1 and 0.2. Internally every value is the 0.2 model
//! (`ui_ext::v0_2`), into which 0.1 data converts without loss. The `*_v0_2` methods and
//! [`Outcome`] show the 0.2 model; the older methods show the 0.1 model, which is lossless for a
//! 0.1 component and drops tooltips, `secret`, `selected`, `indent`, `invalidate-rows`, modals and
//! every effect but pushes for a 0.2 one, as the host's 0.1 view does.
//!
//! What it shares with Fyx and what it mirrors:
//! - Trees, patches, id rules, caps and capability checks are `ui_ext`'s, the same model and
//!   validation the host uses (`ui_ext::Tree`, `ui_ext::LiveIds`).
//! - The call contract follows the host runtime (`ui_ext_wasm`): WASI 0.2 with only stderr, 2 s for
//!   instantiation and `surfaces()`, 200 ms per call, an instance that traps or overruns is
//!   disabled.
//!   - 0.1: at most 64 pushes per call (the 65th traps).
//!   - 0.2: every host import is checked when it is called and its effect applied after the call,
//!     in order, before the call's own value, also when the call returns `Err` (design.md, A).
//!     Pushes past the caps return `limit-exceeded`; a modal needs a click or Enter to show.
//!   - Rows must lie in the requested range.
//!
//!   A conformance test in `ui_ext_wasm` runs the same guests through both and fails when they
//!   drift.
//! - What the renderer does, as far as a test can see it:
//!   - typed text, and a `row-selected` list selection, are written into the tree before the
//!     event is sent;
//!   - a modal mounts when the extension shows it and loses its tree when it closes;
//!   - a push or placed decoration whose tree does not fit remounts that surface or decoration,
//!     and a decoration whose remount fails is removed with a `warn` log;
//!   - a pending tick runs when the test calls [`Harness::tick`]. There is no clock: the delay is
//!     only recorded. After 3 consecutive `tick` errors a surface's ticks stop until the next
//!     event on it.
//!
//!   Static surfaces and surfaces from `add-surface` mount when the test calls
//!   [`Harness::mount`].
//!
//! One difference: a guest blocked inside a WASI call (such as `thread::sleep`) is only stopped
//! when it returns to WebAssembly, while the host's async runtime stops waiting at the deadline.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use semver::Version;
use ui_ext::v0_2 as model;
use ui_ext::{ApiVersion, NodeId, Props, Tree, TreeLimits, TreeNode};
use wasmtime::component::{Component, HasData, Linker, ResourceTable};
use wasmtime::{Engine, Store, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod api_state;
mod v0_2;

pub use api_state::{
    ACTIVATION_WINDOW, HostEffect, MAX_PUSHED_ITEMS_PER_CALL_V0_2, MAX_TICK_DELAY, MIN_TICK_DELAY,
};

use api_state::ApiState;

mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
    });
}

use bindings::fyx::ui::{host as wit_host, types as wit};

/// The `fyx:ui` versions the harness runs, the same as the host's supported range.
pub const SUPPORTED_API_VERSIONS: &[Version] = &[Version::new(0, 1, 0), Version::new(0, 2, 0)];
/// The section `register_extension!` writes: three big-endian `u16`s.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";
/// Instantiation and `surfaces()` together.
pub const INIT_BUDGET: Duration = Duration::from_secs(2);
/// Every later call.
pub const CALL_BUDGET: Duration = Duration::from_millis(200);
pub const EPOCH_INTERVAL: Duration = Duration::from_millis(10);
pub const MAX_PUSHES_PER_CALL: usize = 64;
/// Patches plus nodes all of one call's 0.1 pushes may carry; past it the call traps.
pub const MAX_PUSHED_ITEMS_PER_CALL: usize = 4096;
/// String bytes all of one call's host-import arguments may carry together (`push-update`,
/// `log`, and in 0.2 every other import); past it the call traps.
pub const MAX_LIFTED_TEXT_PER_CALL: usize = 16 << 20;
/// Longest `log` message kept; the rest is cut off. It still counts in full toward
/// [`MAX_LIFTED_TEXT_PER_CALL`].
pub const MAX_LOG_MESSAGE_LEN: usize = 16 << 10;
/// Linear memory one component may grow to, and its other store caps: the host's
/// (`ui_ext_wasm::UI_*_LIMIT`). Growing past one traps, which disables the extension.
pub const MEMORY_LIMIT: usize = 256 << 20;
pub const TABLE_ELEMENTS_LIMIT: usize = 64 * 1024;
pub const INSTANCES_LIMIT: usize = 32;
pub const TABLES_LIMIT: usize = 32;
pub const MEMORIES_LIMIT: usize = 8;
/// Consecutive `tick` errors after which a surface's ticks stop until an event on it.
pub const MAX_CONSECUTIVE_TICK_ERRORS: u32 = 3;
/// `fyx-ext test` sets it to the built component; [`Harness::from_env`] reads it.
pub const COMPONENT_VARIABLE: &str = "FYX_EXT_COMPONENT";
/// What [`Harness::load`] grants: every `[fyx_ui].capabilities` entry a component can use.
pub const ALL_CAPABILITIES: &[&str] = &[
    model::CAPABILITY_PANEL,
    model::CAPABILITY_STATUS_BAR,
    model::CAPABILITY_MODAL,
    model::CAPABILITY_EDITOR_DECORATIONS,
];

/// An update the extension pushed during a call, in the 0.1 model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    pub surface_id: String,
    pub update: ui_ext::Update,
}

/// One export call as the host receives it, in the 0.1 model: the updates pushed during the call,
/// and the guest's own result, whose `Err` is the message the guest returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawReply<T> {
    pub pushed: Vec<Pushed>,
    pub value: std::result::Result<T, String>,
}

/// One export call as the host receives it: the effects of the call, applied, and the guest's own
/// result, not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEffects<T> {
    pub effects: Vec<HostEffect>,
    pub value: std::result::Result<T, String>,
}

/// What one event did, in the 0.1 model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOutcome {
    /// Pushed during the call, applied first, in order.
    pub pushed: Vec<Pushed>,
    /// The call's own reply, applied to the event's surface after the pushes.
    pub update: ui_ext::Update,
    /// Node ids the reply touched in the event's surface (see `ui_ext::Tree::apply_update`).
    pub touched: BTreeSet<NodeId>,
}

/// What one event or tick did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The call's effects, applied first, in order.
    pub effects: Vec<HostEffect>,
    /// The call's own reply, applied to the surface after the effects. For `dismissed` it is not
    /// applied: the modal has no tree any more.
    pub update: model::Update,
    /// Node ids the reply touched, including the targets of `invalidate-rows`.
    pub touched: BTreeSet<NodeId>,
    /// The reply's `invalidate-rows` patches, in order.
    pub invalidated: Vec<model::InvalidateRowsOp>,
}

pub(crate) struct State {
    table: ResourceTable,
    ctx: WasiCtx,
    /// 0.1 pushes of the running call.
    pushes: Vec<Pushed>,
    /// Patches and nodes of the running call's 0.1 pushes.
    pushed_items: usize,
    /// String bytes the running call passed to host imports.
    lifted_text: usize,
    /// 0.2 checks and effects.
    api: ApiState,
    logs: Vec<(ui_ext::LogLevel, String)>,
    call_started: Instant,
    call_budget: Duration,
    limits: wasmtime::StoreLimits,
}

/// The host's store caps (`ui_ext_wasm`'s `ui_store_limits`), so a guest that grows too far
/// fails here as it fails in Fyx (security-2 T9).
fn store_limits() -> wasmtime::StoreLimits {
    wasmtime::StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .table_elements(TABLE_ELEMENTS_LIMIT)
        .instances(INSTANCES_LIMIT)
        .tables(TABLES_LIMIT)
        .memories(MEMORIES_LIMIT)
        .trap_on_grow_failure(true)
        .build()
}

impl State {
    fn begin_call(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
        self.pushes.clear();
        self.pushed_items = 0;
        self.lifted_text = 0;
        self.api.begin_call();
    }

    /// The host's per-call text cap (`ui_ext_wasm`'s `count_lifted_text`).
    pub(crate) fn count_lifted_text(&mut self, bytes: usize) -> wasmtime::Result<()> {
        self.lifted_text = self.lifted_text.saturating_add(bytes);
        if self.lifted_text > MAX_LIFTED_TEXT_PER_CALL {
            wasmtime::bail!(
                "passed more than {MAX_LIFTED_TEXT_PER_CALL} bytes of text to push-update and log \
                 in one call"
            );
        }
        Ok(())
    }

    /// `log` of either world: counted, cut at [`MAX_LOG_MESSAGE_LEN`], kept at once.
    pub(crate) fn write_log(
        &mut self,
        level: ui_ext::LogLevel,
        mut message: String,
    ) -> wasmtime::Result<()> {
        self.count_lifted_text(message.len())?;
        if message.len() > MAX_LOG_MESSAGE_LEN {
            let mut end = MAX_LOG_MESSAGE_LEN;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            let total = message.len();
            message.truncate(end);
            message.push_str(&format!("… ({total} bytes, cut)"));
        }
        self.logs.push((level, message));
        Ok(())
    }

    /// The effects of the call that just returned: a 0.1 component's pushes or a 0.2
    /// component's effect list (one of them is always empty).
    fn finish_call(&mut self) -> Vec<HostEffect> {
        let mut effects = self
            .pushes
            .drain(..)
            .map(|pushed| HostEffect::Push {
                surface_id: pushed.surface_id,
                update: pushed.update.into(),
            })
            .collect::<Vec<_>>();
        effects.extend(self.api.finish_call());
        effects
    }
}

impl HasData for State {
    type Data<'a> = &'a mut State;
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wit::Host for State {}

impl wit_host::Host for State {
    fn push_update(&mut self, surface_id: String, update: wit::Update) -> wasmtime::Result<()> {
        if self.pushes.len() >= MAX_PUSHES_PER_CALL {
            wasmtime::bail!("pushed more than {MAX_PUSHES_PER_CALL} updates in one call");
        }
        let update: ui_ext::Update = update.into();
        self.count_lifted_text(
            surface_id.len() + update_text_len(&model::Update::from(update.clone())),
        )?;
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
        self.pushes.push(Pushed { surface_id, update });
        Ok(())
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) -> wasmtime::Result<()> {
        self.write_log(level.into(), message)
    }
}

/// String bytes of a node's props, as the host counts them: frozen icons carry none.
pub(crate) fn props_text_len(props: &model::Props) -> usize {
    let option_len = |text: &Option<String>| text.as_ref().map_or(0, String::len);
    let icon_len = |icon: &model::IconRef| match icon {
        model::IconRef::Frozen(_) => 0,
        model::IconRef::Upstream(name) => name.len(),
    };
    match props {
        model::Props::Flex(_) | model::Props::List(_) => 0,
        model::Props::Text(props) => props.content.len(),
        model::Props::Button(props) => {
            props.label.len() + props.icon.as_ref().map_or(0, icon_len) + option_len(&props.tooltip)
        }
        model::Props::Input(props) => props.value.len() + props.placeholder.len(),
        model::Props::Table(props) => props.columns.iter().map(String::len).sum(),
        model::Props::Icon(props) => icon_len(&props.name),
        model::Props::Markdown(text) => text.len(),
        model::Props::DiffView(props) => {
            props.old_text.len() + props.new_text.len() + option_len(&props.language)
        }
        // W0 integration stopgap until plan 4.12 (W4): only 0.3 components produce images, and
        // the harness runs 0.1 and 0.2 components.
        model::Props::Image(_) => 0,
    }
}

/// String bytes an update carries.
pub(crate) fn update_text_len(update: &model::Update) -> usize {
    match update {
        model::Update::Unchanged => 0,
        model::Update::Replace(nodes) => nodes.iter().map(|node| props_text_len(&node.props)).sum(),
        model::Update::Patches(patches) => patches
            .iter()
            .map(|patch| match patch {
                model::Patch::SetProps(_, props) => props_text_len(props),
                model::Patch::Insert(insert) => props_text_len(&insert.node.props),
                model::Patch::Remove(_) | model::Patch::InvalidateRows(_) => 0,
            })
            .sum(),
    }
}

struct Runtime {
    engine: Engine,
    linker_v0_1: Linker<State>,
    linker_v0_2: Linker<State>,
}

fn runtime() -> Result<&'static Runtime> {
    static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| build_runtime().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("setting up wasmtime: {error}"))
}

fn build_runtime() -> Result<Runtime> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;
    let weak_engine = engine.weak();
    std::thread::Builder::new()
        .name("fyx-ext-harness-epoch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(EPOCH_INTERVAL);
                let Some(engine) = weak_engine.upgrade() else {
                    break;
                };
                engine.increment_epoch();
            }
        })
        .context("starting the epoch thread")?;
    let mut linker_v0_1 = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker_v0_1)?;
    bindings::UiExtension::add_to_linker::<_, State>(&mut linker_v0_1, |state| state)?;
    let mut linker_v0_2 = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker_v0_2)?;
    v0_2::UiExtension::add_to_linker::<_, State>(&mut linker_v0_2, |state| state)?;
    Ok(Runtime {
        engine,
        linker_v0_1,
        linker_v0_2,
    })
}

/// A component's bindings, by the world it targets. Every call goes through it in the 0.2 model.
enum Bindings {
    V0_1(bindings::UiExtension),
    V0_2(v0_2::UiExtension),
}

impl Bindings {
    fn surfaces(&self, store: &mut Store<State>) -> wasmtime::Result<Vec<model::Surface>> {
        Ok(match self {
            Self::V0_1(extension) => extension
                .call_surfaces(store)?
                .into_iter()
                .map(|surface| ui_ext::Surface::from(surface).into())
                .collect(),
            Self::V0_2(extension) => extension
                .call_surfaces(store)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    fn mount(
        &self,
        store: &mut Store<State>,
        surface_id: &str,
    ) -> wasmtime::Result<std::result::Result<Vec<model::Node>, String>> {
        Ok(match self {
            Self::V0_1(extension) => extension.call_mount(store, surface_id)?.map(|nodes| {
                nodes
                    .into_iter()
                    .map(|node| ui_ext::Node::from(node).into())
                    .collect()
            }),
            Self::V0_2(extension) => extension
                .call_mount(store, surface_id)?
                .map(|nodes| nodes.into_iter().map(Into::into).collect()),
        })
    }

    fn handle_event(
        &self,
        store: &mut Store<State>,
        surface_id: &str,
        event: model::UiEvent,
    ) -> wasmtime::Result<std::result::Result<model::Update, String>> {
        Ok(match self {
            Self::V0_1(extension) => {
                // A 0.1 surface never gets a 0.2-only event; like the host, drop it.
                let Ok(event) = ui_ext::UiEvent::try_from(event) else {
                    return Ok(Ok(model::Update::Unchanged));
                };
                extension
                    .call_handle_event(store, surface_id, &event.into())?
                    .map(|update| ui_ext::Update::from(update).into())
            }
            Self::V0_2(extension) => extension
                .call_handle_event(store, surface_id, &event.into())?
                .map(Into::into),
        })
    }

    fn list_rows(
        &self,
        store: &mut Store<State>,
        surface_id: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> wasmtime::Result<Vec<model::ListRow>> {
        Ok(match self {
            Self::V0_1(extension) => extension
                .call_list_rows(store, surface_id, list, start, count)?
                .into_iter()
                .map(|row| {
                    model::ListRow::from(ui_ext::ListRow {
                        index: row.index,
                        nodes: row.nodes.into_iter().map(Into::into).collect(),
                    })
                })
                .collect(),
            Self::V0_2(extension) => extension
                .call_list_rows(store, surface_id, list, start, count)?
                .into_iter()
                .map(Into::into)
                .collect(),
        })
    }

    /// A 0.1 component has no `tick`; it can never request one either.
    fn tick(
        &self,
        store: &mut Store<State>,
        surface_id: &str,
    ) -> wasmtime::Result<std::result::Result<model::Update, String>> {
        Ok(match self {
            Self::V0_1(_) => Ok(model::Update::Unchanged),
            Self::V0_2(extension) => extension.call_tick(store, surface_id)?.map(Into::into),
        })
    }
}

/// A mounted surface or decoration: the tree in the 0.1 model, which `ui_ext::Tree` validates
/// (the structure carries over exactly), and the full 0.2 props of every node.
#[derive(Debug, Clone)]
struct Mounted {
    tree: Tree,
    props: HashMap<NodeId, model::Props>,
}

#[derive(Debug, Default)]
struct Applied {
    touched: BTreeSet<NodeId>,
    invalidated: Vec<model::InvalidateRowsOp>,
}

impl Mounted {
    fn new(nodes: &[model::Node], limits: TreeLimits) -> Result<Self, ui_ext::TreeError> {
        let tree = Tree::from_nodes(nodes.iter().map(shim::node).collect(), limits)?;
        Ok(Self {
            tree,
            props: nodes
                .iter()
                .map(|node| (node.id, node.props.clone()))
                .collect(),
        })
    }

    /// Applies an update atomically, as the host's tree does. An `invalidate-rows` target must be
    /// a list or table at that point of the update.
    fn apply(&mut self, update: &model::Update, limits: TreeLimits) -> Result<Applied> {
        update.check_caps(limits)?;
        let mut applied = Applied::default();
        match update {
            model::Update::Unchanged => {}
            model::Update::Replace(nodes) => {
                applied.touched = self.tree.apply_update(ui_ext::Update::Replace(
                    nodes.iter().map(shim::node).collect(),
                ))?;
                self.props = nodes
                    .iter()
                    .map(|node| (node.id, node.props.clone()))
                    .collect();
            }
            model::Update::Patches(patches) => {
                let mut next = self.tree.clone();
                let mut run = Vec::new();
                for (index, patch) in patches.iter().enumerate() {
                    if let model::Patch::InvalidateRows(op) = patch {
                        applied
                            .touched
                            .extend(next.apply_patches(std::mem::take(&mut run))?);
                        match next.get(op.target).map(TreeNode::props) {
                            Some(Props::List(_) | Props::Table(_)) => {}
                            Some(props) => anyhow::bail!(
                                "patch {index}: invalidate-rows targets node {}, a {}, not a list \
                                 or table",
                                op.target,
                                props.kind_name()
                            ),
                            None => anyhow::bail!(
                                "patch {index}: invalidate-rows targets unknown node {}",
                                op.target
                            ),
                        }
                        applied.touched.insert(op.target);
                        applied.invalidated.push(op.clone());
                    } else {
                        run.extend(shim::patch(patch));
                    }
                }
                applied.touched.extend(next.apply_patches(run)?);
                self.tree = next;
                for patch in patches {
                    match patch {
                        model::Patch::SetProps(id, props) => {
                            self.props.insert(*id, props.clone());
                        }
                        model::Patch::Insert(op) => {
                            self.props.insert(op.node.id, op.node.props.clone());
                        }
                        model::Patch::Remove(_) | model::Patch::InvalidateRows(_) => {}
                    }
                }
                let tree = &self.tree;
                self.props.retain(|id, _| tree.get(*id).is_some());
            }
        }
        Ok(applied)
    }

    /// What the renderer writes into its tree before it sends `event`: typed text, and the
    /// selection `row-selected` moved.
    fn write_locally(&mut self, event: &model::UiEvent) {
        match event {
            model::UiEvent::InputChanged(id, value) | model::UiEvent::InputSubmitted(id, value) => {
                let Some(model::Props::Input(input)) = self.props.get_mut(id) else {
                    return;
                };
                if input.value == *value {
                    return;
                }
                input.value = value.clone();
                let props = shim::props(&model::Props::Input(input.clone()));
                self.tree
                    .apply_patches(vec![ui_ext::Patch::SetProps(*id, props)])
                    .ok();
            }
            model::UiEvent::RowSelected(id, row) => {
                if let Some(model::Props::List(list)) = self.props.get_mut(id) {
                    list.selected = Some(*row);
                }
            }
            model::UiEvent::Click(_)
            | model::UiEvent::RowClick(..)
            | model::UiEvent::Dismissed(_)
            | model::UiEvent::NotificationAction(_)
            | model::UiEvent::NotificationClosed(..)
            | model::UiEvent::ProgressCancelRequested(_) => {}
        }
    }

    fn nodes(&self) -> Vec<model::Node> {
        self.tree
            .to_nodes()
            .into_iter()
            .filter_map(|node| {
                Some(model::Node {
                    id: node.id,
                    parent: node.parent,
                    props: self.props.get(&node.id)?.clone(),
                })
            })
            .collect()
    }
}

/// A call the renderer makes on its own after applying effects: mounting a shown modal, or
/// remounting a surface or decoration whose new tree did not fit.
enum FollowUp {
    Mount(String),
}

/// One loaded extension and the trees of its mounted surfaces and decorations.
pub struct Harness {
    store: Store<State>,
    extension: Bindings,
    api_version: Version,
    ui_api_version: ApiVersion,
    /// The live surfaces: the static ones, then those `add-surface` added, minus removed ones.
    surfaces: Vec<model::Surface>,
    /// `surfaces` in the 0.1 model; modals left out.
    surfaces_v0_1: Vec<ui_ext::Surface>,
    mounted: HashMap<String, Mounted>,
    decorations: BTreeMap<String, model::DecorationLocation>,
    shown_modals: BTreeSet<String>,
    pending_ticks: BTreeMap<String, Duration>,
    tick_errors: HashMap<String, u32>,
    limits: TreeLimits,
    /// Every effect so far, in order, including those of loading.
    effects: Vec<HostEffect>,
    /// The pushes among `effects`, in the 0.1 model.
    pushed: Vec<Pushed>,
    logs: Vec<(ui_ext::LogLevel, String)>,
    disabled: Option<String>,
}

impl Harness {
    /// Compiles and instantiates a component with every capability ([`ALL_CAPABILITIES`]) and
    /// asks it for its surfaces.
    pub fn load(wasm: &[u8]) -> Result<Self> {
        Self::load_with_capabilities(wasm, ALL_CAPABILITIES)
    }

    /// Like [`Harness::load`], with the `[fyx_ui].capabilities` of a manifest.
    pub fn load_with_capabilities(wasm: &[u8], capabilities: &[&str]) -> Result<Self> {
        let runtime = runtime()?;
        let section_version = api_version_section(wasm)?;
        let component = Component::from_binary(&runtime.engine, wasm)
            .map_err(anyhow::Error::from)
            .context("compiling the component")?;
        let api_version = section_version
            .or_else(|| api_version_from_imports(&component, &runtime.engine))
            .with_context(|| {
                format!(
                    "not a fyx:ui component: no {API_VERSION_SECTION} section and no fyx:ui import"
                )
            })?;
        let ui_api_version = u16::try_from(api_version.major)
            .ok()
            .zip(u16::try_from(api_version.minor).ok())
            .zip(u16::try_from(api_version.patch).ok())
            .and_then(|((major, minor), patch)| ApiVersion::from_parts(major, minor, patch))
            .with_context(|| {
                format!(
                    "the component targets fyx:ui {api_version}; the harness runs {}",
                    SUPPORTED_API_VERSIONS
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        anyhow::ensure!(
            ui_api_version != ApiVersion::V0_3_0,
            "the component targets fyx:ui 0.3.0, which the harness does not run yet (plan 4.12); the harness runs {}",
            SUPPORTED_API_VERSIONS
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );

        let mut wasi = WasiCtxBuilder::new();
        wasi.inherit_stderr();
        let mut store = Store::new(
            &runtime.engine,
            State {
                table: ResourceTable::new(),
                ctx: wasi.build(),
                pushes: Vec::new(),
                pushed_items: 0,
                lifted_text: 0,
                api: ApiState::new(
                    ui_api_version,
                    capabilities.iter().map(ToString::to_string).collect(),
                ),
                logs: Vec::new(),
                call_started: Instant::now(),
                call_budget: INIT_BUDGET,
                limits: store_limits(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.epoch_deadline_callback(|context| {
            let state = context.data();
            if state.call_started.elapsed() > state.call_budget {
                Ok(UpdateDeadline::Interrupt)
            } else {
                Ok(UpdateDeadline::Continue(1))
            }
        });
        store.data_mut().begin_call(INIT_BUDGET);
        store.set_epoch_deadline(1);
        let extension = match ui_api_version {
            ApiVersion::V0_1_0 => {
                bindings::UiExtension::instantiate(&mut store, &component, &runtime.linker_v0_1)
                    .map(Bindings::V0_1)
            }
            ApiVersion::V0_2_0 => {
                v0_2::UiExtension::instantiate(&mut store, &component, &runtime.linker_v0_2)
                    .map(Bindings::V0_2)
            }
            // The staged 0.3 world runs in the host (ui_ext_wasm); the harness gains it with the
            // 0.3 SDK (plan 4.12).
            ApiVersion::V0_3_0 => Err(wasmtime::Error::msg(
                "fyx:ui 0.3 is staged; the test harness does not run 0.3 components yet",
            )),
        }
        .map_err(anyhow::Error::from)
        .context("instantiating the component")?;
        let surfaces = extension
            .surfaces(&mut store)
            .map_err(anyhow::Error::from)
            .context("surfaces() failed")?;
        let effects = store.data_mut().finish_call();
        store
            .data_mut()
            .api
            .register_static(&surfaces)
            .context("the host would refuse the surfaces from surfaces()")?;
        let logs = std::mem::take(&mut store.data_mut().logs);
        let mut harness = Self {
            store,
            extension,
            api_version,
            ui_api_version,
            surfaces,
            surfaces_v0_1: Vec::new(),
            mounted: HashMap::new(),
            decorations: BTreeMap::new(),
            shown_modals: BTreeSet::new(),
            pending_ticks: BTreeMap::new(),
            tick_errors: HashMap::new(),
            limits: ui_api_version.tree_limits(),
            effects: Vec::new(),
            pushed: Vec::new(),
            logs,
            disabled: None,
        };
        harness.record(&effects);
        let follow_ups = harness.apply_effects(&effects);
        harness.run_follow_ups(follow_ups);
        harness.refresh_surfaces_v0_1();
        Ok(harness)
    }

    pub fn load_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let wasm = std::fs::read(path).with_context(|| format!("reading {path:?}"))?;
        Self::load(&wasm).with_context(|| format!("loading {path:?}"))
    }

    /// Loads the component named by `FYX_EXT_COMPONENT`, which `fyx-ext test` sets.
    pub fn from_env() -> Result<Self> {
        let path = std::env::var_os(COMPONENT_VARIABLE).with_context(|| {
            format!("{COMPONENT_VARIABLE} is not set; run the tests with `fyx-ext test`, or set it to a built component")
        })?;
        Self::load_file(path)
    }

    pub fn api_version(&self) -> &Version {
        &self.api_version
    }

    pub fn ui_api_version(&self) -> ApiVersion {
        self.ui_api_version
    }

    /// The live surfaces in the 0.1 model; a 0.2 component's modals are left out.
    pub fn surfaces(&self) -> &[ui_ext::Surface] {
        &self.surfaces_v0_1
    }

    /// The live surfaces: the static ones, then those `add-surface` added, minus removed ones.
    pub fn surfaces_v0_2(&self) -> &[model::Surface] {
        &self.surfaces
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    /// The tree of a mounted surface or decoration, as the host holds it.
    pub fn tree(&self, surface_id: &str) -> Option<&Tree> {
        self.mounted.get(surface_id).map(|mounted| &mounted.tree)
    }

    /// The tree of a mounted surface or decoration in the 0.2 model, parents first.
    pub fn nodes_v0_2(&self, surface_id: &str) -> Option<Vec<model::Node>> {
        self.mounted.get(surface_id).map(Mounted::nodes)
    }

    /// The 0.2 props of one node of a mounted surface or decoration.
    pub fn props_v0_2(&self, surface_id: &str, id: NodeId) -> Option<&model::Props> {
        self.mounted.get(surface_id)?.props.get(&id)
    }

    /// Every update pushed so far, in order, in the 0.1 model.
    pub fn pushed(&self) -> &[Pushed] {
        &self.pushed
    }

    /// Every effect so far, in order, including those of loading.
    pub fn effects(&self) -> &[HostEffect] {
        &self.effects
    }

    /// The delay of the surface's pending tick, already clamped up to [`MIN_TICK_DELAY`].
    pub fn pending_tick(&self, surface_id: &str) -> Option<Duration> {
        self.pending_ticks.get(surface_id).copied()
    }

    /// Whether a modal is open: shown by the extension and not hidden, removed or dismissed.
    pub fn is_shown(&self, surface_id: &str) -> bool {
        self.shown_modals.contains(surface_id)
    }

    /// Where a live decoration is placed; its tree is [`Harness::tree`] of its id.
    pub fn decoration(&self, id: &str) -> Option<&model::DecorationLocation> {
        self.decorations.get(id)
    }

    /// The live decoration ids.
    pub fn decorations(&self) -> impl Iterator<Item = &str> {
        self.decorations.keys().map(String::as_str)
    }

    /// Everything the extension logged so far, and the harness's own `warn` lines about what the
    /// renderer would have dropped.
    pub fn logs(&self) -> &[(ui_ext::LogLevel, String)] {
        &self.logs
    }

    /// Why the extension is disabled, after a trap or an overrun call.
    pub fn disabled(&self) -> Option<&str> {
        self.disabled.as_deref()
    }

    /// Mounts a live surface or decoration (again, if it was mounted) and returns its validated
    /// tree. A modal mounts only while it is shown.
    pub fn mount(&mut self, surface_id: &str) -> Result<&Tree> {
        let live = self.store.data().api.live();
        anyhow::ensure!(
            live.surface_kind(surface_id).is_some() || live.is_decoration(surface_id),
            "the extension has no live surface or decoration {surface_id:?}"
        );
        if matches!(
            live.surface_kind(surface_id),
            Some(model::SurfaceKind::Modal(_))
        ) {
            anyhow::ensure!(
                self.shown_modals.contains(surface_id),
                "modal {surface_id:?} is not shown; it mounts when the extension shows it"
            );
        }
        self.mount_now(surface_id)?;
        Ok(&self.mounted[surface_id].tree)
    }

    /// Sends one event to a mounted surface and applies what it pushed and replied (0.1 model).
    pub fn event(&mut self, surface_id: &str, event: ui_ext::UiEvent) -> Result<EventOutcome> {
        self.event_v0_2(surface_id, event.into())
            .map(EventOutcome::from)
    }

    /// Sends one event to a mounted surface or decoration and applies its effects, then its reply.
    /// Typed text and a `row-selected` selection are written into the tree first. Use
    /// [`Harness::dismiss`] for `dismissed`.
    pub fn event_v0_2(&mut self, surface_id: &str, event: model::UiEvent) -> Result<Outcome> {
        anyhow::ensure!(
            !matches!(event, model::UiEvent::Dismissed(_)),
            "send dismissed with Harness::dismiss, which closes the modal first"
        );
        self.mounted
            .get_mut(surface_id)
            .with_context(|| format!("surface {surface_id:?} is not mounted"))?
            .write_locally(&event);
        self.tick_errors.remove(surface_id);
        let (result, effects) = self.call_handle_event(surface_id, event)?;
        self.finish(surface_id, "handle-event", result, effects)
    }

    /// Calls `handle-event` without a mounted surface or applying the reply: what the host
    /// runtime does before the renderer applies anything. Pushes are applied and recorded as
    /// usual. `Err` means the call trapped or overran its budget, which disables the extension.
    pub fn handle_event_raw(
        &mut self,
        surface_id: &str,
        event: ui_ext::UiEvent,
    ) -> Result<RawReply<ui_ext::Update>> {
        let raw = self.handle_event_raw_v0_2(surface_id, event.into())?;
        Ok(RawReply {
            pushed: shim::pushes(&raw.effects),
            value: raw.value.map(shim::update),
        })
    }

    /// [`Harness::handle_event_raw`] in the 0.2 model, with every effect.
    pub fn handle_event_raw_v0_2(
        &mut self,
        surface_id: &str,
        event: model::UiEvent,
    ) -> Result<RawEffects<model::Update>> {
        let (value, effects) = self.call_handle_event(surface_id, event)?;
        let follow_ups = self.apply_effects(&effects);
        self.run_follow_ups(follow_ups);
        Ok(RawEffects { effects, value })
    }

    /// Closes a shown modal as the user would: its tree and pending tick go first, then
    /// `dismissed(reason)` is sent. Its effects apply; its reply does not, and should be
    /// `unchanged` (the host ignores anything else).
    pub fn dismiss(&mut self, surface_id: &str, reason: model::DismissReason) -> Result<Outcome> {
        anyhow::ensure!(
            self.shown_modals.remove(surface_id),
            "modal {surface_id:?} is not shown"
        );
        self.mounted.remove(surface_id);
        self.pending_ticks.remove(surface_id);
        let (result, effects) =
            self.call_handle_event(surface_id, model::UiEvent::Dismissed(reason))?;
        let follow_ups = self.apply_effects(&effects);
        self.run_follow_ups(follow_ups);
        let update = result.map_err(|message| {
            anyhow::anyhow!("handle-event({surface_id:?}, dismissed): {message}")
        })?;
        Ok(Outcome {
            effects,
            update,
            touched: BTreeSet::new(),
            invalidated: Vec::new(),
        })
    }

    /// Runs the surface's pending tick, as the host does when it falls due, and applies its
    /// effects, then its reply. It fails without a call when no tick is pending, when the surface
    /// is not mounted, or after [`MAX_CONSECUTIVE_TICK_ERRORS`] failed ticks in a row until the
    /// next event on the surface.
    pub fn tick(&mut self, surface_id: &str) -> Result<Outcome> {
        anyhow::ensure!(
            self.pending_ticks.contains_key(surface_id),
            "no tick is pending for {surface_id:?}: the extension did not request one, or \
             cancelled it"
        );
        let errors = self.tick_errors.get(surface_id).copied().unwrap_or(0);
        anyhow::ensure!(
            errors < MAX_CONSECUTIVE_TICK_ERRORS,
            "the ticks of {surface_id:?} stopped after {errors} failed ticks in a row; an event on \
             it resumes them"
        );
        anyhow::ensure!(
            self.mounted.contains_key(surface_id),
            "surface {surface_id:?} is not mounted; the host ticks mounted surfaces only"
        );
        self.pending_ticks.remove(surface_id);
        let surface = surface_id.to_string();
        let (result, effects) =
            self.call(move |extension, store| extension.tick(store, &surface))?;
        match &result {
            Ok(_) => {
                self.tick_errors.remove(surface_id);
            }
            Err(_) => *self.tick_errors.entry(surface_id.to_string()).or_default() += 1,
        }
        self.finish(surface_id, "tick", result, effects)
    }

    /// Rows `start..start + count` of a list, each validated as a single-root tree (0.1 model).
    pub fn rows(
        &mut self,
        surface_id: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> Result<Vec<(u32, Tree)>> {
        Ok(self
            .rows_checked(surface_id, list, start, count)?
            .into_iter()
            .map(|(row, tree)| (row.index, tree))
            .collect())
    }

    /// Rows `start..start + count` of a list, each validated as a single-root tree; `indent` as
    /// the extension sent it.
    pub fn rows_v0_2(
        &mut self,
        surface_id: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> Result<Vec<model::ListRow>> {
        Ok(self
            .rows_checked(surface_id, list, start, count)?
            .into_iter()
            .map(|(row, _)| row)
            .collect())
    }

    /// The content of a text node.
    pub fn text(&self, surface_id: &str, id: NodeId) -> Option<&str> {
        match self.tree(surface_id)?.get(id).map(TreeNode::props) {
            Some(Props::Text(text)) => Some(&text.content),
            _ => None,
        }
    }

    /// The first node in tree order whose props match.
    pub fn find(&self, surface_id: &str, matches: impl Fn(&Props) -> bool) -> Option<NodeId> {
        self.tree(surface_id)?
            .to_nodes()
            .into_iter()
            .find(|node| matches(&node.props))
            .map(|node| node.id)
    }

    pub fn find_text(&self, surface_id: &str, content: &str) -> Option<NodeId> {
        self.find(
            surface_id,
            |props| matches!(props, Props::Text(text) if text.content == content),
        )
    }

    pub fn find_button(&self, surface_id: &str, label: &str) -> Option<NodeId> {
        self.find(
            surface_id,
            |props| matches!(props, Props::Button(button) if button.label == label),
        )
    }

    /// Clicks the first button with this label (0.1 model).
    pub fn click(&mut self, surface_id: &str, label: &str) -> Result<EventOutcome> {
        self.click_v0_2(surface_id, label).map(EventOutcome::from)
    }

    /// Clicks the first button with this label.
    pub fn click_v0_2(&mut self, surface_id: &str, label: &str) -> Result<Outcome> {
        let button = self
            .find_button(surface_id, label)
            .with_context(|| format!("no button {label:?} on {surface_id:?}"))?;
        self.event_v0_2(surface_id, model::UiEvent::Click(button))
    }

    fn call_handle_event(
        &mut self,
        surface_id: &str,
        event: model::UiEvent,
    ) -> Result<(std::result::Result<model::Update, String>, Vec<HostEffect>)> {
        let surface = surface_id.to_string();
        self.call(move |extension, store| {
            store.data_mut().api.begin_event(&surface, &event);
            extension.handle_event(store, &surface, event)
        })
    }

    /// After an event or tick: the effects, then the reply on the surface, then the renderer's own
    /// follow-up calls.
    fn finish(
        &mut self,
        surface_id: &str,
        what: &str,
        result: std::result::Result<model::Update, String>,
        effects: Vec<HostEffect>,
    ) -> Result<Outcome> {
        let follow_ups = self.apply_effects(&effects);
        let applied = match result {
            Ok(update) => match self.mounted.get_mut(surface_id) {
                Some(mounted) => mounted
                    .apply(&update, self.limits)
                    .with_context(|| {
                        format!("the host would reject the reply to {what} on {surface_id:?}")
                    })
                    .map(|applied| (update, applied)),
                // The effects removed or closed it: the host discards the reply.
                None => Ok((update, Applied::default())),
            },
            Err(message) => Err(anyhow::anyhow!("{what}({surface_id:?}): {message}")),
        };
        self.run_follow_ups(follow_ups);
        let (update, applied) = applied?;
        Ok(Outcome {
            effects,
            update,
            touched: applied.touched,
            invalidated: applied.invalidated,
        })
    }

    fn rows_checked(
        &mut self,
        surface_id: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> Result<Vec<(model::ListRow, Tree)>> {
        let surface = surface_id.to_string();
        let (rows, effects) = self.call(move |extension, store| {
            extension.list_rows(store, &surface, list, start, count)
        })?;
        let follow_ups = self.apply_effects(&effects);
        self.run_follow_ups(follow_ups);
        let requested = start..start.saturating_add(count);
        rows.into_iter()
            .map(|row| {
                anyhow::ensure!(
                    requested.contains(&row.index),
                    "list-rows({surface_id:?}, {list}) returned row {} outside {requested:?}",
                    row.index
                );
                let tree = Tree::from_row(row.nodes.iter().map(shim::node).collect(), self.limits)
                    .with_context(|| {
                        format!("invalid row {} of list {list} on {surface_id:?}", row.index)
                    })?;
                Ok((row, tree))
            })
            .collect()
    }

    /// Calls `mount` and stores the validated tree; the call's effects apply first.
    fn mount_now(&mut self, surface_id: &str) -> Result<()> {
        let surface = surface_id.to_string();
        let (result, effects) =
            self.call(move |extension, store| extension.mount(store, &surface))?;
        let follow_ups = self.apply_effects(&effects);
        let mounted = result
            .map_err(|message| anyhow::anyhow!("mount({surface_id:?}): {message}"))
            .and_then(|nodes| {
                Mounted::new(&nodes, self.limits)
                    .with_context(|| format!("invalid tree from mount({surface_id:?})"))
            });
        if let Ok(mounted) = &mounted {
            self.mounted.insert(surface_id.to_string(), mounted.clone());
        }
        self.run_follow_ups(follow_ups);
        mounted.map(drop)
    }

    /// Runs one export call with the call budget and records its effects. A trap or an overrun
    /// disables the extension and drops the effects, as in the host.
    fn call<T>(
        &mut self,
        call: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> Result<(T, Vec<HostEffect>)> {
        if let Some(reason) = &self.disabled {
            anyhow::bail!("the extension is disabled after an earlier failure: {reason}");
        }
        self.store.data_mut().begin_call(CALL_BUDGET);
        self.store.set_epoch_deadline(1);
        let result = call(&self.extension, &mut self.store);
        let effects = self.store.data_mut().finish_call();
        self.logs.append(&mut self.store.data_mut().logs);
        match result {
            Ok(value) => {
                self.record(&effects);
                Ok((value, effects))
            }
            Err(error) => {
                let error = anyhow::Error::from(error);
                self.disabled = Some(format!("{error:#}"));
                Err(error.context("the call trapped or ran over its time budget"))
            }
        }
    }

    fn record(&mut self, effects: &[HostEffect]) {
        self.effects.extend_from_slice(effects);
        self.pushed.extend(shim::pushes(effects));
    }

    /// Applies a call's effects in order, as the renderer does, and returns the calls it would make
    /// next.
    fn apply_effects(&mut self, effects: &[HostEffect]) -> Vec<FollowUp> {
        let mut follow_ups = Vec::new();
        for effect in effects {
            match effect {
                HostEffect::Push { surface_id, update } => {
                    if let Some(mounted) = self.mounted.get_mut(surface_id)
                        && mounted.apply(update, self.limits).is_err()
                    {
                        follow_ups.push(FollowUp::Mount(surface_id.clone()));
                    }
                }
                HostEffect::RequestTick { surface_id, delay } => {
                    self.pending_ticks.insert(surface_id.clone(), *delay);
                }
                HostEffect::CancelTick { surface_id } => {
                    self.pending_ticks.remove(surface_id);
                }
                HostEffect::AddSurface(surface) => {
                    self.surfaces.push(surface.clone());
                    self.refresh_surfaces_v0_1();
                }
                HostEffect::RemoveSurface { surface_id } => {
                    self.surfaces.retain(|surface| surface.id != *surface_id);
                    self.refresh_surfaces_v0_1();
                    self.forget(surface_id);
                }
                HostEffect::ShowSurface { surface_id } => {
                    if self.is_modal(surface_id) && self.shown_modals.insert(surface_id.clone()) {
                        follow_ups.push(FollowUp::Mount(surface_id.clone()));
                    }
                }
                HostEffect::HideSurface { surface_id } => {
                    if self.is_modal(surface_id) {
                        self.forget(surface_id);
                    }
                }
                HostEffect::PlaceDecoration {
                    id,
                    location,
                    nodes,
                } => {
                    self.decorations.insert(id.clone(), location.clone());
                    match Mounted::new(nodes, self.limits) {
                        Ok(mounted) => {
                            self.mounted.insert(id.clone(), mounted);
                        }
                        Err(_) => {
                            self.mounted.remove(id);
                            follow_ups.push(FollowUp::Mount(id.clone()));
                        }
                    }
                }
                HostEffect::RemoveDecoration { id } => {
                    self.decorations.remove(id);
                    self.mounted.remove(id);
                }
            }
        }
        follow_ups
    }

    fn run_follow_ups(&mut self, follow_ups: Vec<FollowUp>) {
        for FollowUp::Mount(id) in follow_ups {
            if self.disabled.is_some() {
                return;
            }
            let live = self.store.data().api.live();
            let is_decoration = live.is_decoration(&id);
            if !is_decoration && live.surface_kind(&id).is_none() {
                continue;
            }
            if let Err(error) = self.mount_now(&id) {
                if is_decoration && self.disabled.is_none() {
                    self.decorations.remove(&id);
                    self.mounted.remove(&id);
                    self.store.data_mut().api.decoration_dropped(&id);
                    self.logs.push((
                        ui_ext::LogLevel::Warn,
                        format!(
                            "fyx-ext-harness: removed decoration {id:?}: its tree did not fit and \
                             {error:#}"
                        ),
                    ));
                } else {
                    self.mounted.remove(&id);
                    self.logs.push((
                        ui_ext::LogLevel::Warn,
                        format!("fyx-ext-harness: the remount of {id:?} failed: {error:#}"),
                    ));
                }
            }
        }
    }

    fn is_modal(&self, surface_id: &str) -> bool {
        matches!(
            self.store.data().api.live().surface_kind(surface_id),
            Some(model::SurfaceKind::Modal(_))
        ) || self.surfaces.iter().any(|surface| {
            surface.id == surface_id && matches!(surface.kind, model::SurfaceKind::Modal(_))
        })
    }

    /// Drops what the renderer holds for a surface that was removed or a modal that closed.
    fn forget(&mut self, surface_id: &str) {
        self.mounted.remove(surface_id);
        self.shown_modals.remove(surface_id);
        self.pending_ticks.remove(surface_id);
        self.tick_errors.remove(surface_id);
    }

    fn refresh_surfaces_v0_1(&mut self) {
        self.surfaces_v0_1 = self.surfaces.iter().filter_map(shim::surface).collect();
    }
}

impl From<Outcome> for EventOutcome {
    fn from(outcome: Outcome) -> Self {
        Self {
            pushed: shim::pushes(&outcome.effects),
            update: shim::update(outcome.update),
            touched: outcome.touched,
        }
    }
}

/// The 0.2 model seen through the 0.1 types, as the host's 0.1 view shows it: lossless for a 0.1
/// component. For a 0.2 one it drops tooltips, `secret`, `selected`, `indent`, `invalidate-rows`,
/// modals and every effect but pushes; frozen icons become their snake_case name; a 0.3 image
/// becomes its alt text.
mod shim {
    use super::*;

    fn icon(icon: &model::IconRef) -> String {
        match icon {
            model::IconRef::Frozen(icon) => icon.name().replace('-', "_"),
            model::IconRef::Upstream(name) => name.clone(),
        }
    }

    pub(super) fn props(props: &model::Props) -> Props {
        match props {
            model::Props::Flex(flex) => Props::Flex(flex.clone()),
            model::Props::Text(text) => Props::Text(text.clone()),
            model::Props::Button(button) => Props::Button(ui_ext::ButtonProps {
                label: button.label.clone(),
                icon: button.icon.as_ref().map(icon),
                style: button.style,
                disabled: button.disabled,
            }),
            model::Props::Input(input) => Props::Input(ui_ext::InputProps {
                value: input.value.clone(),
                placeholder: input.placeholder.clone(),
            }),
            model::Props::List(list) => Props::List(ui_ext::ListProps {
                row_count: list.row_count,
                row_height: list.row_height,
            }),
            model::Props::Table(table) => Props::Table(table.clone()),
            model::Props::Icon(props) => Props::Icon(ui_ext::IconProps {
                name: icon(&props.name),
                color: props.color,
            }),
            model::Props::Markdown(source) => Props::Markdown(source.clone()),
            model::Props::DiffView(diff) => Props::DiffView(diff.clone()),
            // 0.3 only; 0.1 has no image, so its alt text stands in.
            model::Props::Image(image) => Props::Text(ui_ext::TextProps {
                content: image.alt.clone(),
                ..ui_ext::TextProps::default()
            }),
        }
    }

    pub(super) fn node(node: &model::Node) -> ui_ext::Node {
        ui_ext::Node {
            id: node.id,
            parent: node.parent,
            props: props(&node.props),
        }
    }

    pub(super) fn patch(patch: &model::Patch) -> Option<ui_ext::Patch> {
        match patch {
            model::Patch::SetProps(id, new) => Some(ui_ext::Patch::SetProps(*id, props(new))),
            model::Patch::Insert(op) => Some(ui_ext::Patch::Insert(ui_ext::InsertOp {
                node: node(&op.node),
                before: op.before,
            })),
            model::Patch::Remove(id) => Some(ui_ext::Patch::Remove(*id)),
            model::Patch::InvalidateRows(_) => None,
        }
    }

    pub(super) fn update(update: model::Update) -> ui_ext::Update {
        match update {
            model::Update::Unchanged => ui_ext::Update::Unchanged,
            model::Update::Replace(nodes) => {
                ui_ext::Update::Replace(nodes.iter().map(node).collect())
            }
            model::Update::Patches(patches) => {
                ui_ext::Update::Patches(patches.iter().filter_map(patch).collect())
            }
        }
    }

    pub(super) fn surface(surface: &model::Surface) -> Option<ui_ext::Surface> {
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

    pub(super) fn pushes(effects: &[HostEffect]) -> Vec<Pushed> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                HostEffect::Push { surface_id, update } => Some(Pushed {
                    surface_id: surface_id.clone(),
                    update: self::update(update.clone()),
                }),
                _ => None,
            })
            .collect()
    }
}

/// The version in the component's `fyx:ui-api-version` section, if it has one.
pub fn api_version_section(wasm: &[u8]) -> Result<Option<Version>> {
    let mut version = None;
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let wasmparser::Payload::CustomSection(section) =
            payload.context("parsing the component")?
            && section.name() == API_VERSION_SECTION
        {
            let [a, b, c, d, e, f] = *section.data() else {
                anyhow::bail!(
                    "the {API_VERSION_SECTION} section must be 6 bytes, got {:?}",
                    section.data()
                );
            };
            version = Some(Version::new(
                u16::from_be_bytes([a, b]).into(),
                u16::from_be_bytes([c, d]).into(),
                u16::from_be_bytes([e, f]).into(),
            ));
        }
    }
    Ok(version)
}

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

impl From<wit_host::LogLevel> for ui_ext::LogLevel {
    fn from(level: wit_host::LogLevel) -> Self {
        match level {
            wit_host::LogLevel::Trace => Self::Trace,
            wit_host::LogLevel::Debug => Self::Debug,
            wit_host::LogLevel::Info => Self::Info,
            wit_host::LogLevel::Warn => Self::Warn,
            wit_host::LogLevel::Error => Self::Error,
        }
    }
}

impl From<wit::ColorToken> for ui_ext::ColorToken {
    fn from(color: wit::ColorToken) -> Self {
        match color {
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

impl From<wit::Props> for Props {
    fn from(props: wit::Props) -> Self {
        match props {
            wit::Props::Flex(flex) => Self::Flex(ui_ext::FlexProps {
                direction: match flex.direction {
                    wit::FlexDirection::Row => ui_ext::FlexDirection::Row,
                    wit::FlexDirection::Column => ui_ext::FlexDirection::Column,
                },
                gap: flex.gap,
                padding: flex.padding,
                grow: flex.grow,
                align: match flex.align {
                    wit::Align::Start => ui_ext::Align::Start,
                    wit::Align::Center => ui_ext::Align::Center,
                    wit::Align::End => ui_ext::Align::End,
                    wit::Align::Stretch => ui_ext::Align::Stretch,
                },
                background: flex.background.map(Into::into),
            }),
            wit::Props::Text(text) => Self::Text(ui_ext::TextProps {
                content: text.content,
                color: text.color.map(Into::into),
                bold: text.bold,
                monospace: text.monospace,
            }),
            wit::Props::Button(button) => Self::Button(ui_ext::ButtonProps {
                label: button.label,
                icon: button.icon,
                style: match button.style {
                    wit::ButtonStyle::Default => ui_ext::ButtonStyle::Default,
                    wit::ButtonStyle::Filled => ui_ext::ButtonStyle::Filled,
                    wit::ButtonStyle::Subtle => ui_ext::ButtonStyle::Subtle,
                    wit::ButtonStyle::Danger => ui_ext::ButtonStyle::Danger,
                },
                disabled: button.disabled,
            }),
            wit::Props::Input(input) => Self::Input(ui_ext::InputProps {
                value: input.value,
                placeholder: input.placeholder,
            }),
            wit::Props::List(list) => Self::List(ui_ext::ListProps {
                row_count: list.row_count,
                row_height: list.row_height,
            }),
            wit::Props::Table(table) => Self::Table(ui_ext::TableProps {
                columns: table.columns,
                row_count: table.row_count,
            }),
            wit::Props::Icon(icon) => Self::Icon(ui_ext::IconProps {
                name: icon.name,
                color: icon.color.map(Into::into),
            }),
            wit::Props::Markdown(source) => Self::Markdown(source),
            wit::Props::DiffView(diff) => Self::DiffView(ui_ext::DiffViewProps {
                old_text: diff.old_text,
                new_text: diff.new_text,
                language: diff.language,
            }),
        }
    }
}

impl From<wit::Node> for ui_ext::Node {
    fn from(node: wit::Node) -> Self {
        Self {
            id: node.id,
            parent: node.parent,
            props: node.props.into(),
        }
    }
}

impl From<wit::Update> for ui_ext::Update {
    fn from(update: wit::Update) -> Self {
        match update {
            wit::Update::Unchanged => Self::Unchanged,
            wit::Update::Replace(nodes) => {
                Self::Replace(nodes.into_iter().map(Into::into).collect())
            }
            wit::Update::Patches(patches) => Self::Patches(
                patches
                    .into_iter()
                    .map(|patch| match patch {
                        wit::Patch::SetProps((id, props)) => {
                            ui_ext::Patch::SetProps(id, props.into())
                        }
                        wit::Patch::Insert(insert) => ui_ext::Patch::Insert(ui_ext::InsertOp {
                            node: insert.node.into(),
                            before: insert.before,
                        }),
                        wit::Patch::Remove(id) => ui_ext::Patch::Remove(id),
                    })
                    .collect(),
            ),
        }
    }
}

impl From<wit::Surface> for ui_ext::Surface {
    fn from(surface: wit::Surface) -> Self {
        Self {
            id: surface.id,
            title: surface.title,
            kind: match surface.kind {
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
    fn from(event: ui_ext::UiEvent) -> Self {
        match event {
            ui_ext::UiEvent::Click(id) => Self::Click(id),
            ui_ext::UiEvent::InputChanged(id, value) => Self::InputChanged((id, value)),
            ui_ext::UiEvent::InputSubmitted(id, value) => Self::InputSubmitted((id, value)),
            ui_ext::UiEvent::RowClick(id, index) => Self::RowClick((id, index)),
        }
    }
}

#[cfg(test)]
mod tests;
