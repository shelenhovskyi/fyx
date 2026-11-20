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
    pin::pin,
    sync::{
        Arc, Condvar, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use fs::Fs;
use futures::{
    FutureExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
    future::BoxFuture,
};
use gpui::{AsyncApp, Task};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use ui_ext::{ApiVersion, TreeLimits, integration, terminal};
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

mod consent_items;
mod conversions;
mod dispatch;
pub mod edit;
mod effects;
mod fyx_ui_v0_2;
mod fyx_ui_v0_3;
mod fyx_ui_v0_4;
pub mod images;
pub mod journal;
// The WIT imports, the load options and revocation wire it next (plan 5.11).
mod accounts;
#[cfg_attr(not(test), allow(dead_code))]
mod editor_context;
mod extension_marks;
mod extension_settings;
mod listeners;
pub mod network;
pub mod observe;
pub mod process;
mod proposals;
mod relay;
mod spawn_sandbox;
pub use accounts::set_account_dirs;
mod stderr;
mod terminals;
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
/// The newest version this build knows: the staged 0.4 (plan P5).
pub const MAX_VERSION: Version = Version::new(0, 4, 0);
/// The newest frozen version: 0.3 (integration-6, plan 4.9). Newer ones are staged: they load only
/// where [`LoadOptions::allow_staged`] allows them (docs/fyx/wit-0.2/migration.md, section 2).
/// Freezing a version names it here explicitly; never set this to [`MAX_VERSION`], which may be a
/// staged world (0.4 now).
pub const MAX_FROZEN_VERSION: Version = Version::new(0, 3, 0);

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
/// Linear memory one UI component may grow to, all its memories together (a component and its
/// adapters have a few; triage-3, host memory): room for a JS engine (a componentize-js guest
/// embeds one) as well as a tree mirror and state. It also bounds each string and list one host
/// call copies out of the guest; [`MAX_LIFTED_TEXT_PER_CALL`] bounds their sum per call.
pub const UI_MEMORY_LIMIT: usize = 256 << 20;
/// Entries in an instance's WASI resource table (streams, pollables, handles). wasmtime's default
/// is a million; a UI component needs a few dozen.
pub const UI_RESOURCE_TABLE_LIMIT: usize = 4096;
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
const ALL_UI_CAPABILITIES: [&str; 7] = [
    CAPABILITY_PANEL,
    CAPABILITY_STATUS_BAR,
    CAPABILITY_MODAL,
    CAPABILITY_EDITOR_DECORATIONS,
    ui_ext::image::CAPABILITY_IMAGES,
    ui_ext::canvas::CAPABILITY_CANVAS,
    ui_ext::v0_2::CAPABILITY_NOTIFICATIONS,
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
    /// `process:spawn`: the programs it may run (spec decision 24, design.md 0.3 §0.6).
    #[serde(default)]
    pub processes: Vec<ProcessSpec>,
    /// `[fyx_ui.process]`: what every program gets.
    #[serde(default)]
    pub process: Option<ProcessManifest>,
    /// `name` from the top of `extension.toml`, as its author wrote it: check it before showing
    /// it (the provenance line of notifications, design.md M.4).
    #[serde(skip)]
    pub name: Option<String>,
    /// `version` from the top of `extension.toml`, unchecked; for the `user-agent` of
    /// `network:fetch`.
    #[serde(skip)]
    pub version: Option<String>,
    /// 0.3, with `network:fetch`: the hosts the extension may fetch from (design.md §8).
    #[serde(default)]
    pub network: Option<NetworkManifest>,
    /// 0.4: `[[fyx_ui.terminal_profiles]]` (design §B.1).
    #[serde(default)]
    pub terminal_profiles: Vec<ui_ext::terminal::TerminalProfileSpec>,
    /// 0.4: `[[fyx_ui.terminal_programs]]`, for `terminal:create` (§B.3).
    #[serde(default)]
    pub terminal_programs: Vec<ui_ext::terminal::TerminalProgramSpec>,
    /// 0.4: `[fyx_ui.terminal_env]`, for `terminal:env` (§B.4).
    #[serde(default)]
    pub terminal_env: Option<ui_ext::terminal::TerminalEnvSpec>,
    /// 0.4: `[[fyx_ui.listeners]]`, for `network:listen` (§B.7).
    #[serde(default)]
    pub listeners: Vec<ui_ext::integration::ListenerSpec>,
    /// 0.4: `[[fyx_ui.discovery_files]]`, for `fs:discovery-file` (§B.8).
    #[serde(default)]
    pub discovery_files: Vec<ui_ext::integration::DiscoveryFileSpec>,
    /// 0.4: `[[fyx_ui.relay_channels]]`, for `relay:hooks` (§B.9).
    #[serde(default)]
    pub relay_channels: Vec<ui_ext::integration::RelayChannelSpec>,
    /// 0.4: `[[fyx_ui.settings]]`, the extension's settings (design §D.1, plan 6.1); no
    /// capability. Checked by [`UiManifest::settings_specs`].
    #[serde(default)]
    pub settings: Vec<ui_ext::settings::SettingEntry>,
}

/// What each world's manifest may declare (wit-0.3/migration.md §2): a capability a world cannot
/// use is an error, not a warning, so the install dialog never asks consent for it.
fn world_capabilities(version: &Version) -> &'static [&'static str] {
    const V0_1: &[&str] = &[
        CAPABILITY_PANEL,
        CAPABILITY_STATUS_BAR,
        // Accepted, unused: the 0.2 migration declares them before the component moves.
        CAPABILITY_MODAL,
        CAPABILITY_EDITOR_DECORATIONS,
    ];
    const V0_2: &[&str] = &[
        CAPABILITY_PANEL,
        CAPABILITY_STATUS_BAR,
        CAPABILITY_MODAL,
        CAPABILITY_EDITOR_DECORATIONS,
        ui_ext::v0_2::CAPABILITY_NOTIFICATIONS,
    ];
    // The smaller 0.3 (owner, 2026-11-19): pickers, hover, code lenses, virtual documents,
    // commands and document context are 0.4's, so their capabilities are refused here.
    const V0_3: &[&str] = &[
        CAPABILITY_PANEL,
        CAPABILITY_STATUS_BAR,
        CAPABILITY_MODAL,
        CAPABILITY_EDITOR_DECORATIONS,
        ui_ext::v0_2::CAPABILITY_NOTIFICATIONS,
        ui_ext::image::CAPABILITY_IMAGES,
        ui_ext::canvas::CAPABILITY_CANVAS,
        network::CAPABILITY_NETWORK,
        process::CAPABILITY_PROCESS,
        edit::CAPABILITY_EDIT,
        observe::CAPABILITY_OBSERVE,
    ];
    // Staged (plan P5): 0.3 plus the terminal and integration capabilities of
    // docs/fyx/wit-0.4/design.md, T3 included (§C.3); plan 4.2-4.4's join with their WIT.
    const V0_4: &[&str] = &[
        CAPABILITY_PANEL,
        CAPABILITY_STATUS_BAR,
        CAPABILITY_MODAL,
        CAPABILITY_EDITOR_DECORATIONS,
        ui_ext::v0_2::CAPABILITY_NOTIFICATIONS,
        ui_ext::image::CAPABILITY_IMAGES,
        ui_ext::canvas::CAPABILITY_CANVAS,
        network::CAPABILITY_NETWORK,
        process::CAPABILITY_PROCESS,
        edit::CAPABILITY_EDIT,
        observe::CAPABILITY_OBSERVE,
        terminal::CAPABILITY_PROFILES,
        terminal::CAPABILITY_PSEUDO,
        terminal::CAPABILITY_CREATE,
        terminal::CAPABILITY_ENV,
        terminal::CAPABILITY_STATUS,
        terminal::CAPABILITY_OBSERVE,
        terminal::CAPABILITY_READ,
        terminal::CAPABILITY_WRITE,
        terminal::CAPABILITY_PACKAGE_FILES,
        integration::CAPABILITY_NETWORK_LISTEN,
        integration::CAPABILITY_DISCOVERY_FILE,
        integration::CAPABILITY_RELAY_HOOKS,
        integration::CAPABILITY_REVIEW_PROPOSE,
        integration::CAPABILITY_CONTEXT_EDITOR,
        integration::CAPABILITY_WORKSPACE_OPEN,
        integration::CAPABILITY_WORKSPACE_DIAGNOSTICS,
        integration::CAPABILITY_DIAGNOSTICS_PUBLISH,
        integration::CAPABILITY_FILE_DECORATIONS,
        integration::CAPABILITY_APPROVALS,
    ];
    if *version >= Version::new(0, 4, 0) {
        V0_4
    } else if *version >= Version::new(0, 3, 0) {
        V0_3
    } else if *version >= Version::new(0, 2, 0) {
        V0_2
    } else {
        V0_1
    }
}

