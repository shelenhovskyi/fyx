//! Host side of the `fyx:ui` worlds: `fyx:ui@0.1.0` (`crates/ui_ext/wit/fyx-ui.wit`, here) and
//! `fyx:ui@0.2.0` (`crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit`, `fyx_ui_v0_2.rs`). One runtime
//! loads both side by side; the version a component targets, read from its `fyx:ui-api-version`
//! section before instantiation, picks its bindings (`dispatch.rs`).
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
    path::{Component as PathComponent, Path, PathBuf},
    pin::{Pin, pin},
    sync::{
        Arc, Condvar, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
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
use sha2::{Digest as _, Sha256};
use ui_ext::{ApiVersion, TreeLimits};
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

mod conversions;
mod dispatch;
mod effects;
mod fyx_ui_v0_2;
mod fyx_ui_v0_3;
pub mod images;
pub mod ticks;

pub use effects::{
    ACTIVATION_WINDOW, EffectReply, EffectReplyError, HostEffect, MAX_PUSHED_ITEMS_PER_CALL_V0_2,
    MAX_TICK_DELAY, MIN_TICK_DELAY,
};
pub use ui_ext::v0_2::{CAPABILITY_EDITOR_DECORATIONS, CAPABILITY_MODAL};

use dispatch::UiBindings;

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
pub const MAX_VERSION: Version = Version::new(0, 3, 0);
/// The newest frozen version. Newer ones are staged: they load only where
/// [`LoadOptions::allow_staged`] allows them (docs/fyx/wit-0.2/migration.md, section 2). The
/// commit that declares 0.2 frozen sets this to [`MAX_VERSION`].
pub const MAX_FROZEN_VERSION: Version = Version::new(0, 1, 0);

/// Custom section with the `fyx:ui` version a component targets: three big-endian `u16`s, the same
/// encoding as `zed:api-version`.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";

pub const UI_EPOCH_INTERVAL: Duration = Duration::from_millis(10);
pub const UI_CALL_BUDGET: Duration = Duration::from_millis(200);
/// Instantiation and `surfaces` run once per load and may set up state, so they get more time.
const UI_INIT_BUDGET: Duration = Duration::from_secs(2);
/// The first `mount` of each surface builds its whole tree (and, in a JS guest, warms the engine
/// up), so it gets this instead of [`UI_CALL_BUDGET`]. Later mounts of the surface do not.
pub const UI_FIRST_MOUNT_BUDGET: Duration = Duration::from_secs(2);
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
/// String bytes all of one call's `push-update` and `log` arguments may carry together; past it
/// the call traps and the extension is disabled.
///
/// Guest memory bounds what one host call lifts (wasmtime checks every string and list against
/// it before copying, and neither `Config::max_wasm_stack`, which bounds the guest's stack, nor
/// the resource limiter, which sees only guest memory and tables, bounds the host's copies). It
/// does not bound a call's total: a guest can push the same 200 MiB string 64 times, and pushes
/// are kept until the call returns. So their text is counted here.
pub const MAX_LIFTED_TEXT_PER_CALL: usize = 16 << 20;
/// Longest `log` message written out; the rest is cut off (it still counts toward
/// [`MAX_LIFTED_TEXT_PER_CALL`]).
pub const MAX_LOG_MESSAGE_LEN: usize = 16 << 10;
/// Linear memory one UI component may grow to: room for a JS engine (a componentize-js guest
/// embeds one) as well as a tree mirror and state. It also bounds each string and list one host
/// call copies out of the guest; [`MAX_LIFTED_TEXT_PER_CALL`] bounds their sum per call.
pub const UI_MEMORY_LIMIT: usize = 256 << 20;
/// Table elements per table (function references; a Rust component uses a few thousand).
pub const UI_TABLE_ELEMENTS_LIMIT: usize = 64 * 1024;
/// Core instances, tables and memories per component (a component and its adapters use a handful).
pub const UI_INSTANCES_LIMIT: usize = 32;
pub const UI_TABLES_LIMIT: usize = 32;
pub const UI_MEMORIES_LIMIT: usize = 8;

/// The `fyx:ui` versions a load accepts. Like `extension_host`'s `wasm_api_version_range`, which
/// caps stable and preview while Zed stages its extension API, staged versions are accepted only
/// when `allow_staged` ([`staged_api_allowed`]). Within the range only the versions
/// [`ApiVersion`] knows exist: for 0.x every minor version is its own world.
pub fn api_version_range(allow_staged: bool) -> RangeInclusive<Version> {
    MIN_VERSION..=if allow_staged {
        MAX_VERSION
    } else {
        MAX_FROZEN_VERSION
    }
}

/// The world of a version this build knows, on any release channel.
pub fn known_api_version(version: &Version) -> Option<ApiVersion> {
    if !version.pre.is_empty() || !version.build.is_empty() {
        return None;
    }
    ApiVersion::from_parts(
        u16::try_from(version.major).ok()?,
        u16::try_from(version.minor).ok()?,
        u16::try_from(version.patch).ok()?,
    )
}

/// Whether some build may load `version`, staged versions included.
pub fn is_supported_api_version(version: &Version) -> bool {
    known_api_version(version).is_some()
}

/// Whether staged versions may load: in Dev and Nightly builds, and for dev extensions
/// (`FYX_UI_DEV_EXTENSIONS`, `fyx.extensions.dev_paths`); never for installed extensions on Stable
/// or Preview.
pub fn staged_api_allowed(dev_or_nightly_build: bool, dev_extension: bool) -> bool {
    dev_or_nightly_build || dev_extension
}

/// What [`LoadOptions::allow_staged`] means when it is `None`: debug builds, or a process started
/// with `FYX_UI_DEV_EXTENSIONS`. Callers that know the release channel pass
/// [`staged_api_allowed`] instead.
pub fn staged_api_allowed_by_default() -> bool {
    cfg!(debug_assertions) || std::env::var_os("FYX_UI_DEV_EXTENSIONS").is_some()
}

/// Every capability a surface or decoration can need, for a load without a manifest.
const ALL_UI_CAPABILITIES: [&str; 5] = [
    CAPABILITY_PANEL,
    CAPABILITY_STATUS_BAR,
    CAPABILITY_MODAL,
    CAPABILITY_EDITOR_DECORATIONS,
    ui_ext::image::CAPABILITY_IMAGES,
];

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
    /// Capabilities the surfaces need: [`CAPABILITY_PANEL`], [`CAPABILITY_STATUS_BAR`], and for
    /// 0.2 [`CAPABILITY_MODAL`] and [`CAPABILITY_EDITOR_DECORATIONS`].
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
        validate_id("extension", &id).with_context(|| format!("in {path:?}"))?;
        for capability in &manifest.capabilities {
            permissions::grantable_tier(capability)
                .with_context(|| format!("extension {id}, in {path:?}"))?;
        }
        anyhow::ensure!(
            is_supported_api_version(&manifest.api_version),
            "extension {id} targets fyx:ui {}, this build supports 0.1.0, 0.2.0 and 0.3.0 (staged)",
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
    /// String bytes of the running call's `push-update` and `log` arguments.
    lifted_text: usize,
    limits: wasmtime::StoreLimits,
    call_started: Instant,
    call_budget: Duration,
    /// Surfaces `mount` has been called for, so only the first mount gets
    /// [`UI_FIRST_MOUNT_BUDGET`].
    mounted: std::collections::HashSet<String>,
    disabled: Option<String>,
    capability_check: Option<Arc<dyn permissions::CapabilityCheck>>,
    diagnostics: diagnostics::DiagnosticsBuffer,
    /// Live ids, declared capabilities, shown modals and the effect list of the 0.2 imports.
    api: effects::ApiState,
}

impl UiState {
    /// For host imports that read or act for the extension: whether it may use `capability`
    /// now. Answers from the host's permission broker each time, so a revocation applies to the
    /// next call. Without a broker, T1 and T2 are denied.
    pub fn check_capability(&self, capability: &str) -> Result<(), permissions::CapabilityDenied> {
        let result = permissions::check_without_broker(
            self.capability_check.as_ref(),
            &self.extension_id,
            capability,
        );
        self.diagnostics.capability_check(
            capability,
            result.as_ref().err().map(|denied| denied.reason.clone()),
        );
        result
    }

    fn new(
        extension_id: Arc<str>,
        ctx: WasiCtx,
        capability_check: Option<Arc<dyn permissions::CapabilityCheck>>,
        diagnostics: diagnostics::DiagnosticsBuffer,
        api: effects::ApiState,
    ) -> Self {
        Self {
            capability_check,
            diagnostics,
            api,
            extension_id,
            table: ResourceTable::new(),
            ctx,
            pushes: Vec::new(),
            pushed_items: 0,
            lifted_text: 0,
            limits: ui_store_limits(),
            call_started: Instant::now(),
            call_budget: UI_INIT_BUDGET,
            mounted: Default::default(),
            disabled: None,
        }
    }

    /// A fresh time budget without dropping the effects made so far: `surfaces` right after
    /// instantiation, whose effects (and the instantiation's) apply together (triage-3, BUG 32).
    fn restart_budget(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
    }

    fn begin_call(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
        self.pushes.clear();
        self.pushed_items = 0;
        self.lifted_text = 0;
        self.api.begin_call();
    }

    fn count_lifted_text(&mut self, bytes: usize) -> wasmtime::Result<()> {
        self.lifted_text = self.lifted_text.saturating_add(bytes);
        if self.lifted_text > MAX_LIFTED_TEXT_PER_CALL {
            wasmtime::bail!(
                "passed more than {MAX_LIFTED_TEXT_PER_CALL} bytes of text to push-update and log \
                 in one call"
            );
        }
        Ok(())
    }
}

/// Which time budget a call gets.
enum CallBudget {
    Regular,
    /// A `mount` of this surface: [`UI_FIRST_MOUNT_BUDGET`] the first time.
    Mount(String),
}

impl CallBudget {
    fn resolve(self, state: &mut UiState) -> Duration {
        match self {
            CallBudget::Mount(surface_id) if state.mounted.insert(surface_id.clone()) => {
                UI_FIRST_MOUNT_BUDGET
            }
            CallBudget::Mount(_) | CallBudget::Regular => UI_CALL_BUDGET,
        }
    }
}

fn props_text_len(props: &ui_ext::Props) -> usize {
    use ui_ext::Props;
    let option_len = |text: &Option<String>| text.as_ref().map_or(0, String::len);
    match props {
        Props::Flex(_) | Props::List(_) => 0,
        Props::Text(props) => props.content.len(),
        Props::Button(props) => props.label.len() + option_len(&props.icon),
        Props::Input(props) => props.value.len() + props.placeholder.len(),
        Props::Table(props) => props.columns.iter().map(String::len).sum(),
        Props::Icon(props) => props.name.len(),
        Props::Markdown(text) => text.len(),
        Props::DiffView(props) => {
            props.old_text.len() + props.new_text.len() + option_len(&props.language)
        }
    }
}

/// String bytes an update carries.
fn update_text_len(update: &ui_ext::Update) -> usize {
    match update {
        ui_ext::Update::Unchanged => 0,
        ui_ext::Update::Replace(nodes) => {
            nodes.iter().map(|node| props_text_len(&node.props)).sum()
        }
        ui_ext::Update::Patches(patches) => patches
            .iter()
            .map(|patch| match patch {
                ui_ext::Patch::SetProps(_, props) => props_text_len(props),
                ui_ext::Patch::Insert(insert) => props_text_len(&insert.node.props),
                ui_ext::Patch::Remove(_) => 0,
            })
            .sum(),
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
        self.count_lifted_text(surface_id.len() + update_text_len(&update))?;
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
        self.write_log(level, message)
    }
}

impl UiState {
    /// `log` of either world: counted toward [`MAX_LIFTED_TEXT_PER_CALL`], cut at
    /// [`MAX_LOG_MESSAGE_LEN`], written at once (not an effect).
    fn write_log(&mut self, level: log::Level, mut message: String) -> wasmtime::Result<()> {
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
        log::log!(
            target: "fyx_ui",
            level,
            "{}: {}",
            self.extension_id,
            ui_ext::sanitize_for_log(&message)
        );
        self.diagnostics.log(level, &message);
        Ok(())
    }
}

struct UiRuntime {
    engine: Engine,
    /// One linker per world, so each version's imports are exactly what its world declares.
    linker: Linker<UiState>,
    linker_v0_2: Linker<UiState>,
    /// Only what this build implements of the staged 0.3 world; instantiation adds traps for the
    /// rest.
    linker_v0_3: Linker<UiState>,
}

fn ui_runtime() -> Result<&'static UiRuntime> {
    static RUNTIME: OnceLock<Result<UiRuntime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| build_ui_runtime().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow!("failed to set up the fyx:ui wasm runtime: {error}"))
}

