//! Runs a Fyx UI extension component in wasmtime without the editor, so extension tests can mount
//! surfaces, send events and assert the trees headless (platform spec decision 10, plan task 1.8;
//! the spec's `fyx_sdk::testing`).
//!
//! Apache-2.0, like the SDK, so extensions can use it as a dev-dependency, and host-only, so the
//! SDK itself never pulls in wasmtime.
//!
//! What it shares with Fyx and what it mirrors:
//! - Trees, patches and their limits are `ui_ext`'s, the same model and validation the host uses.
//! - The call contract follows the host runtime (`ui_ext_wasm`): WASI 0.2 with only stderr, 2 s for
//!   instantiation and `surfaces()`, 200 ms per call, at most 64 pushes per call, an instance
//!   that traps or overruns is disabled, pushes apply in order before the call's own reply, rows
//!   must lie in the requested range. A conformance test in `ui_ext_wasm` runs the same guests
//!   through both and fails when they drift.
//! - Like the renderer, typed text is written into the input node before `input-changed` or
//!   `input-submitted` is sent.
//!
//! One difference: a guest blocked inside a WASI call (such as `thread::sleep`) is only stopped
//! when it returns to WebAssembly, while the host's async runtime stops waiting at the deadline.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use semver::Version;
use ui_ext::{NodeId, Props, Tree, TreeLimits, TreeNode};
use wasmtime::component::{Component, HasData, Linker, ResourceTable};
use wasmtime::{Engine, Store, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

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
pub const SUPPORTED_API_VERSIONS: &[Version] = &[Version::new(0, 1, 0)];
/// The section `register_extension!` writes: three big-endian `u16`s.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";
/// Instantiation and `surfaces()` together.
pub const INIT_BUDGET: Duration = Duration::from_secs(2);
/// Every later call.
pub const CALL_BUDGET: Duration = Duration::from_millis(200);
pub const EPOCH_INTERVAL: Duration = Duration::from_millis(10);
pub const MAX_PUSHES_PER_CALL: usize = 64;
/// `fyx-ext test` sets it to the built component; [`Harness::from_env`] reads it.
pub const COMPONENT_VARIABLE: &str = "FYX_EXT_COMPONENT";

/// An update the extension pushed during a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    pub surface_id: String,
    pub update: ui_ext::Update,
}

/// One export call as the host receives it: the updates pushed during the call, and the guest's
/// own result, whose `Err` is the message the guest returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawReply<T> {
    pub pushed: Vec<Pushed>,
    pub value: std::result::Result<T, String>,
}

/// What one event did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOutcome {
    /// Pushed during the call, applied first, in order.
    pub pushed: Vec<Pushed>,
    /// The call's own reply, applied to the event's surface after the pushes.
    pub update: ui_ext::Update,
    /// Node ids the reply touched in the event's surface (see `ui_ext::Tree::apply_update`).
    pub touched: BTreeSet<NodeId>,
}

struct State {
    table: ResourceTable,
    ctx: WasiCtx,
    pushes: Vec<Pushed>,
    logs: Vec<(ui_ext::LogLevel, String)>,
    call_started: Instant,
    call_budget: Duration,
}

impl State {
    fn begin_call(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
        self.pushes.clear();
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
        self.pushes.push(Pushed {
            surface_id,
            update: update.into(),
        });
        Ok(())
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) -> wasmtime::Result<()> {
        self.logs.push((level.into(), message));
        Ok(())
    }
}

struct Runtime {
    engine: Engine,
    linker: Linker<State>,
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
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    bindings::UiExtension::add_to_linker::<_, State>(&mut linker, |state| state)?;
    Ok(Runtime { engine, linker })
}

/// One loaded extension and the trees of its mounted surfaces.
pub struct Harness {
    store: Store<State>,
    extension: bindings::UiExtension,
    api_version: Version,
    surfaces: Vec<ui_ext::Surface>,
    trees: HashMap<String, Tree>,
    limits: TreeLimits,
    /// Every update pushed so far, in order, including those during loading.
    pushed: Vec<Pushed>,
    logs: Vec<(ui_ext::LogLevel, String)>,
    disabled: Option<String>,
}