/// `[fyx_ui.network]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkManifest {
    pub domains: Vec<String>,
    /// 0.4 (design wit-0.4 §C.2): the author's one line under the item at install, required
    /// there and refused below 0.4.
    #[serde(default)]
    pub reason: Option<String>,
}

/// One `[[fyx_ui.processes]]` entry, shared with the harness.
pub use ui_ext::process::ProcessSpec;

/// `[fyx_ui.process]`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessManifest {
    /// Environment variables passed through besides `PATH`, `HOME`/`USERPROFILE` and `LANG`.
    #[serde(default)]
    pub env: Vec<String>,
}

/// The consent item of a network domain: an update that adds one is held (decision 21).
pub fn domain_consent_item(domain: &str) -> String {
    format!("network:fetch@{domain}")
}

/// The consent item of a program.
pub fn process_consent_item(program: &str) -> String {
    format!("process:spawn@{program}")
}

impl UiManifest {
    /// What the user consents to: the capabilities, and each domain, program, argument pattern
    /// and environment name, so an update that adds any of them is held until it is approved
    /// (design.md 0.3 §0.6).
    pub fn consent_items(&self) -> Vec<String> {
        let domains = self
            .network
            .iter()
            .flat_map(|network| network.domains.iter())
            .map(|domain| domain_consent_item(domain));
        let programs = self.processes.iter().flat_map(|process| {
            let program = match &process.path {
                Some(path) => format!("{} ({path})", process.name),
                None => process.name.clone(),
            };
            std::iter::once(process_consent_item(&program)).chain(
                process
                    .args
                    .iter()
                    // Its own prefix, so a pattern is not shown as one more program.
                    .map(move |pattern| format!("process:args@{} {pattern}", process.name)),
            )
        });
        let env = self
            .process
            .iter()
            .flat_map(|process| process.env.iter())
            .map(|name| format!("process:env@{name}"));
        self.capabilities
            .iter()
            .cloned()
            .chain(domains)
            .chain(programs)
            .chain(env)
            .collect()
    }

    /// What the manifest's world may declare (wit-0.3/migration.md §2): its capabilities, and the
    /// 0.3 tables (`network`, `processes`, `process`) only from 0.3 on.
    pub fn check_world(&self) -> Result<()> {
        let version = &self.api_version;
        let world = world_capabilities(version);
        for capability in &self.capabilities {
            if world.contains(&capability.as_str()) {
                continue;
            }
            anyhow::ensure!(
                permissions::tier_of(capability).is_some(),
                "unknown capability {capability:?}"
            );
            // The first world that has it.
            let needs = [
                ("0.2", Version::new(0, 2, 0)),
                ("0.3", Version::new(0, 3, 0)),
                ("0.4", Version::new(0, 4, 0)),
            ]
            .into_iter()
            .find(|(_, world)| world_capabilities(world).contains(&capability.as_str()))
            .map(|(needs, _)| needs);
            match needs {
                Some(needs) => bail!(
                    "capability {capability:?} needs fyx:ui {needs}; this extension targets \
                     {version}"
                ),
                None => bail!(
                    "capability {capability:?} is not in fyx:ui {version}; it is planned for a \
                     later version"
                ),
            }
        }
        if *version < Version::new(0, 3, 0) {
            for (present, table) in [
                (self.network.is_some(), "[fyx_ui.network]"),
                (!self.processes.is_empty(), "[[fyx_ui.processes]]"),
                (self.process.is_some(), "[fyx_ui.process]"),
            ] {
                anyhow::ensure!(
                    !present,
                    "{table} needs fyx:ui 0.3; this extension targets {version}"
                );
            }
        }
        if *version < Version::new(0, 4, 0) {
            for (present, table) in [
                (
                    !self.terminal_profiles.is_empty(),
                    "[[fyx_ui.terminal_profiles]]",
                ),
                (
                    !self.terminal_programs.is_empty(),
                    "[[fyx_ui.terminal_programs]]",
                ),
                (self.terminal_env.is_some(), "[fyx_ui.terminal_env]"),
                (!self.listeners.is_empty(), "[[fyx_ui.listeners]]"),
                (
                    !self.discovery_files.is_empty(),
                    "[[fyx_ui.discovery_files]]",
                ),
                (!self.relay_channels.is_empty(), "[[fyx_ui.relay_channels]]"),
                (!self.settings.is_empty(), "[[fyx_ui.settings]]"),
            ] {
                anyhow::ensure!(
                    !present,
                    "{table} needs fyx:ui 0.4; this extension targets {version}"
                );
            }
        }
        self.check_t2_reasons(version)
    }

    /// `reason` on `[[fyx_ui.processes]]` and `[fyx_ui.network]`: from 0.4 every T2 item has
    /// one (design wit-0.4 §C.2); below 0.4 the key does not exist.
    fn check_t2_reasons(&self, version: &Version) -> Result<()> {
        let items = self
            .processes
            .iter()
            .map(|spec| {
                (
                    format!("[[fyx_ui.processes]] {:?}", spec.name),
                    &spec.reason,
                )
            })
            .chain(
                self.network
                    .iter()
                    .map(|network| ("[fyx_ui.network]".to_string(), &network.reason)),
            );
        for (what, reason) in items {
            if *version < Version::new(0, 4, 0) {
                anyhow::ensure!(
                    reason.is_none(),
                    "{what}: reason needs fyx:ui 0.4; this extension targets {version}"
                );
            } else {
                let reason = reason
                    .as_deref()
                    .ok_or_else(|| anyhow!("{what}: a reason is required (fyx:ui 0.4)"))?;
                terminal::check_reason(&what, reason).map_err(anyhow::Error::msg)?;
            }
        }
        Ok(())
    }

    /// `[fyx_ui.terminal_env]`, checked as [`UiManifest::check_terminals`] does; `None` without
    /// one or when it does not pass.
    pub fn checked_terminal_env(&self) -> Option<terminal::TerminalEnv> {
        let integration = integration::IntegrationTables {
            listeners: &self.listeners,
            discovery_files: &self.discovery_files,
            relay_channels: &self.relay_channels,
        };
        let env = self.terminal_env.clone()?;
        terminal::check_terminal_env(env, &|name| integration.listener(name)).ok()
    }

    /// `[[fyx_ui.settings]]`, checked (design §D.1): the declarations the host computes the
    /// effective values from. Empty without the table.
    pub fn settings_specs(&self) -> Result<Vec<ui_ext::settings::SettingSpec>> {
        ui_ext::settings::check_settings(&self.settings).map_err(anyhow::Error::msg)
    }