/// All of WASI p2 is linked (Rust's std imports parts of it, and a component that imports an
/// interface the linker lacks would not instantiate), but no network use is allowed: no TCP or
/// UDP socket, no name lookup, and no address passes the check. A guest's `connect` fails with
/// "access denied" instead of reaching the network.
fn deny_network(wasi: &mut WasiCtxBuilder) {
    wasi.allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false)
        .socket_addr_check(|_, _| Box::pin(async { false }));
}

/// Longest extension or surface id.
pub const MAX_ID_LEN: usize = 64;

/// Extension and surface ids name panels, settings keys and files, so they are restricted to
/// lowercase ASCII letters, digits, `-` and `_`, start with a letter or digit, and are at most
/// `MAX_ID_LEN` long. That also keeps `<extension id>/<surface id>` unambiguous.
pub fn validate_id(kind: &str, id: &str) -> Result<()> {
    if id.is_empty() || id.len() > MAX_ID_LEN {
        anyhow::bail!("{kind} id {id:?} must be 1 to {MAX_ID_LEN} characters long");
    }
    if !id.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
    }) {
        anyhow::bail!("{kind} id {id:?} may only use lowercase ASCII letters, digits, '-' and '_'");
    }
    if !id.as_bytes()[0].is_ascii_alphanumeric() {
        anyhow::bail!("{kind} id {id:?} must start with a letter or a digit");
    }
    Ok(())
}