impl Harness {
    /// Compiles and instantiates a component and asks it for its surfaces.
    pub fn load(wasm: &[u8]) -> Result<Self> {
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
        anyhow::ensure!(
            SUPPORTED_API_VERSIONS.contains(&api_version),
            "the component targets fyx:ui {api_version}; the harness runs {}",
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
                logs: Vec::new(),
                call_started: Instant::now(),
                call_budget: INIT_BUDGET,
            },
        );
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
        let extension = bindings::UiExtension::instantiate(&mut store, &component, &runtime.linker)
            .map_err(anyhow::Error::from)
            .context("instantiating the component")?;
        let surfaces = extension
            .call_surfaces(&mut store)
            .map_err(anyhow::Error::from)
            .context("surfaces() failed")?;
        let pushed = std::mem::take(&mut store.data_mut().pushes);
        let logs = std::mem::take(&mut store.data_mut().logs);
        Ok(Self {
            store,
            extension,
            api_version,
            surfaces: surfaces.into_iter().map(Into::into).collect(),
            trees: HashMap::new(),
            limits: TreeLimits::default(),
            pushed,
            logs,
            disabled: None,
        })
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

    pub fn surfaces(&self) -> &[ui_ext::Surface] {
        &self.surfaces
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    /// The tree of a mounted surface, as the host holds it.
    pub fn tree(&self, surface_id: &str) -> Option<&Tree> {
        self.trees.get(surface_id)
    }

    /// Every update pushed so far, in order.
    pub fn pushed(&self) -> &[Pushed] {
        &self.pushed
    }

    /// Everything the extension logged so far.
    pub fn logs(&self) -> &[(ui_ext::LogLevel, String)] {
        &self.logs
    }

    /// Why the extension is disabled, after a trap or an overrun call.
    pub fn disabled(&self) -> Option<&str> {
        self.disabled.as_deref()
    }

    /// Mounts a declared surface (again, if it was mounted) and returns its validated tree.
    pub fn mount(&mut self, surface_id: &str) -> Result<&Tree> {
        anyhow::ensure!(
            self.surfaces.iter().any(|surface| surface.id == surface_id),
            "the extension declares no surface {surface_id:?}"
        );
        let (result, pushed) =
            self.call(|extension, store| extension.call_mount(store, surface_id))?;
        self.apply_pushes(&pushed);
        let nodes =
            result.map_err(|message| anyhow::anyhow!("mount({surface_id:?}): {message}"))?;
        let tree = Tree::from_nodes(nodes.into_iter().map(Into::into).collect(), self.limits)
            .with_context(|| format!("invalid tree from mount({surface_id:?})"))?;
        Ok(self
            .trees
            .entry(surface_id.to_string())
            .insert_entry(tree)
            .into_mut())
    }

    /// Sends one event to a mounted surface and applies what it pushed and replied.
    pub fn event(&mut self, surface_id: &str, event: ui_ext::UiEvent) -> Result<EventOutcome> {
        let tree = self
            .trees
            .get_mut(surface_id)
            .with_context(|| format!("surface {surface_id:?} is not mounted"))?;
        if let ui_ext::UiEvent::InputChanged(id, value)
        | ui_ext::UiEvent::InputSubmitted(id, value) = &event
        {
            write_input_locally(tree, *id, value);
        }
        let reply = self.handle_event_raw(surface_id, event)?;
        let update = reply
            .value
            .map_err(|message| anyhow::anyhow!("handle-event({surface_id:?}): {message}"))?;
        let tree = self
            .trees
            .get_mut(surface_id)
            .with_context(|| format!("surface {surface_id:?} is not mounted"))?;
        let touched = tree.apply_update(update.clone()).with_context(|| {
            format!("the host would reject the reply to an event on {surface_id:?}")
        })?;
        Ok(EventOutcome {
            pushed: reply.pushed,
            update,
            touched,
        })
    }

    /// Calls `handle-event` without a mounted surface or applying the reply: what the host
    /// runtime does before the renderer applies anything. Pushes are applied and recorded as
    /// usual. `Err` means the call trapped or overran its budget, which disables the extension.
    pub fn handle_event_raw(
        &mut self,
        surface_id: &str,
        event: ui_ext::UiEvent,
    ) -> Result<RawReply<ui_ext::Update>> {
        let event = wit::UiEvent::from(event);
        let (result, pushed) =
            self.call(|extension, store| extension.call_handle_event(store, surface_id, &event))?;
        self.apply_pushes(&pushed);
        Ok(RawReply {
            pushed,
            value: result.map(Into::into),
        })
    }

    /// Rows `start..start + count` of a list, each validated as a single-root tree.
    pub fn rows(
        &mut self,
        surface_id: &str,
        list: NodeId,
        start: u32,
        count: u32,
    ) -> Result<Vec<(u32, Tree)>> {
        let (rows, pushed) = self.call(|extension, store| {
            extension.call_list_rows(store, surface_id, list, start, count)
        })?;
        self.apply_pushes(&pushed);
        let requested = start..start.saturating_add(count);
        rows.into_iter()
            .map(|row| {
                anyhow::ensure!(
                    requested.contains(&row.index),
                    "list-rows({surface_id:?}, {list}) returned row {} outside {requested:?}",
                    row.index
                );
                let tree =
                    Tree::from_row(row.nodes.into_iter().map(Into::into).collect(), self.limits)
                        .with_context(|| {
                            format!("invalid row {} of list {list} on {surface_id:?}", row.index)
                        })?;
                Ok((row.index, tree))
            })
            .collect()
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

    /// Clicks the first button with this label.
    pub fn click(&mut self, surface_id: &str, label: &str) -> Result<EventOutcome> {
        let button = self
            .find_button(surface_id, label)
            .with_context(|| format!("no button {label:?} on {surface_id:?}"))?;
        self.event(surface_id, ui_ext::UiEvent::Click(button))
    }

    /// Runs one export call with the call budget; a trap or an overrun disables the extension,
    /// as in the host.
    fn call<T>(
        &mut self,
        call: impl FnOnce(&bindings::UiExtension, &mut Store<State>) -> wasmtime::Result<T>,
    ) -> Result<(T, Vec<Pushed>)> {
        if let Some(reason) = &self.disabled {
            anyhow::bail!("the extension is disabled after an earlier failure: {reason}");
        }
        self.store.data_mut().begin_call(CALL_BUDGET);
        self.store.set_epoch_deadline(1);
        let result = call(&self.extension, &mut self.store);
        let pushes = std::mem::take(&mut self.store.data_mut().pushes);
        self.logs.append(&mut self.store.data_mut().logs);
        match result {
            Ok(value) => Ok((value, pushes)),
            Err(error) => {
                let error = anyhow::Error::from(error);
                self.disabled = Some(format!("{error:#}"));
                Err(error.context("the call trapped or ran over its time budget"))
            }
        }
    }

    /// Applies pushes in order to the surfaces they name, when mounted, and records them.
    fn apply_pushes(&mut self, pushed: &[Pushed]) {
        for push in pushed {
            if let Some(tree) = self.trees.get_mut(&push.surface_id) {
                // A push the host rejects is dropped there too (and remounts the surface); the
                // record keeps it for assertions.
                tree.apply_update(push.update.clone()).ok();
            }
            self.pushed.push(push.clone());
        }
    }
}

/// What the renderer does with typed text before it sends the event: the value goes into the
/// input's props, so the extension does not have to echo it.
fn write_input_locally(tree: &mut Tree, id: NodeId, value: &str) {
    let Some(Props::Input(input)) = tree.get(id).map(TreeNode::props) else {
        return;
    };
    if input.value == value {
        return;
    }
    let props = Props::Input(ui_ext::InputProps {
        value: value.to_string(),
        ..input.clone()
    });
    tree.apply_patches(vec![ui_ext::Patch::SetProps(id, props)])
        .ok();
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