    /// The 0.4 terminal and integration tables (design §B, plan 5.5): each goes with its
    /// capability, and their entries pass `ui_ext::terminal` and `ui_ext::integration`'s checks.
    pub fn check_terminals(&self) -> Result<()> {
        let declares = |capability: &str| self.capabilities.iter().any(|c| c == capability);
        let integration = integration::IntegrationTables {
            listeners: &self.listeners,
            discovery_files: &self.discovery_files,
            relay_channels: &self.relay_channels,
        };
        for (present, table, capability) in [
            (
                !self.terminal_profiles.is_empty(),
                "[[fyx_ui.terminal_profiles]]",
                terminal::CAPABILITY_PROFILES,
            ),
            (
                !self.terminal_programs.is_empty(),
                "[[fyx_ui.terminal_programs]]",
                terminal::CAPABILITY_CREATE,
            ),
            (
                self.terminal_env.is_some(),
                "[fyx_ui.terminal_env]",
                terminal::CAPABILITY_ENV,
            ),
            (
                !self.listeners.is_empty(),
                "[[fyx_ui.listeners]]",
                integration::CAPABILITY_NETWORK_LISTEN,
            ),
            (
                !self.discovery_files.is_empty(),
                "[[fyx_ui.discovery_files]]",
                integration::CAPABILITY_DISCOVERY_FILE,
            ),
            (
                !self.relay_channels.is_empty(),
                "[[fyx_ui.relay_channels]]",
                integration::CAPABILITY_RELAY_HOOKS,
            ),
        ] {
            match (present, declares(capability)) {
                (true, false) => bail!("{table} needs the {capability} capability"),
                (false, true) => bail!("{capability} needs {table}"),
                _ => {}
            }
        }
        terminal::check_profiles(self.terminal_profiles.clone()).map_err(anyhow::Error::msg)?;
        terminal::check_programs(&self.terminal_programs).map_err(anyhow::Error::msg)?;
        integration.check().map_err(anyhow::Error::msg)?;
        if let Some(env) = &self.terminal_env {
            terminal::check_terminal_env(env.clone(), &|name| integration.listener(name))
                .map_err(anyhow::Error::msg)?;
        }
        // `${package}` names the package's files (§B.6), so it needs `package:files`, as
        // `fyx-ext check` and the harness require.
        let uses_package = |text: &str, usage: terminal::TemplateUse| {
            terminal::Template::parse(text, usage, &|_| Some(true))
                .is_ok_and(|template| template.0.contains(&terminal::TemplatePart::Package))
        };
        let in_profiles = self.terminal_profiles.iter().any(|profile| {
            profile
                .args
                .iter()
                .any(|arg| uses_package(arg, terminal::TemplateUse::ProfileArg))
        });
        let in_env = self.terminal_env.iter().any(|env| {
            env.vars
                .values()
                .any(|value| uses_package(value, terminal::TemplateUse::EnvValue))
        });
        if (in_profiles || in_env) && !declares(terminal::CAPABILITY_PACKAGE_FILES) {
            bail!(
                "${{package}} in {} needs the {} capability",
                if in_profiles {
                    "[[fyx_ui.terminal_profiles]] args"
                } else {
                    "[fyx_ui.terminal_env]"
                },
                terminal::CAPABILITY_PACKAGE_FILES
            );
        }
        Ok(())
    }

    /// `[fyx_ui.network]` and `network:fetch` go together; the domains follow
    /// `network::DomainList::parse` (IDNA, no public-suffix wildcards, localhost only with a
    /// port).
    pub fn check_network(&self) -> Result<()> {
        let declares = self
            .capabilities
            .iter()
            .any(|capability| capability == network::CAPABILITY_NETWORK);
        match &self.network {
            Some(table) => {
                anyhow::ensure!(
                    declares,
                    "[fyx_ui.network] needs {:?} in capabilities",
                    network::CAPABILITY_NETWORK
                );
                network::DomainList::parse(&table.domains).map_err(anyhow::Error::msg)?;
            }
            None => anyhow::ensure!(
                !declares,
                "{:?} needs [fyx_ui.network] domains",
                network::CAPABILITY_NETWORK
            ),
        }
        Ok(())
    }

    /// The process rules (spec decision 24), checked when the manifest is read. W2c enforces
    /// them when a program runs (plan 4.7).
    pub fn check_processes(&self) -> Result<()> {
        let declares = self.capabilities.iter().any(|c| c == "process:spawn");
        let mut seen = std::collections::BTreeSet::new();
        for process in &self.processes {
            let name = &process.name;
            let plain = !name.is_empty()
                && name.len() <= 64
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                && !name.starts_with('.');
            if !plain {
                bail!("[[fyx_ui.processes]] name {name:?} must be a program name, not a path");
            }
            if !seen.insert(name.clone()) {
                bail!("[[fyx_ui.processes]] names {name:?} twice");
            }
            if let Some(path) = &process.path {
                let relative = Path::new(path);
                let inside = !path.is_empty()
                    && relative
                        .components()
                        .all(|component| matches!(component, PathComponent::Normal(_)));
                if !inside {
                    bail!(
                        "[[fyx_ui.processes]] {name}: path {path:?} must be relative, inside the worktree"
                    );
                }
            }
            for pattern in &process.args {
                if pattern.is_empty() || pattern.chars().any(char::is_control) {
                    bail!("[[fyx_ui.processes]] {name}: argument pattern {pattern:?} is not valid");
                }
            }
        }
        for variable in self.process.iter().flat_map(|process| process.env.iter()) {
            let valid = !variable.is_empty()
                && variable
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                bail!("[fyx_ui.process] env {variable:?} is not a variable name");
            }
            // Names that load code into a program (`LD_PRELOAD`, `NODE_OPTIONS`, …) are refused
            // even when declared (design.md §9).
            ui_ext::process::check_env_name(variable)
                .map_err(|error| anyhow::anyhow!("[fyx_ui.process] env: {error}"))?;
        }
        let has_table = !self.processes.is_empty() || self.process.is_some();
        if has_table && !declares {
            bail!("[[fyx_ui.processes]] needs the process:spawn capability");
        }
        if declares && self.processes.is_empty() {
            bail!("process:spawn needs [[fyx_ui.processes]]");
        }
        Ok(())
    }
}

/// Only `[fyx_ui].api_version`, read before anything else so that a manifest for a newer world
/// fails with its version rather than with an unknown field (wit-0.3/migration.md §2).
#[derive(Deserialize)]
struct ApiVersionOnly {
    api_version: Version,
}