/// The surfaces a component declares: valid ids, none twice.
pub fn validate_surfaces(surfaces: &[ui_ext::Surface]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for surface in surfaces {
        validate_id("surface", &surface.id)?;
        if !seen.insert(surface.id.as_str()) {
            anyhow::bail!("surface id {:?} is declared twice", surface.id);
        }
    }
    Ok(())
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
                // Nothing to interrupt or time out while no guest call runs: park instead of
                // waking 100 times a second for the whole process (triage-3, PERF 7).
                drop(
                    CALLS_STARTED
                        .wait_while(lock_active_calls(), |active| *active == 0)
                        .unwrap_or_else(PoisonError::into_inner),
                );
                std::thread::sleep(UI_EPOCH_INTERVAL);
                let Some(engine) = weak_engine.upgrade() else {
                    break;
                };
                engine.increment_epoch();
                EPOCH_TICKS.fetch_add(1, Ordering::Relaxed);
                wake_deadlines();
            }
        })
        .context("failed to spawn the fyx:ui epoch thread")?;

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    bindings::UiExtension::add_to_linker::<_, UiState>(&mut linker, |state| state)?;
    // `decorations` is linked for every 0.2 component and checked per call, so a component that
    // imports it loads whether or not it has the capability.
    let mut linker_v0_2 = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker_v0_2)?;
    fyx_ui_v0_2::UiExtension::add_to_linker::<_, UiState>(&mut linker_v0_2, |state| state)?;
    let mut linker_v0_3 = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker_v0_3)?;
    fyx_ui_v0_3::add_to_linker(&mut linker_v0_3)?;
    Ok(UiRuntime {
        engine,
        linker,
        linker_v0_2,
        linker_v0_3,
    })
}

/// Guest calls running now (instantiation included): the epoch thread ticks only while there
/// are any.
static ACTIVE_CALLS: Mutex<usize> = Mutex::new(0);
static CALLS_STARTED: Condvar = Condvar::new();
/// Epoch ticks so far, for tests.
static EPOCH_TICKS: AtomicU64 = AtomicU64::new(0);

fn lock_active_calls() -> std::sync::MutexGuard<'static, usize> {
    ACTIVE_CALLS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One running guest call, counted in [`ACTIVE_CALLS`] while it lives.
struct ActiveCall;

impl ActiveCall {
    fn begin() -> Self {
        let mut active = lock_active_calls();
        *active += 1;
        if *active == 1 {
            CALLS_STARTED.notify_all();
        }
        ActiveCall
    }
}

impl Drop for ActiveCall {
    fn drop(&mut self) {
        let mut active = lock_active_calls();
        *active = active.saturating_sub(1);
    }
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
    // Every guest call runs in here, so this is what keeps the epoch thread ticking.
    let _active = ActiveCall::begin();
    let deadline = Deadline(Instant::now() + budget + UI_DEADLINE_SLACK);
    match futures::future::select(pin!(call), deadline).await {
        Either::Left((result, _)) => result,
        Either::Right(((), _)) => Err(anyhow!(
            "exceeded its time budget of {budget:?} while blocked in a host call"
        )),
    }
}