#[derive(Deserialize)]
struct ExtensionToml {
    id: Arc<str>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    fyx_ui: Option<toml::Table>,
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
        let Some(table) = toml.fyx_ui else {
            return Ok(None);
        };
        let id = toml.id;
        validate_id("extension", &id).with_context(|| format!("in {path:?}"))?;
        // The version first, then the manifest of that world (wit-0.3/migration.md §2).
        let ApiVersionOnly { api_version } = ApiVersionOnly::deserialize(table.clone())
            .with_context(|| format!("extension {id}: [fyx_ui].api_version, in {path:?}"))?;
        anyhow::ensure!(
            is_supported_api_version(&api_version),
            "extension {id} targets fyx:ui {api_version}, this build supports 0.1.0, 0.2.0, 0.3.0 and the staged \
             0.4.0",
        );
        let mut manifest = UiManifest::deserialize(table)
            .with_context(|| format!("parsing [fyx_ui] of {path:?}"))?;
        manifest.name = toml.name;
        manifest.version = toml.version;
        manifest
            .check_world()
            .with_context(|| format!("extension {id}, in {path:?}"))?;
        manifest
            .check_network()
            .with_context(|| format!("extension {id}, in {path:?}"))?;
        manifest
            .check_processes()
            .with_context(|| format!("extension {id}, in {path:?}"))?;
        manifest
            .check_terminals()
            .with_context(|| format!("extension {id}, in {path:?}"))?;
        manifest
            .settings_specs()
            .with_context(|| format!("extension {id}, in {path:?}"))?;
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
    limits: UiLimits,
    call_started: Instant,
    call_budget: Duration,
    /// Epochs the running call has executed in, as time: what its budget counts.
    executed: Duration,
    /// Surfaces `mount` has been called for, so only the first mount gets
    /// [`UI_FIRST_MOUNT_BUDGET`].
    mounted: std::collections::HashSet<String>,
    disabled: Option<String>,
    capability_check: Option<Arc<dyn permissions::CapabilityCheck>>,
    diagnostics: diagnostics::DiagnosticsBuffer,
    /// Live ids, declared capabilities, shown modals and the effect list of the 0.2 imports.
    api: effects::ApiState,
    /// The 0.3 `network` import's ids, counters and the requests of the running call.
    network: network::NetworkState,
    /// The 0.3 `process` import's ids, counters and the spawns of the running call.
    process: process::ProcessState,
    /// The 0.3 `workspace-edit` import's ids, counters and the edits of the running call.
    edit: edit::EditState,
    /// What the running call asked of the 0.3 `agent-observe` feed.
    observe: observe::ObserveState,
    /// The 0.4 `terminals` import: the extension's own terminals.
    terminals: terminals::TerminalState,
    /// The 0.4 `listeners` import (§B.7).
    listeners: Arc<listeners::ListenersHandle>,
    /// The 0.4 `context` import (§B.11).
    context: Arc<editor_context::ContextHandle>,
    /// The 0.4 `review` import (§B.10).
    review: Arc<proposals::ReviewHandle>,
    /// The 0.4 `relay` import (§B.9).
    relay: Arc<relay::RelayHandle>,
    /// The 0.4 `settings` import (§D.4).
    settings: Arc<extension_settings::SettingsHandle>,
    /// The 0.4 `context` marks: published diagnostics and file decorations (§E, §F.2).
    marks: Arc<extension_marks::MarksHandle>,
    /// 0.4 (§C.4): where programs started and T3 use are recorded.
    journal: Option<Arc<dyn journal::Journal>>,
    /// 0.3 `permissions.query`: each T2 state as of the call's first query of it, so one call
    /// sees one answer (design.md §7).
    permission_mirror: std::collections::HashMap<&'static str, permissions::PermissionState>,
    /// 0.3 `permissions.request` in the running call: the prompts to ask for once it returns.
    permission_requests: Vec<&'static str>,
}

/// The 0.3 T2 state of a new [`UiState`].
struct IoStates {
    network: network::NetworkState,
    process: process::ProcessState,
    edit: edit::EditState,
    observe: observe::ObserveState,
    terminals: terminals::TerminalState,
    listeners: Arc<listeners::ListenersHandle>,
    context: Arc<editor_context::ContextHandle>,
    review: Arc<proposals::ReviewHandle>,
    relay: Arc<relay::RelayHandle>,
    settings: Arc<extension_settings::SettingsHandle>,
    marks: Arc<extension_marks::MarksHandle>,
    journal: Option<Arc<dyn journal::Journal>>,
}

impl UiState {
    /// For the T2 imports: the broker's state of `capability`, `Unavailable` without a broker.
    /// Recorded for the Extension Inspector like [`UiState::check_capability`].
    pub fn permission_state(&self, capability: &str) -> permissions::PermissionState {
        let state = self
            .capability_check
            .as_ref()
            .map_or(permissions::PermissionState::Unavailable, |broker| {
                broker.state(&self.extension_id, capability)
            });
        self.diagnostics.capability_check(
            capability,
            match state {
                permissions::PermissionState::Granted | permissions::PermissionState::Ask => None,
                other => Some(format!("{other:?}")),
            },
        );
        state
    }

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
        io: IoStates,
    ) -> Self {
        let IoStates {
            network,
            process,
            edit,
            observe,
            terminals,
            listeners,
            context,
            review,
            relay,
            settings,
            marks,
            journal,
        } = io;
        Self {
            edit,
            observe,
            terminals,
            listeners,
            context,
            review,
            relay,
            settings,
            marks,
            journal,
            capability_check,
            diagnostics: diagnostics.clone(),
            api,
            network,
            process,
            extension_id,
            table: {
                let mut table = ResourceTable::new();
                table.set_max_capacity(UI_RESOURCE_TABLE_LIMIT);
                table
            },
            ctx,
            pushes: Vec::new(),
            pushed_items: 0,
            lifted_text: 0,
            limits: UiLimits::reporting_to(diagnostics),
            call_started: Instant::now(),
            call_budget: UI_INIT_BUDGET,
            executed: Duration::ZERO,
            mounted: Default::default(),
            disabled: None,
            permission_mirror: Default::default(),
            permission_requests: Vec::new(),
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
        self.executed = Duration::ZERO;
        self.pushes.clear();
        self.pushed_items = 0;
        self.lifted_text = 0;
        self.permission_mirror.clear();
        self.permission_requests.clear();
        self.api.begin_call();
    }

    /// `permissions.query`: `unavailable` unless the manifest declares it, else the broker's
    /// state as of this call's first query of it.
    fn query_permission(&mut self, capability: &'static str) -> permissions::PermissionState {
        if !self.api.declares(capability) {
            return permissions::PermissionState::Unavailable;
        }
        if let Some(state) = self.permission_mirror.get(capability) {
            return *state;
        }
        let state = self.permission_state(capability);
        self.permission_mirror.insert(capability, state);
        state
    }

    /// The manifest and the broker for a T2 capability at every call: `ask` is accepted (the
    /// prompt comes when the operation is applied), `denied` and `unavailable` refuse.
    fn permit_t2(&self, capability: &str) -> Result<(), ui_ext::v0_2::HostError> {
        use ui_ext::v0_2::{HostError, HostErrorCode};
        if !self.api.declares(capability) {
            return Err(HostError {
                code: HostErrorCode::NotPermitted,
                message: format!("{capability:?} is not in the manifest"),
            });
        }
        match self.permission_state(capability) {
            permissions::PermissionState::Granted | permissions::PermissionState::Ask => Ok(()),
            state => Err(HostError {
                code: HostErrorCode::Denied,
                message: format!("{capability} is {state:?}"),
            }),
        }
    }

    /// A T1 capability of the 0.4 `terminals` import: declared, and granted by the broker.
    fn permit_t1(&self, capability: &str) -> Result<(), ui_ext::v0_2::HostError> {
        if !self.api.declares(capability) {
            return Err(ui_ext::v0_2::HostError {
                code: ui_ext::v0_2::HostErrorCode::NotPermitted,
                message: format!("{capability:?} is not in the manifest"),
            });
        }
        self.permit(capability)
    }

    /// 0.4 (§C.2): whether the broker turned off one item of a granted capability, such as a
    /// single program. Without a broker nothing is granted, so the capability check refuses.
    fn item_denied(&self, item: &permissions::ConsentItem) -> bool {
        self.capability_check.as_ref().is_some_and(|broker| {
            matches!(
                broker.item_state(&self.extension_id, item),
                permissions::PermissionState::Denied | permissions::PermissionState::Unavailable
            )
        })
    }

    /// Records a journal entry, when the host keeps a journal.
    fn journal(&self, kind: journal::JournalKind) {
        if let Some(journal) = &self.journal {
            journal.record(&self.extension_id, kind);
        }
    }

    /// A T2 capability a synchronous 0.4 call needs now ([`terminals::t2_now`]).
    fn permit_t2_now(&self, capability: &'static str) -> Result<(), ui_ext::v0_2::HostError> {
        if !self.api.declares(capability) {
            return Err(ui_ext::v0_2::HostError {
                code: ui_ext::v0_2::HostErrorCode::NotPermitted,
                message: format!("{capability:?} is not in the manifest"),
            });
        }
        terminals::t2_now(
            self.capability_check.as_ref(),
            &self.extension_id,
            self.permission_state(capability),
            capability,
        )
    }

    /// Whether a declared T2 capability is granted now.
    fn t2_granted(&self, capability: &str) -> bool {
        self.api.declares(capability)
            && self.permission_state(capability) == permissions::PermissionState::Granted
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
    /// The staged 0.4 world, likewise.
    linker_v0_4: Linker<UiState>,
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

/// The store's resource limiter: [`ui_store_limits`] per memory and table, and
/// [`UI_MEMORY_LIMIT`] for all the instance's memories together, so eight memories cannot take
/// eight times the budget (triage-3, host memory). Memories only grow, so the sum of the growths
/// is what they hold.
pub(crate) struct UiLimits {
    inner: wasmtime::StoreLimits,
    memory_total: usize,
    /// Where the Inspector reads `memory_total` from; `None` in tests of the limiter alone.
    report: Option<diagnostics::DiagnosticsBuffer>,
}

impl UiLimits {
    pub(crate) fn new() -> Self {
        Self {
            inner: ui_store_limits(),
            memory_total: 0,
            report: None,
        }
    }

    fn reporting_to(diagnostics: diagnostics::DiagnosticsBuffer) -> Self {
        Self {
            report: Some(diagnostics),
            ..Self::new()
        }
    }
}

impl wasmtime::ResourceLimiter for UiLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if !self.inner.memory_growing(current, desired, maximum)? {
            return Ok(false);
        }
        let total = self.memory_total + desired.saturating_sub(current);
        if total > UI_MEMORY_LIMIT {
            // Like `trap_on_grow_failure`: a trap, which disables the extension, with the reason.
            wasmtime::bail!(
                "memory of all its instances would be {total} bytes, the limit is {UI_MEMORY_LIMIT}"
            );
        }
        self.memory_total = total;
        if let Some(report) = &self.report {
            report.memory(total);
        }
        Ok(true)
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.memory_grow_failed(error)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.inner.table_growing(current, desired, maximum)
    }

    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.table_grow_failed(error)
    }

    fn instances(&self) -> usize {
        self.inner.instances()
    }

    fn tables(&self) -> usize {
        self.inner.tables()
    }

    fn memories(&self) -> usize {
        self.inner.memories()
    }
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
    let mut linker_v0_4 = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker_v0_4)?;
    fyx_ui_v0_4::add_to_linker(&mut linker_v0_4)?;
    Ok(UiRuntime {
        engine,
        linker,
        linker_v0_2,
        linker_v0_3,
        linker_v0_4,
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

/// Wakers of calls waiting in [`with_budget`], woken on every epoch tick.
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

/// The waker a guest call is polled with: it notes when the call was first woken, so
/// [`with_budget`] can tell a call that waits on the host (not woken) from one that is ready but
/// not polled because the tokio workers are busy (woken).
struct RecordingWaker {
    woken_at: Mutex<Option<Instant>>,
    inner: Waker,
}

impl std::task::Wake for RecordingWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(Instant::now);
        self.inner.wake_by_ref();
    }
}