type UiCall =
    Box<dyn Send + for<'a> FnOnce(&'a mut UiBindings, &'a mut Store<UiState>) -> BoxFuture<'a, ()>>;

/// A loaded `fyx:ui` component of either world. Calls are serialized on its own queue.
#[derive(Clone)]
pub struct UiExtension {
    extension_id: Arc<str>,
    api_version: Version,
    ui_api_version: ApiVersion,
    capabilities: Arc<[String]>,
    /// The static surfaces as the 0.1 renderer sees them: modals are left out.
    surfaces: Arc<[ui_ext::Surface]>,
    surfaces_v0_2: Arc<[ui_ext::v0_2::Surface]>,
    initial_effects: Arc<[HostEffect]>,
    limits: TreeLimits,
    tx: mpsc::UnboundedSender<UiCall>,
    _task: Arc<Task<Result<(), gpui_tokio::JoinError>>>,
    from_precompiled: bool,
    diagnostics: diagnostics::DiagnosticsBuffer,
}

/// Where a precompiled component (`.cwasm`) is kept: `<dir>/<stem>-<key>.cwasm`, where the key
/// hashes the engine's compatibility hash (wasmtime version and configuration) and the
/// component's SHA-256, so a different engine or component never matches a stale file.
///
/// Loading one is loading trusted machine code, so the caller must only ever name a place that
/// Fyx alone writes (the install store), never a path inside an extension package.
#[derive(Clone, Debug)]
pub struct PrecompiledCache {
    pub dir: PathBuf,
    pub stem: String,
}

impl PrecompiledCache {
    /// The file for this component under the current engine.
    pub fn file_for(&self, wasm_bytes: &[u8]) -> Result<PathBuf> {
        let engine = &ui_runtime()?.engine;
        struct Sha256Hasher(Sha256);
        impl std::hash::Hasher for Sha256Hasher {
            fn finish(&self) -> u64 {
                0
            }
            fn write(&mut self, bytes: &[u8]) {
                self.0.update(bytes);
            }
        }
        let mut hasher = Sha256Hasher(Sha256::new());
        std::hash::Hash::hash(&engine.precompile_compatibility_hash(), &mut hasher);
        hasher.0.update(Sha256::digest(wasm_bytes));
        let key = format!("{:x}", hasher.0.finalize());
        Ok(self.dir.join(format!("{}-{}.cwasm", self.stem, &key[..32])))
    }
}

/// Compiles `wasm_bytes`, or loads the precompiled copy in `cache` when it is there and fits this
/// engine. A missing, stale or unreadable copy is (re)written after compiling. Returns whether the
/// copy was used.
fn compile_component(
    engine: &Engine,
    wasm_bytes: &[u8],
    cache: Option<&PrecompiledCache>,
) -> Result<(Component, bool)> {
    let file = match cache.map(|cache| cache.file_for(wasm_bytes)).transpose() {
        Ok(file) => file,
        Err(error) => {
            log::warn!("fyx:ui: no precompiled cache: {error:#}");
            None
        }
    };
    // Read into memory rather than `deserialize_file`, which maps the file: on Windows a mapped
    // file can be neither replaced nor deleted, which would pin old versions in the install store.
    if let Some(file) = &file
        && let Ok(bytes) = std::fs::read(file)
    {
        // SAFETY: `file` is only ever written by `compile_component` below, from a component
        // this engine compiled, at a path the caller guarantees only Fyx writes (see
        // `PrecompiledCache`). wasmtime still checks the header against this engine.
        match unsafe { Component::deserialize(engine, &bytes) } {
            Ok(component) => return Ok((component, true)),
            Err(error) => log::warn!(
                "fyx:ui: precompiled {} is unusable, compiling again: {error:#}",
                file.display()
            ),
        }
    }
    let component = Component::from_binary(engine, wasm_bytes)
        .map_err(anyhow::Error::from)
        .context("failed to compile fyx:ui component")?;
    if let Some(file) = &file {
        if let Err(error) = write_precompiled(&component, file) {
            log::warn!(
                "fyx:ui: could not save precompiled {}: {error:#}",
                file.display()
            );
        }
    }
    Ok((component, false))
}

/// Writes through a temporary file and a rename, so a crash never leaves a torn `.cwasm`.
fn write_precompiled(component: &Component, file: &Path) -> Result<()> {
    let bytes = component.serialize().map_err(anyhow::Error::from)?;
    let dir = file.parent().context("no parent directory")?;
    std::fs::create_dir_all(dir)?;
    let temp = file.with_extension("cwasm.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, file)?;
    Ok(())
}

/// How [`UiExtension::load_with_options`] loads a component.
#[derive(Clone, Default)]
pub struct LoadOptions {
    pub cache: Option<PrecompiledCache>,
    /// The host's permission broker, for [`UiState::check_capability`].
    pub capability_check: Option<Arc<dyn permissions::CapabilityCheck>>,
    /// `[fyx_ui].capabilities`: what the extension declared. The static surfaces are checked
    /// against it at load, and `add-surface`, `show-surface` and the decorations at each call,
    /// together with the broker. `None` (no manifest: tests and bare components) declares every
    /// UI capability. [`UiExtension::load_from_dir_with_options`] sets it from the manifest.
    pub capabilities: Option<Vec<String>>,
    /// Whether a staged version (0.2 until it is frozen) may load; `None` means
    /// [`staged_api_allowed_by_default`].
    pub allow_staged: Option<bool>,
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
        Self::load_with_cache(extension_id, wasm_bytes, None, cx)
    }

    /// [`UiExtension::load`], using and filling a precompiled cache. Compiling runs on the
    /// background executor and instantiation on tokio, so no window waits on either.
    pub fn load_with_cache(
        extension_id: Arc<str>,
        wasm_bytes: Vec<u8>,
        cache: Option<PrecompiledCache>,
        cx: &AsyncApp,
    ) -> Task<Result<(Self, mpsc::UnboundedReceiver<PushedUpdate>)>> {
        Self::load_with_options(
            extension_id,
            wasm_bytes,
            LoadOptions {
                cache,
                ..LoadOptions::default()
            },
            cx,
        )
    }

    /// [`UiExtension::load`] with a precompiled cache, a permission broker, the declared
    /// capabilities and staging.
    pub fn load_with_options(
        extension_id: Arc<str>,
        wasm_bytes: Vec<u8>,
        options: LoadOptions,
        cx: &AsyncApp,
    ) -> Task<Result<(Self, mpsc::UnboundedReceiver<PushedUpdate>)>> {
        let LoadOptions {
            cache,
            capability_check,
            capabilities,
            allow_staged,
        } = options;
        let allow_staged = allow_staged.unwrap_or_else(staged_api_allowed_by_default);
        let capabilities: Arc<[String]> = match capabilities {
            Some(capabilities) => capabilities.into(),
            None => ALL_UI_CAPABILITIES
                .iter()
                .map(|&capability| capability.into())
                .collect(),
        };
        let diagnostics = diagnostics::DiagnosticsBuffer::default();
        if let Err(error) = validate_id("extension", &extension_id) {
            return Task::ready(Err(error));
        }
        let compile_task = cx.background_executor().spawn({
            let extension_id = extension_id.clone();
            async move {
                let runtime = ui_runtime()?;
                let section_version = parse_api_version(&extension_id, &wasm_bytes)?;
                let (component, from_precompiled) =
                    compile_component(&runtime.engine, &wasm_bytes, cache.as_ref())?;
                let api_version = section_version
                    .or_else(|| api_version_from_imports(&component, &runtime.engine))
                    .with_context(|| {
                        format!(
                            "extension {extension_id} is not a fyx:ui component: it has no \
                             {API_VERSION_SECTION} section and imports no fyx:ui interface"
                        )
                    })?;
                let ui_api_version = known_api_version(&api_version).with_context(|| {
                    format!(
                        "extension {extension_id} targets fyx:ui {api_version}, this build \
                         supports 0.1.0, 0.2.0 and 0.3.0 (staged)"
                    )
                })?;
                anyhow::ensure!(
                    api_version_range(allow_staged).contains(&api_version),
                    "extension {extension_id} targets fyx:ui {api_version}, which is staged: this \
                     build loads it only in Dev and Nightly builds or as a dev extension, and \
                     otherwise supports {MIN_VERSION}..={MAX_FROZEN_VERSION}",
                );
                anyhow::Ok((api_version, ui_api_version, component, from_precompiled))
            }
        });

        cx.spawn(async move |cx| {
            let (api_version, ui_api_version, component, from_precompiled) = compile_task.await?;
            let (pushed_tx, pushed_rx) = mpsc::unbounded();

            let instantiate = {
                let extension_id = extension_id.clone();
                let diagnostics = diagnostics.clone();
                let capabilities = capabilities.clone();
                async move {
                    let runtime = ui_runtime()?;
                    let mut wasi = WasiCtxBuilder::new();
                    wasi.inherit_stderr();
                    deny_network(&mut wasi);
                    // No preopened directories, no environment variables, no arguments: the
                    // builder starts empty and only stderr is inherited.
                    let mut store = Store::new(
                        &runtime.engine,
                        UiState::new(
                            extension_id,
                            wasi.build(),
                            capability_check,
                            diagnostics.clone(),
                            effects::ApiState::new(ui_api_version, capabilities),
                        ),
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
                        UiBindings::instantiate(ui_api_version, &mut store, &component, runtime)
                            .await
                            .map_err(anyhow::Error::from)
                    })
                    .await
                    .context("failed to instantiate fyx:ui component")?;
                    store.data_mut().restart_budget(UI_INIT_BUDGET);
                    let surfaces = with_budget(UI_INIT_BUDGET, async {
                        extension
                            .call_surfaces(&mut store)
                            .await
                            .map_err(anyhow::Error::from)
                    })
                    .await
                    .context("fyx:ui surfaces() failed")?;
                    if ui_api_version == ApiVersion::V0_1_0 {
                        let surfaces: Vec<ui_ext::Surface> = surfaces
                            .iter()
                            .filter_map(dispatch::shim::surface)
                            .collect();
                        validate_surfaces(&surfaces).context("invalid surfaces()")?;
                    }
                    store
                        .data_mut()
                        .api
                        .register_static(&surfaces)
                        .context("invalid surfaces()")?;
                    let initial_effects: Arc<[HostEffect]> = store.data_mut().finish_call().into();
                    // Nothing replies to instantiation, so its pushes go to the channel; the
                    // receiver is still in hand here, so sending cannot fail.
                    for pushed in dispatch::shim::pushes(initial_effects.to_vec()) {
                        pushed_tx.unbounded_send(pushed).ok();
                    }
                    anyhow::Ok((extension, store, surfaces, initial_effects))
                }
            };
            let (mut extension, mut store, surfaces, initial_effects) =
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
                    ui_api_version,
                    capabilities,
                    surfaces: surfaces
                        .iter()
                        .filter_map(dispatch::shim::surface)
                        .collect(),
                    surfaces_v0_2: surfaces.into(),
                    initial_effects,
                    limits: ui_api_version.tree_limits(),
                    tx,
                    _task: Arc::new(task),
                    from_precompiled,
                    diagnostics,
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
        Self::load_from_dir_with_options(extension_dir, LoadOptions::default(), fs, cx).await
    }

    /// [`UiExtension::load_from_dir`] with a precompiled cache, a permission broker and staging.
    /// The declared capabilities come from the manifest; `options.capabilities` is ignored.
    pub async fn load_from_dir_with_options(
        extension_dir: &Path,
        options: LoadOptions,
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
        Self::load_checked(extension_id, manifest, wasm_bytes, options, cx)
            .await
            .map(Some)
    }

    /// Loads `wasm_bytes` as the component `manifest` describes, with the manifest's checks: its
    /// capabilities apply and its API version must be the component's. For a caller that read
    /// the manifest and the bytes itself, once, so what it checked (consent) is what loads
    /// (triage-3, SEC T1).
    pub async fn load_checked(
        extension_id: Arc<str>,
        manifest: UiManifest,
        wasm_bytes: Vec<u8>,
        options: LoadOptions,
        cx: &AsyncApp,
    ) -> Result<(Self, mpsc::UnboundedReceiver<PushedUpdate>)> {
        let options = LoadOptions {
            capabilities: Some(manifest.capabilities.clone()),
            ..options
        };
        let (extension, pushed) =
            Self::load_with_options(extension_id.clone(), wasm_bytes, options, cx)
                .await
                .with_context(|| format!("loading fyx:ui extension {extension_id}"))?;
        anyhow::ensure!(
            extension.api_version == manifest.api_version,
            "extension {extension_id}: extension.toml says fyx:ui {}, the component targets {}",
            manifest.api_version,
            extension.api_version,
        );
        Ok((extension, pushed))
    }

    pub fn extension_id(&self) -> &Arc<str> {
        &self.extension_id
    }

    pub fn api_version(&self) -> &Version {
        &self.api_version
    }

    /// What the Extension Inspector shows: recent calls, `log` output and capability checks.
    pub fn diagnostics(&self) -> diagnostics::Diagnostics {
        self.diagnostics.snapshot()
    }

    /// Whether the component came from a precompiled cache rather than a fresh compile.
    pub fn from_precompiled(&self) -> bool {
        self.from_precompiled
    }

    /// What `surfaces` returned right after instantiation, in the 0.1 model: a 0.2 component's
    /// modals are left out ([`UiExtension::surfaces_v0_2`] has them).
    pub fn surfaces(&self) -> &[ui_ext::Surface] {
        &self.surfaces
    }

    pub fn limits(&self) -> TreeLimits {
        self.limits
    }

    /// Runs one guest call on the extension's queue and returns its result with the effects the
    /// guest made during it, in order. A failed call (trap, budget) disables the extension and
    /// drops them.
    async fn call<T, F>(
        &self,
        kind: &'static str,
        surface_id: &str,
        budget: CallBudget,
        f: F,
    ) -> Result<(T, Vec<HostEffect>)>
    where
        T: 'static + Send,
        F: 'static
            + Send
            + for<'a> FnOnce(
                &'a mut UiBindings,
                &'a mut Store<UiState>,
            ) -> BoxFuture<'a, wasmtime::Result<T>>,
    {
        let (return_tx, return_rx) = oneshot::channel();
        let diagnostics = self.diagnostics.clone();
        let surface_id = surface_id.to_string();
        self.tx
            .unbounded_send(Box::new(move |extension, store| {
                async move {
                    let result = if let Some(reason) = &store.data().disabled {
                        Err(anyhow!("disabled after an earlier failure: {reason}"))
                    } else {
                        let budget = budget.resolve(store.data_mut());
                        store.data_mut().begin_call(budget);
                        store.set_epoch_deadline(1);
                        let result = with_budget(budget, async {
                            f(extension, store).await.map_err(anyhow::Error::from)
                        })
                        .await;
                        let effects = store.data_mut().finish_call();
                        diagnostics.call(diagnostics::CallRecord {
                            kind,
                            surface_id,
                            at: std::time::SystemTime::now(),
                            duration: store.data().call_started.elapsed(),
                            budget,
                            pushes: effects
                                .iter()
                                .filter(|effect| matches!(effect, HostEffect::Push { .. }))
                                .count(),
                            error: result.as_ref().err().map(|error| format!("{error:#}")),
                        });
                        // A trap or a dropped call, the time budget included, leaves the instance
                        // unusable.
                        if let Err(error) = &result {
                            store.data_mut().disabled = Some(format!("{error:#}"));
                        }
                        result.map(|value| (value, effects))
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

pub mod diagnostics;
pub mod permissions;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_v0_2;
#[cfg(test)]
mod tests_v0_3;