/// Runs a guest call with `budget`. The epoch callback interrupts a guest that keeps executing
/// longer than `budget`, counting only epochs it executed in (`UiState::executed`). This drops the
/// call when the guest is blocked in a host call instead (`wasi:io/poll` behind `thread::sleep`,
/// for example), where no epoch callback runs.
///
/// Both count the call's own time, not wall time (async-1 #5): a call that is ready but waits for
/// a busy tokio worker spends neither, so other work on the shared runtime, or other UI
/// extensions, cannot get a healthy extension disabled.
async fn with_budget<T>(budget: Duration, call: impl Future<Output = Result<T>>) -> Result<T> {
    // Every guest call runs in here, so this is what keeps the epoch thread ticking (triage-3,
    // PERF 7); a call waiting on the host counts too, since the epoch thread polls it again.
    let _active = ActiveCall::begin();
    let limit = budget + UI_DEADLINE_SLACK;
    let mut call = pin!(call);
    let mut blocked = Duration::ZERO;
    let mut waiting: Option<(Instant, Arc<RecordingWaker>)> = None;
    futures::future::poll_fn(move |cx| {
        let now = Instant::now();
        if let Some((since, waker)) = waiting.take() {
            let woken_at = *waker
                .woken_at
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let until = woken_at.map_or(now, |at| at.clamp(since, now));
            blocked += until.saturating_duration_since(since);
        }
        if blocked > limit {
            return Poll::Ready(Err(anyhow!(
                "exceeded its time budget of {budget:?} while blocked in a host call"
            )));
        }
        let waker = Arc::new(RecordingWaker {
            woken_at: Mutex::new(None),
            inner: cx.waker().clone(),
        });
        let result = call
            .as_mut()
            .poll(&mut Context::from_waker(&Waker::from(waker.clone())));
        if result.is_pending() {
            waiting = Some((Instant::now(), waker));
            // The epoch thread polls the call again, so time blocked in the host is counted.
            DEADLINE_WAKERS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(cx.waker().clone());
        }
        result
    })
    .await
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
    /// `extension.toml`'s `name`, unchecked; `None` without a manifest.
    name: Option<Arc<str>>,
    /// Starts `network.fetch` requests after their call and hands back the outcomes.
    network: Arc<network::NetworkHandle>,
    /// Starts `process.spawn` programs after their call; dropping the last clone kills them.
    processes: Arc<process::ProcessHandle>,
    /// Starts `workspace-edit.apply-edit` edits after their call.
    edits: Arc<edit::EditHandle>,
    /// The `agent-observe` feed; dropping the last clone stops it.
    observer: Arc<observe::ObserveHandle>,
    /// The 0.4 terminals; dropping the last clone closes them all.
    terminals: Arc<terminals::TerminalsHandle>,
    /// The 0.4 listeners; they close when the instance goes.
    listeners: Arc<listeners::ListenersHandle>,
    /// The 0.4 editor context; its feed stops when the instance goes.
    context: Arc<editor_context::ContextHandle>,
    /// The 0.4 proposals; their reviews close when the instance goes.
    review: Arc<proposals::ReviewHandle>,
    /// The 0.4 relay channels; unanswered requests fail open when the instance goes.
    relay: Arc<relay::RelayHandle>,
    /// The 0.4 extension settings (design §D): declarations and effective values.
    settings: Arc<extension_settings::SettingsHandle>,
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
    /// Whether a staged version (none since 0.3 is frozen) may load; `None` means
    /// [`staged_api_allowed_by_default`].
    pub allow_staged: Option<bool>,
    /// `[fyx_ui.network] domains`; [`UiExtension::load_checked`] sets it from the manifest.
    /// Applies only to a 0.3 component that declares `network:fetch`.
    pub network_domains: Option<Vec<String>>,
    /// `extension.toml`'s `version`, for the `user-agent` of `network:fetch`; set by
    /// [`UiExtension::load_checked`].
    pub extension_version: Option<String>,
    /// Fyx's `proxy` setting for `network:fetch`, kept current by the host; `None` goes direct.
    /// Environment proxies are never used.
    pub proxy: Option<network::ProxySetting>,
    /// `fyx.extensions.network.private_hosts` for `network:fetch`, kept current by the host;
    /// `None` lists no name.
    pub private_hosts: Option<network::PrivateHosts>,
    /// Name resolution for `network:fetch`; `None` is the system resolver. For tests.
    pub resolver: Option<Arc<dyn network::Resolve>>,
    /// `[[fyx_ui.processes]]`; [`UiExtension::load_checked`] sets it from the manifest. Applies
    /// only to a 0.3 component that declares `process:spawn`.
    pub processes: Option<Vec<process::ProcessSpec>>,
    /// `[fyx_ui.process] env`; set by [`UiExtension::load_checked`].
    pub process_env: Option<Vec<String>>,
    /// The host's open worktrees, for `process:spawn` (`cwd`, `path` programs, the `PATH`
    /// filter). Without it, `spawn` is `unavailable`.
    pub workspace: Option<Arc<dyn process::Workspace>>,
    /// Fyx's environment as `process:spawn` sees it; `None` is the real one. For tests.
    pub process_environment: Option<process::Environment>,
    /// The editor side of `workspace:edit`. Without it, `apply-edit` is `unavailable`.
    pub workspace_edits: Option<Arc<dyn edit::WorkspaceEdits>>,
    /// The agent reviews for `agent:observe`. Without them, `start` is `unavailable`.
    pub agent_reviews: Option<Arc<dyn observe::AgentReviews>>,
    /// The terminal host for the 0.4 `terminals` import (`terminal_host`, W1). Without it, the
    /// import is `unavailable`.
    pub terminals: Option<Arc<dyn terminal::TerminalHost>>,
    /// `[[fyx_ui.terminal_programs]]`; set by [`UiExtension::load_checked`]. Applies only to a
    /// 0.4 component that declares `terminal:create`.
    pub terminal_programs: Option<Vec<terminal::TerminalProgramSpec>>,
    /// `[fyx_ui.terminal_env]`, checked; set by [`UiExtension::load_checked`].
    pub terminal_env: Option<terminal::TerminalEnv>,
    /// `[[fyx_ui.terminal_profiles]]`, checked; set by [`UiExtension::load_checked`]. Sent to the
    /// terminal host (P2.4) with `terminal:profiles`.
    pub terminal_profiles: Option<Vec<terminal::TerminalProfile>>,
    /// The extension's directory, for `${package}` in `terminal:env`; set by
    /// [`UiExtension::load_from_dir_with_options`].
    pub package_dir: Option<PathBuf>,
    /// 0.4 (design §C.4): the usage journal. Without one nothing is recorded.
    pub journal: Option<Arc<dyn journal::Journal>>,
    /// `[[fyx_ui.listeners]]`; set by [`UiExtension::load_checked`]. Applies only to a 0.4
    /// component that declares `network:listen` (§B.7).
    pub listeners: Option<Vec<ui_ext::integration::ListenerSpec>>,
    /// `[[fyx_ui.discovery_files]]`; set by [`UiExtension::load_checked`]. Applies only when
    /// `fs:discovery-file` is declared too (§B.8).
    pub discovery_files: Option<Vec<ui_ext::integration::DiscoveryFileSpec>>,
    /// Told when a listener opens, changes its granted discovery files, and closes: the
    /// discovery-file writer (§B.8, plan 5.12, W1) and the profiles' `${listen.*}`.
    pub listener_observer: Option<Arc<dyn ui_ext::integration::ListenerObserver>>,
    /// The editor side of the 0.4 `context` import (§B.11). Without it, the import is
    /// `unavailable`.
    pub context: Option<Arc<dyn ui_ext::context::ContextHost>>,
    /// The review bridge of the 0.4 `review` import (§B.10). Without it, the import is
    /// `unavailable`.
    pub review: Option<Arc<dyn ui_ext::review::ReviewHost>>,
    /// The language-server side of the 0.4 `context.diagnostics` and `watch-diagnostics`
    /// (§B.11). Without it, they are `unavailable`.
    pub diagnostics_source: Option<Arc<dyn ui_ext::context::DiagnosticsSource>>,
    /// `[[fyx_ui.relay_channels]]`; set by [`UiExtension::load_checked`]. Applies only to a 0.4
    /// component that declares `relay:hooks` (§B.9).
    pub relay_channels: Option<Vec<ui_ext::integration::RelayChannelSpec>>,
    /// The `fyx-relay` binary, for `${relay}` in `terminal:env` values (§B.9); W1's glue fills
    /// it from `terminal_host::relay_binary`. Without it such a value is left out.
    pub relay_binary: Option<PathBuf>,
    /// `[[fyx_ui.settings]]`, checked; set by [`UiExtension::load_checked`]. Applies only to a
    /// 0.4 component (design §D).
    pub settings: Option<Vec<ui_ext::settings::SettingSpec>>,
    /// The extension's object under `fyx_extensions.<id>` in the **user** settings file at load,
    /// never project settings (§D.2). Later changes go to [`UiExtension::set_user_settings`].
    pub user_settings: Option<ui_ext::settings::RawValue>,
    /// 0.4 `diagnostics:publish` (§E): where published diagnostics go. Without it the calls
    /// are `unavailable`.
    pub diagnostics_publisher: Option<Arc<dyn ui_ext::published_diagnostics::DiagnosticsPublisher>>,
    /// 0.4 `workspace:file-decorations` (§F.2): the project panel's decorations. Without it the
    /// calls are `unavailable`.
    pub file_decorations: Option<Arc<dyn ui_ext::file_decorations::FileDecorationsHost>>,
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
            network_domains,
            extension_version,
            proxy,
            private_hosts,
            resolver,
            processes,
            process_env,
            workspace,
            process_environment,
            workspace_edits,
            agent_reviews,
            terminals: terminal_host,
            terminal_programs,
            terminal_env,
            terminal_profiles,
            package_dir,
            journal,
            listeners: listener_specs,
            discovery_files,
            listener_observer,
            context: context_host,
            review: review_host,
            diagnostics_source,
            relay_channels,
            relay_binary,
            settings: setting_specs,
            user_settings,
            diagnostics_publisher,
            file_decorations,
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
        let declares_network = capabilities
            .iter()
            .any(|capability| capability == network::CAPABILITY_NETWORK);
        let domains = match network_domains {
            Some(domains) if declares_network => match network::DomainList::parse(&domains) {
                Ok(domains) => domains,
                Err(error) => return Task::ready(Err(anyhow::Error::msg(error))),
            },
            _ => network::DomainList::default(),
        };
        let declares_process = capabilities
            .iter()
            .any(|capability| capability == process::CAPABILITY_PROCESS);
        let process_policy = if declares_process {
            process::ProcessPolicy {
                programs: processes.unwrap_or_default(),
                env: process_env.unwrap_or_default().into_iter().collect(),
            }
        } else {
            process::ProcessPolicy::default()
        };
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
                         supports 0.1.0, 0.2.0, 0.3.0 and the staged 0.4.0"
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
            // Read here: `load_with_options` may be called while the app is borrowed.
            let app_version = cx.update(|cx| release_channel::AppVersion::global(cx));
            let fetcher = network::Fetcher::new(
                network::user_agent(&app_version, &extension_id, extension_version.as_deref()),
                resolver,
            )
            .with_proxy(proxy.unwrap_or_default())
            .with_private_hosts(private_hosts.unwrap_or_default());
            let (api_version, ui_api_version, component, from_precompiled) = compile_task.await?;
            let (pushed_tx, pushed_rx) = mpsc::unbounded();
            // Only a 0.3 or 0.4 component has the T2 imports.
            let has_io = ui_api_version >= ApiVersion::V0_3_0;
            let domains = Arc::new(if has_io {
                domains
            } else {
                network::DomainList::default()
            });
            let running = Arc::new(network::Running::new(diagnostics.clone()));
            let network = Arc::new(network::NetworkHandle::new(
                extension_id.clone(),
                domains.clone(),
                fetcher,
                capability_check.clone(),
                running.clone(),
            ));
            let process_policy = Arc::new(if has_io {
                process_policy
            } else {
                process::ProcessPolicy::default()
            });
            let process_set = Arc::new(process::ProcessSet::default());
            let terminal_environment = process_environment.clone();
            let processes = Arc::new(process::ProcessHandle::new(
                extension_id.clone(),
                process_set.clone(),
                capability_check.clone(),
                workspace.clone(),
                process_environment,
                Arc::new(process_policy.env.clone()),
                running.clone(),
                journal.clone(),
                domains.clone(),
            ));
            let is_v0_3 = has_io;
            let edit_set = Arc::new(edit::EditSet::default());
            let edits = Arc::new(edit::EditHandle::new(
                extension_id.clone(),
                capability_check.clone(),
                workspace_edits.filter(|_| is_v0_3),
                edit_set.clone(),
                running.clone(),
            ));
            let observer = Arc::new(observe::ObserveHandle::new(
                extension_id.clone(),
                capability_check.clone(),
                agent_reviews.filter(|_| is_v0_3),
                running.clone(),
            ));
            // Only a 0.4 component has the `terminals` import.
            let is_v0_4 = ui_api_version >= ApiVersion::V0_4_0;
            let declares = |wanted: &str| capabilities.iter().any(|c| c.as_str() == wanted);
            // P2.4: what the host gets for profiles and env, filtered by the grants when sent.
            let integration = terminals::IntegrationSource {
                profiles: terminal_profiles
                    .filter(|_| declares(terminal::CAPABILITY_PROFILES))
                    .unwrap_or_default(),
                env: terminal_env
                    .clone()
                    .filter(|_| declares(terminal::CAPABILITY_ENV)),
                package_dir: package_dir
                    .clone()
                    .filter(|_| declares(terminal::CAPABILITY_PACKAGE_FILES)),
            };
            let terminals = Arc::new(terminals::TerminalsHandle::new(
                extension_id.clone(),
                terminal_host.filter(|_| is_v0_4),
                integration,
                capability_check.clone(),
            ));
            let declares_create = declares(terminal::CAPABILITY_CREATE);
            let terminal_policy = Arc::new(if is_v0_4 {
                terminals::TerminalPolicy::new(
                    &terminal_programs
                        .filter(|_| declares_create)
                        .unwrap_or_default(),
                    terminal_env,
                    package_dir,
                    relay_binary,
                )
            } else {
                terminals::TerminalPolicy::default()
            });
            // 0.4 `network:listen` (§B.7): the declared listeners, and their discovery files
            // when `fs:discovery-file` is declared too.
            let listens = is_v0_4 && declares(ui_ext::integration::CAPABILITY_NETWORK_LISTEN);
            let listeners = Arc::new(listeners::ListenersHandle::new(
                extension_id.clone(),
                listener_specs.filter(|_| listens).unwrap_or_default(),
                discovery_files
                    .filter(|_| listens && declares(ui_ext::integration::CAPABILITY_DISCOVERY_FILE))
                    .unwrap_or_default(),
                capability_check.clone(),
                listener_observer,
            ));
            let context = Arc::new(editor_context::ContextHandle::new(
                extension_id.clone(),
                context_host.filter(|_| is_v0_4),
                workspace.clone(),
                listeners.clone(),
                diagnostics_source.filter(|_| is_v0_4),
            ));
            let review = Arc::new(proposals::ReviewHandle::new(
                extension_id.clone(),
                review_host.filter(|_| is_v0_4),
                workspace.clone(),
            ));
            // 0.4 `relay:hooks` (§B.9): the declared channels.
            let relay = Arc::new(relay::RelayHandle::new(
                extension_id.clone(),
                relay_channels
                    .filter(|_| is_v0_4 && declares(ui_ext::integration::CAPABILITY_RELAY_HOOKS))
                    .unwrap_or_default(),
                capability_check.clone(),
            ));
            // 0.4 settings (§D): the declarations and the user's values at load.
            let settings = Arc::new(extension_settings::SettingsHandle::new(
                setting_specs.filter(|_| is_v0_4).unwrap_or_default(),
                user_settings.as_ref(),
            ));
            // 0.4 marks (§E, §F.2): what the instance publishes goes with it.
            let marks = Arc::new(extension_marks::MarksHandle::new(
                extension_id.clone(),
                workspace.clone(),
                diagnostics_publisher.filter(|_| is_v0_4),
                file_decorations.filter(|_| is_v0_4),
            ));
            let edits_available = edits.available();
            let reviews_available = observer.available();

            let instantiate = {
                let extension_id = extension_id.clone();
                let diagnostics = diagnostics.clone();
                let capabilities = capabilities.clone();
                let network = network.clone();
                let process_set = process_set.clone();
                let terminals = terminals.clone();
                let listeners = listeners.clone();
                let context = context.clone();
                let review = review.clone();
                let relay = relay.clone();
                let settings = settings.clone();
                let marks = marks.clone();
                let workspace_for_terminals = workspace.clone();
                let revocable = network::Revocable {
                    processes: Arc::downgrade(&process_set),
                    edits: Arc::downgrade(&edit_set),
                    observer: Arc::downgrade(&observer),
                    terminals: Arc::downgrade(&terminals),
                    listeners: Arc::downgrade(&listeners),
                    context: Arc::downgrade(&context),
                    review: Arc::downgrade(&review),
                    relay: Arc::downgrade(&relay),
                    marks: Arc::downgrade(&marks),
                };
                async move {
                    // On tokio: revocations reach in-flight requests, processes and `handle-io`.
                    if has_io {
                        network.watch_permissions(revocable);
                    }
                    if is_v0_4 {
                        terminals.forward(&running);
                        listeners.forward(&running);
                        context.forward(&running);
                        review.forward(&running);
                        relay.forward(&running);
                        settings.forward(&running);
                    }
                    let runtime = ui_runtime()?;
                    let mut wasi = WasiCtxBuilder::new();
                    // Line by line through `log`, capped (triage-3, SEC T5).
                    wasi.stderr(stderr::GuestStderr::new(
                        extension_id.clone(),
                        diagnostics.clone(),
                    ));
                    deny_network(&mut wasi);
                    // No preopened directories, no environment variables, no arguments: the
                    // builder starts empty and only stderr goes anywhere.
                    let mut store = Store::new(
                        &runtime.engine,
                        UiState::new(
                            extension_id.clone(),
                            wasi.build(),
                            capability_check,
                            diagnostics.clone(),
                            effects::ApiState::new(ui_api_version, capabilities),
                            IoStates {
                                network: network::NetworkState::new(domains, running),
                                process: process::ProcessState::new(
                                    process_policy,
                                    process_set,
                                    workspace,
                                ),
                                edit: edit::EditState::new(edits_available),
                                observe: observe::ObserveState::new(reviews_available),
                                terminals: terminals::TerminalState::new(
                                    extension_id.clone(),
                                    terminals.clone(),
                                    terminal_policy,
                                    workspace_for_terminals,
                                    terminal_environment,
                                    listeners.clone(),
                                ),
                                listeners,
                                context,
                                review,
                                relay,
                                settings,
                                marks,
                                journal,
                            },
                        ),
                    );
                    store.limiter(|state| &mut state.limits);
                    // The callback runs only while the guest executes: once per epoch it runs
                    // through, and once on resuming after it yielded, however long it waited
                    // (async-1 #5).
                    store.epoch_deadline_callback(|mut context| {
                        let state = context.data_mut();
                        state.executed += UI_EPOCH_INTERVAL;
                        if state.executed > state.call_budget {
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
                    name: None,
                    network,
                    processes,
                    edits,
                    observer,
                    terminals,
                    listeners,
                    context,
                    review,
                    relay,
                    settings,
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
        let options = LoadOptions {
            package_dir: Some(extension_dir.to_path_buf()),
            ..options
        };
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
            network_domains: manifest.network.as_ref().map(|table| table.domains.clone()),
            extension_version: manifest.version.clone(),
            processes: Some(manifest.processes.clone()),
            process_env: manifest.process.as_ref().map(|table| table.env.clone()),
            terminal_programs: Some(manifest.terminal_programs.clone()),
            terminal_env: manifest.checked_terminal_env(),
            terminal_profiles: terminal::check_profiles(manifest.terminal_profiles.clone()).ok(),
            listeners: Some(manifest.listeners.clone()),
            discovery_files: Some(manifest.discovery_files.clone()),
            relay_channels: Some(manifest.relay_channels.clone()),
            settings: manifest.settings_specs().ok(),
            ..options
        };
        let (mut extension, pushed) =
            Self::load_with_options(extension_id.clone(), wasm_bytes, options, cx)
                .await
                .with_context(|| format!("loading fyx:ui extension {extension_id}"))?;
        anyhow::ensure!(
            extension.api_version == manifest.api_version,
            "extension {extension_id}: extension.toml says fyx:ui {}, the component targets {}",
            manifest.api_version,
            extension.api_version,
        );
        extension.name = manifest.name.map(Arc::from);
        Ok((extension, pushed))
    }

    pub fn extension_id(&self) -> &Arc<str> {
        &self.extension_id
    }

    /// P2.4: the registry started this instance, after consent, at a load or reload. It becomes
    /// the one that speaks to the terminal host for the extension, and sends the host its
    /// profiles and env as granted (again whenever a grant changes; emptied when the instance
    /// goes). An instance it replaced no longer closes terminals when it is dropped.
    ///
    /// 0.4 `network:listen` (§B.7): the replaced instance closes its listeners and this one
    /// opens the granted ones, which then follow the grants until the instance goes.
    pub fn activate_terminals(&self) {
        self.terminals.activate();
        self.listeners.activate();
    }

    /// Whether the 0.4 `context` import has an editor host ([`LoadOptions::context`]).
    pub fn context_available(&self) -> bool {
        self.context.available()
    }

    /// Whether the 0.4 `context.diagnostics` has a language-server source
    /// ([`LoadOptions::diagnostics_source`]).
    pub fn diagnostics_available(&self) -> bool {
        self.context.diagnostics_available()
    }

    /// Whether the 0.4 `review` import has a review bridge.
    pub fn review_available(&self) -> bool {
        self.review.available()
    }

    /// 0.4 `relay:hooks` (§B.9): a hook ran `fyx-relay --channel <extension-id>/<channel>`.
    /// Delivers `io-event::relay-request` with a new id and returns it; `reply` gets the
    /// extension's `relay.respond`, at most once. Refused, with the reason for the log, when the
    /// channel is not declared, `relay:hooks` is not granted, stdin and argv exceed 1 MiB or 8
    /// requests are unanswered. A refused, expired (the channel's `timeout_ms`) or revoked
    /// request, or one whose instance goes, drops `reply` without calling it: answer the
    /// channel's `on_failure` then. `terminal` is this extension's handle for the hook's
    /// terminal, if it has one.
    pub fn relay_request(
        &self,
        channel: &str,
        terminal: Option<terminal::TerminalHandle>,
        argv: Vec<String>,
        stdin: String,
        reply: ui_ext::integration::RelayReply,
    ) -> Result<u64, String> {
        self.relay.request(channel, terminal, argv, stdin, reply)
    }

    /// The declared `[[fyx_ui.relay_channels]]` (empty without `relay:hooks` or before 0.4),
    /// granted or not: their `timeout_ms` and `on_failure` apply to refused requests too.
    pub fn relay_channels(&self) -> &[ui_ext::integration::RelayChannelSpec] {
        self.relay.channels()
    }

    /// The checked `[[fyx_ui.settings]]` of a 0.4 extension, in manifest order, for the Settings
    /// tab (design §D.3); empty without the table or below 0.4.
    pub fn setting_specs(&self) -> &[ui_ext::settings::SettingSpec] {
        self.settings.specs()
    }

    /// The effective values the extension gets now, with the user's invalid values' errors and
    /// the unknown keys, for the Settings tab (§D.2, §D.3).
    pub fn effective_settings(&self) -> ui_ext::settings::EffectiveSettings {
        self.settings.effective()
    }

    /// The user settings file changed: `user` is the extension's object under
    /// `fyx_extensions.<id>` in the **user** settings (never project settings, §D.2), `None`
    /// when there is none. Returns the keys whose effective value changed, in manifest order;
    /// a running 0.4 extension gets them as `settings-changed` (§D.4).
    pub fn set_user_settings(&self, user: Option<&ui_ext::settings::RawValue>) -> Vec<String> {
        self.settings.update(user)
    }

    /// Whether `relay:hooks` is declared, with channels, and granted now.
    pub fn relay_granted(&self) -> bool {
        self.relay.granted()
    }

    /// `${listen.<name>.port}` while the listener is open (tests and the Extension Inspector).
    pub fn listener_port(&self, name: &str) -> Option<u16> {
        self.listeners.port(name)
    }

    /// Whether the 0.4 `terminals` import has a terminal host (a 0.4 component loaded with
    /// [`LoadOptions::terminals`]).
    pub fn terminals_available(&self) -> bool {
        self.terminals.available()
    }

    /// `extension.toml`'s `name` as its author wrote it (check it before showing it).
    pub fn display_name(&self) -> Option<&str> {
        self.name.as_deref()
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
        let network = self.network.clone();
        let processes = self.processes.clone();
        let edits = self.edits.clone();
        let observer = self.observer.clone();
        let terminals = self.terminals.clone();
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
                        // `permissions.request` prompts, after the call and only if it returned;
                        // the answer reaches the guest as `permission-changed`.
                        let prompts = std::mem::take(&mut store.data_mut().permission_requests);
                        if result.is_ok()
                            && let Some(broker) = store.data().capability_check.clone()
                        {
                            for capability in prompts {
                                let extension_id = store.data().extension_id.clone();
                                let broker = broker.clone();
                                tokio::spawn(async move {
                                    broker.request(&extension_id, capability).await;
                                });
                            }
                        }
                        // The call's requests start now, after it returned, or are dropped with
                        // its effects if it failed (design.md §0.3).
                        network.start(store.data_mut().network.finish_call(result.is_ok()));
                        processes.start(store.data_mut().process.finish_call(result.is_ok()));
                        edits.start(store.data_mut().edit.finish_call(result.is_ok()));
                        observer.apply(store.data_mut().observe.finish_call(result.is_ok()));
                        terminals.start_t3(
                            store.data_mut().terminals.finish_call(result.is_ok()),
                            store.data().journal.clone(),
                        );
                        // A trap kills the extension's processes with it (design.md §9) and ends
                        // its agent feed.
                        if result.is_err() {
                            processes.kill_all();
                            observer.stop();
                        }
                        diagnostics.call(diagnostics::CallRecord {
                            kind,
                            surface_id,
                            at: std::time::SystemTime::now(),
                            duration: store.data().call_started.elapsed(),
                            executed: store.data().executed,
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

#[cfg(test)]
mod tests_v0_4;
