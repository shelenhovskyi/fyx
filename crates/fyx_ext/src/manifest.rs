//! `extension.toml`: the fields Fyx reads, checked with the host's rules
//! (`extension_host::wasm_host::fyx_ui::UiManifest`) plus the registry's naming rules.

use std::path::{Component as PathComponent, Path, PathBuf};

use anyhow::{Context as _, Result};
use semver::Version;
use serde::Deserialize;

use ui_ext::{integration, terminal};

use crate::Report;

pub const MANIFEST_FILE: &str = "extension.toml";

/// The `fyx:ui` versions this `fyx-ext` knows how to check, matching the host's
/// `MIN_VERSION..=MAX_VERSION`.
pub const SUPPORTED_API_VERSIONS: &[Version] = &[
    Version::new(0, 1, 0),
    Version::new(0, 2, 0),
    Version::new(0, 3, 0),
    Version::new(0, 4, 0),
];

/// The newest frozen version, the host's `MAX_FROZEN_VERSION`. Newer ones are staged: dev builds
/// of Fyx load them (debug builds, or a process started with `FYX_UI_DEV_EXTENSIONS`), releases
/// refuse them until they are frozen.
pub const MAX_FROZEN_API_VERSION: Version = Version::new(0, 3, 0);

/// `[fyx_ui.network] domains` entries per manifest, the host's `network::MAX_DOMAINS`.
pub const MAX_NETWORK_DOMAINS: usize = 64;

const NETWORK_FETCH: &str = "network:fetch";
const PROCESS_SPAWN: &str = "process:spawn";

/// What each world accepts, the host's `world_capabilities` (`UiManifest::check_world`,
/// migration.md §2). 0.3 is the smaller 0.3 (A3): what is planned stays out.
fn world_capabilities(version: &Version) -> Option<&'static [&'static str]> {
    const V0_1: &[&str] = &[
        "ui:panel",
        "ui:status-bar",
        // Accepted, unused: a manifest declares them before its component moves to 0.2.
        "ui:modal",
        "ui:editor-decorations",
    ];
    const V0_2: &[&str] = &[
        "ui:panel",
        "ui:status-bar",
        "ui:modal",
        "ui:editor-decorations",
        "ui:notifications",
    ];
    const V0_3: &[&str] = &[
        "ui:panel",
        "ui:status-bar",
        "ui:modal",
        "ui:editor-decorations",
        "ui:notifications",
        "ui:images",
        "ui:canvas",
        NETWORK_FETCH,
        PROCESS_SPAWN,
        "workspace:edit",
        "agent:observe",
    ];
    // Staged: 0.3 plus the terminal and integration capabilities (plan P5 5.5), the T3
    // `terminal:read` and `terminal:write` included (design wit-0.4 §C.3).
    const V0_4: &[&str] = &[
        "ui:panel",
        "ui:status-bar",
        "ui:modal",
        "ui:editor-decorations",
        "ui:notifications",
        "ui:images",
        "ui:canvas",
        NETWORK_FETCH,
        PROCESS_SPAWN,
        "workspace:edit",
        "agent:observe",
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
        integration::CAPABILITY_APPROVALS,
        integration::CAPABILITY_WORKSPACE_DIAGNOSTICS,
    ];
    if *version >= Version::new(0, 4, 0) {
        Some(V0_4)
    } else if *version >= Version::new(0, 3, 0) {
        Some(V0_3)
    } else if *version >= Version::new(0, 2, 0) {
        Some(V0_2)
    } else {
        Some(V0_1)
    }
}

/// Capabilities a manifest may declare (platform spec, decision 21). Ones the current world does
/// not provide yet are accepted with a warning, so a manifest can be written ahead of the host.
pub const KNOWN_CAPABILITIES: &[(&str, CapabilityStatus)] = &[
    ("ui:panel", CapabilityStatus::Since(Version::new(0, 1, 0))),
    (
        "ui:status-bar",
        CapabilityStatus::Since(Version::new(0, 1, 0)),
    ),
    ("ui:modal", CapabilityStatus::Since(Version::new(0, 2, 0))),
    (
        "ui:editor-decorations",
        CapabilityStatus::Since(Version::new(0, 2, 0)),
    ),
    (
        "ui:notifications",
        CapabilityStatus::Since(Version::new(0, 2, 0)),
    ),
    ("ui:images", CapabilityStatus::Since(Version::new(0, 3, 0))),
    (
        NETWORK_FETCH,
        CapabilityStatus::Since(Version::new(0, 3, 0)),
    ),
    ("ui:canvas", CapabilityStatus::Since(Version::new(0, 3, 0))),
    ("ui:picker", CapabilityStatus::Planned(NOT_BUILT)),
    ("ui:code-lens", CapabilityStatus::Planned(NOT_BUILT)),
    ("ui:hover", CapabilityStatus::Planned(NOT_BUILT)),
    ("ui:virtual-documents", CapabilityStatus::Planned(NOT_BUILT)),
    ("ui:commands", CapabilityStatus::Planned(NOT_BUILT)),
    (
        "workspace:read-buffers",
        CapabilityStatus::Planned("a later release"),
    ),
    (
        "workspace:read-files",
        CapabilityStatus::Planned("a later release"),
    ),
    (
        "workspace:edit",
        CapabilityStatus::Since(Version::new(0, 3, 0)),
    ),
    (
        PROCESS_SPAWN,
        CapabilityStatus::Since(Version::new(0, 3, 0)),
    ),
    (
        "agent:observe",
        CapabilityStatus::Since(Version::new(0, 3, 0)),
    ),
    (terminal::CAPABILITY_PROFILES, SINCE_V0_4),
    (terminal::CAPABILITY_PSEUDO, SINCE_V0_4),
    (terminal::CAPABILITY_CREATE, SINCE_V0_4),
    (terminal::CAPABILITY_ENV, SINCE_V0_4),
    (terminal::CAPABILITY_STATUS, SINCE_V0_4),
    (terminal::CAPABILITY_OBSERVE, SINCE_V0_4),
    (terminal::CAPABILITY_READ, SINCE_V0_4),
    (terminal::CAPABILITY_WRITE, SINCE_V0_4),
    (terminal::CAPABILITY_PACKAGE_FILES, SINCE_V0_4),
    (integration::CAPABILITY_NETWORK_LISTEN, SINCE_V0_4),
    (integration::CAPABILITY_DISCOVERY_FILE, SINCE_V0_4),
    (integration::CAPABILITY_RELAY_HOOKS, SINCE_V0_4),
    (integration::CAPABILITY_REVIEW_PROPOSE, SINCE_V0_4),
    (integration::CAPABILITY_CONTEXT_EDITOR, SINCE_V0_4),
    (integration::CAPABILITY_WORKSPACE_OPEN, SINCE_V0_4),
    (integration::CAPABILITY_APPROVALS, SINCE_V0_4),
    (integration::CAPABILITY_WORKSPACE_DIAGNOSTICS, SINCE_V0_4),
];

const SINCE_V0_4: CapabilityStatus = CapabilityStatus::Since(Version::new(0, 4, 0));

/// Capabilities that a surface kind needs, so they are used by `surfaces()` rather than by an
/// import.
pub const SURFACE_CAPABILITIES: &[&str] = &["ui:panel", "ui:status-bar", "ui:modal"];

/// fyx:ui 0.3 parts the host does not run yet: their imports trap and their nodes are refused.
const NOT_BUILT: &str = "fyx:ui 0.4";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityStatus {
    /// Provided by the host from this `fyx:ui` version on.
    Since(Version),
    Planned(&'static str),
}

/// Whether the host provides `capability` to an extension that targets `api_version`.
pub fn capability_available(capability: &str, api_version: Option<&Version>) -> bool {
    KNOWN_CAPABILITIES.iter().any(|(name, status)| {
        name == &capability
            && matches!(status, CapabilityStatus::Since(since)
                if api_version.is_none_or(|version| version >= since))
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionToml {
    pub id: String,
    pub name: Option<String>,
    pub version: String,
    pub schema_version: Option<i64>,
    pub fyx_ui: Option<FyxUiTable>,
}

/// `[fyx_ui]`. Unknown keys are rejected, as by the host.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FyxUiTable {
    pub api_version: String,
    pub component: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 0.3, with `network:fetch`: the hosts the extension may fetch from.
    #[serde(default)]
    pub network: Option<NetworkTable>,
    /// 0.3, with `process:spawn`: the programs the extension may run.
    #[serde(default)]
    pub processes: Vec<ui_ext::process::ProcessSpec>,
    /// 0.3, with `process:spawn`: the environment names passed through.
    #[serde(default)]
    pub process: Option<ProcessTable>,
    /// 0.4, with `terminal:profiles`.
    #[serde(default)]
    pub terminal_profiles: Vec<terminal::TerminalProfileSpec>,
    /// 0.4, with `terminal:create`.
    #[serde(default)]
    pub terminal_programs: Vec<terminal::TerminalProgramSpec>,
    /// 0.4, with `terminal:env`.
    #[serde(default)]
    pub terminal_env: Option<terminal::TerminalEnvSpec>,
    /// 0.4, with `network:listen`.
    #[serde(default)]
    pub listeners: Vec<integration::ListenerSpec>,
    /// 0.4, with `fs:discovery-file`.
    #[serde(default)]
    pub discovery_files: Vec<integration::DiscoveryFileSpec>,
    /// 0.4, with `relay:hooks`.
    #[serde(default)]
    pub relay_channels: Vec<integration::RelayChannelSpec>,
}

/// `[fyx_ui.process]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessTable {
    #[serde(default)]
    pub env: Vec<String>,
}

/// `[fyx_ui.network]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkTable {
    pub domains: Vec<String>,
    /// 0.4 (design wit-0.4 §C.2): the author's line under the item at install, required there
    /// and refused below 0.4.
    #[serde(default)]
    pub reason: Option<String>,
}

/// A manifest whose required parts parsed; `report` holds what is wrong with the rest.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub dir: PathBuf,
    pub id: String,
    pub version: String,
    pub api_version: Option<Version>,
    pub component: String,
    pub capabilities: Vec<String>,
    /// The top-level files and folders that `${package}/...` names in profile args and
    /// `[fyx_ui.terminal_env]` values (design B.6), which `fyx-ext package` packs.
    pub package_files: Vec<String>,
}

impl Manifest {
    /// The component inside the extension folder, with `\` read as `/` on every OS.
    pub fn component_path(&self) -> PathBuf {
        match normalized_relative(&self.component) {
            Some(normalized) => self.dir.join(normalized),
            None => self.dir.join(&self.component),
        }
    }

    /// The component's name inside a package: `/`-separated.
    pub fn component_archive_name(&self) -> Option<String> {
        normalized_relative(&self.component)
    }

    pub fn package_file_name(&self) -> String {
        format!("{}-{}.fyxext", self.id, self.version)
    }
}

/// Reads and checks `<dir>/extension.toml`. An error means the manifest cannot be used at all;
/// problems that still leave a usable manifest go into `report`.
pub fn read(dir: &Path, report: &mut Report) -> Result<Manifest> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {path:?}"))?;
    parse(dir, &text, report)
}

pub fn parse(dir: &Path, text: &str, report: &mut Report) -> Result<Manifest> {
    let toml: ExtensionToml =
        toml::from_str(text).with_context(|| format!("parsing {MANIFEST_FILE}"))?;
    let table = toml.fyx_ui.with_context(|| {
        format!("{MANIFEST_FILE} has no [fyx_ui] table, so Fyx would not load it as a UI extension")
    })?;

    if !is_valid_id(&toml.id) {
        report.error(format!(
            "id {:?}: use lower-case letters and digits separated by single '-', starting with a letter",
            toml.id
        ));
    }
    if Version::parse(&toml.version).is_err() {
        report.error(format!(
            "version {:?} is not a semantic version (like 0.1.0)",
            toml.version
        ));
    }
    if toml
        .name
        .as_deref()
        .is_none_or(|name| name.trim().is_empty())
    {
        report.warning("name is missing; the extensions page shows the id instead");
    }
    if toml.schema_version != Some(1) {
        report.warning("schema_version should be 1");
    }

    let api_version = match Version::parse(&table.api_version) {
        Ok(version) if SUPPORTED_API_VERSIONS.contains(&version) => {
            if version > MAX_FROZEN_API_VERSION {
                report.warning(format!(
                    "[fyx_ui].api_version {version} is staged: dev builds of Fyx load it (debug \
                     builds, or with FYX_UI_DEV_EXTENSIONS set), releases refuse it until it is \
                     frozen; releases run {MAX_FROZEN_API_VERSION}"
                ));
            }
            Some(version)
        }
        Ok(version) => {
            report.error(format!(
                "[fyx_ui].api_version {version} is not supported; this fyx-ext and the current host support {}",
                supported_versions()
            ));
            Some(version)
        }
        Err(_) => {
            report.error(format!(
                "[fyx_ui].api_version {:?} is not a version (like 0.1.0)",
                table.api_version
            ));
            None
        }
    };

    if !is_relative_inside(&table.component) {
        report.error(format!(
            "[fyx_ui].component must be a relative path inside the extension, without `.` or `..`, got {:?}",
            table.component
        ));
    }

    for (index, capability) in table.capabilities.iter().enumerate() {
        if table.capabilities[..index].contains(capability) {
            report.warning(format!("capability {capability:?} is declared twice"));
            continue;
        }
        match KNOWN_CAPABILITIES
            .iter()
            .find(|(name, _)| name == capability)
        {
            None => report.error(format!(
                "unknown capability {capability:?}; known capabilities: {}",
                KNOWN_CAPABILITIES
                    .iter()
                    .map(|(name, _)| format!("{name:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Some((_, status)) => {
                let world = api_version
                    .as_ref()
                    .map(|version| (version, world_capabilities(version)));
                match (status, world) {
                    // The host refuses what the targeted world does not take.
                    (status, Some((version, Some(world))))
                        if !world.contains(&capability.as_str()) =>
                    {
                        report.error(match status {
                            CapabilityStatus::Since(since) => format!(
                                "capability {capability:?} needs fyx:ui {since}; this extension \
                                 targets {version}"
                            ),
                            CapabilityStatus::Planned(release) => format!(
                                "capability {capability:?} is not in fyx:ui {version}; it is \
                                 planned for {release}"
                            ),
                        });
                    }
                    (CapabilityStatus::Planned(release), _) => report.warning(format!(
                        "capability {capability:?} arrives with {release}; the current host \
                         ignores it"
                    )),
                    (CapabilityStatus::Since(since), Some((version, _))) if version < since => {
                        report.warning(format!(
                            "capability {capability:?} is accepted but unused until fyx:ui \
                             {since}; this extension targets {version}"
                        ))
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(version) = &api_version {
        check_world_tables(&table, version, report);
    }
    check_network(&table, report);
    check_processes(&table, report);
    check_terminals(&table, report);
    let package_files = check_package_files(dir, &table, report);

    Ok(Manifest {
        dir: dir.to_path_buf(),
        id: toml.id,
        version: toml.version,
        api_version,
        component: table.component,
        capabilities: table.capabilities,
        package_files,
    })
}

/// The values that may hold `${package}`: profile args and `[fyx_ui.terminal_env]` values.
fn package_templates(table: &FyxUiTable) -> impl Iterator<Item = &str> {
    let args = table
        .terminal_profiles
        .iter()
        .flat_map(|profile| profile.args.iter());
    let env = table.terminal_env.iter().flat_map(|env| env.vars.values());
    args.chain(env).map(String::as_str)
}

/// `${package}` and `package:files` (design B.6): the placeholder needs the capability, the
/// capability is useless without it, and every `${package}/<path>` names a file or folder in the
/// extension, which is then packed. Returns the top-level names to pack.
fn check_package_files(dir: &Path, table: &FyxUiTable, report: &mut Report) -> Vec<String> {
    const PLACEHOLDER: &str = "${package}";
    let declares = table
        .capabilities
        .iter()
        .any(|name| name == terminal::CAPABILITY_PACKAGE_FILES);
    let mut uses = false;
    let mut top_level = std::collections::BTreeSet::new();
    for value in package_templates(table) {
        for (at, _) in value.match_indices(PLACEHOLDER) {
            uses = true;
            let rest = &value[at + PLACEHOLDER.len()..];
            let Some(path) = rest.strip_prefix('/') else {
                continue;
            };
            let path = path
                .split(|c: char| c.is_whitespace() || c == '$')
                .next()
                .unwrap_or_default()
                .trim_end_matches('/');
            if path.is_empty() {
                continue;
            }
            let Some(normalized) = normalized_relative(path) else {
                report.error(format!(
                    "{value:?}: {PLACEHOLDER}/{path} is not a plain path inside the extension"
                ));
                continue;
            };
            if std::fs::symlink_metadata(dir.join(&normalized)).is_err() {
                report.error(format!(
                    "{value:?}: {PLACEHOLDER}/{normalized} is not in the extension folder, so the \
                     installed extension would point at nothing"
                ));
                continue;
            }
            if let Some(first) = normalized.split('/').next() {
                top_level.insert(first.to_owned());
            }
        }
    }
    match (uses, declares) {
        (true, false) => report.error(format!(
            "{PLACEHOLDER} needs {:?} in [fyx_ui].capabilities",
            terminal::CAPABILITY_PACKAGE_FILES
        )),
        (false, true) => report.warning(format!(
            "capability {:?} grants nothing without {PLACEHOLDER} in a terminal profile's args or \
             [fyx_ui].terminal_env",
            terminal::CAPABILITY_PACKAGE_FILES
        )),
        _ => {}
    }
    top_level.into_iter().collect()
}

/// Each world's tables only from that world on, as the host's `UiManifest::check_world`.
fn check_world_tables(table: &FyxUiTable, version: &Version, report: &mut Report) {
    let v0_3 = Version::new(0, 3, 0);
    let v0_4 = Version::new(0, 4, 0);
    let mut tables = vec![
        (table.network.is_some(), "[fyx_ui.network]", &v0_3),
        (!table.processes.is_empty(), "[[fyx_ui.processes]]", &v0_3),
        (table.process.is_some(), "[fyx_ui.process]", &v0_3),
    ];
    tables.extend(
        terminal_tables(table)
            .into_iter()
            .map(|(present, name, _)| (present, name, &v0_4)),
    );
    for (present, name, since) in tables {
        if present && version < since {
            report.error(format!(
                "{name} needs fyx:ui {since}; this extension targets {version}"
            ));
        }
    }
    check_t2_reasons(table, version, report);
}

/// `reason` on `[[fyx_ui.processes]]` and `[fyx_ui.network]`, as the host's
/// `UiManifest::check_t2_reasons`: from 0.4 every T2 item has one line of at most 200 bytes
/// (design wit-0.4 §C.2); below 0.4 the key does not exist.
fn check_t2_reasons(table: &FyxUiTable, version: &Version, report: &mut Report) {
    let items = table
        .processes
        .iter()
        .map(|spec| {
            (
                format!("[[fyx_ui.processes]] {:?}", spec.name),
                spec.reason.as_deref(),
            )
        })
        .chain(
            table
                .network
                .iter()
                .map(|network| ("[fyx_ui.network]".to_owned(), network.reason.as_deref())),
        );
    for (what, reason) in items {
        match reason {
            Some(_) if *version < Version::new(0, 4, 0) => report.error(format!(
                "{what}: reason needs fyx:ui 0.4; this extension targets {version}"
            )),
            Some(reason) => {
                if let Err(error) = terminal::check_reason(&what, reason) {
                    report.error(error);
                }
            }
            None if *version >= Version::new(0, 4, 0) => report.error(format!(
                "{what}: a reason is required (fyx:ui 0.4): one line, shown under the item at                  install"
            )),
            None => {}
        }
    }
}

/// The 0.4 tables: whether each is present, its name and the capability it goes with.
fn terminal_tables(table: &FyxUiTable) -> [(bool, &'static str, &'static str); 6] {
    [
        (
            !table.terminal_profiles.is_empty(),
            "[[fyx_ui.terminal_profiles]]",
            terminal::CAPABILITY_PROFILES,
        ),
        (
            !table.terminal_programs.is_empty(),
            "[[fyx_ui.terminal_programs]]",
            terminal::CAPABILITY_CREATE,
        ),
        (
            table.terminal_env.is_some(),
            "[fyx_ui.terminal_env]",
            terminal::CAPABILITY_ENV,
        ),
        (
            !table.listeners.is_empty(),
            "[[fyx_ui.listeners]]",
            integration::CAPABILITY_NETWORK_LISTEN,
        ),
        (
            !table.discovery_files.is_empty(),
            "[[fyx_ui.discovery_files]]",
            integration::CAPABILITY_DISCOVERY_FILE,
        ),
        (
            !table.relay_channels.is_empty(),
            "[[fyx_ui.relay_channels]]",
            integration::CAPABILITY_RELAY_HOOKS,
        ),
    ]
}

/// The 0.4 terminal and integration tables against their capabilities, with the host's entry
/// checks (`UiManifest::check_terminals`).
fn check_terminals(table: &FyxUiTable, report: &mut Report) {
    let declares = |capability: &str| table.capabilities.iter().any(|name| name == capability);
    for (present, name, capability) in terminal_tables(table) {
        match (present, declares(capability)) {
            (true, false) => report.error(format!(
                "{name} needs {capability:?} in [fyx_ui].capabilities"
            )),
            (false, true) => report.error(format!("capability {capability:?} needs {name}")),
            _ => {}
        }
    }
    let integration = integration::IntegrationTables {
        listeners: &table.listeners,
        discovery_files: &table.discovery_files,
        relay_channels: &table.relay_channels,
    };
    let checks = [
        terminal::check_profiles(table.terminal_profiles.clone()).map(drop),
        terminal::check_programs(&table.terminal_programs),
        integration.check(),
        table.terminal_env.clone().map_or(Ok(()), |env| {
            terminal::check_terminal_env(env, &|name| integration.listener(name)).map(drop)
        }),
    ];
    for error in checks.into_iter().filter_map(Result::err) {
        report.error(error);
    }
}

/// `[fyx_ui.network]` against `network:fetch`, with the host's count cap and the shape of each
/// pattern. The host checks the rest when it loads the extension: public-suffix wildcards and
/// IP literals.
fn check_network(table: &FyxUiTable, report: &mut Report) {
    let declares = table.capabilities.iter().any(|name| name == NETWORK_FETCH);
    let Some(network) = &table.network else {
        if declares {
            report.error(format!(
                "capability {NETWORK_FETCH:?} needs [fyx_ui.network] domains, the hosts it may \
                 fetch from"
            ));
        }
        return;
    };
    if !declares {
        report.error(format!(
            "[fyx_ui.network] needs {NETWORK_FETCH:?} in [fyx_ui].capabilities"
        ));
    }
    if network.domains.is_empty() {
        report.error("[fyx_ui.network] domains is empty, so every fetch would be denied");
    }
    if network.domains.len() > MAX_NETWORK_DOMAINS {
        report.error(format!(
            "[fyx_ui.network] has {} domains; at most {MAX_NETWORK_DOMAINS}",
            network.domains.len()
        ));
    }
    for domain in &network.domains {
        let host = domain
            .rsplit_once(':')
            .map_or(domain.as_str(), |(host, _)| host);
        let loopback = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1";
        if domain.contains("://") || domain.contains('/') || host.is_empty() {
            report.error(format!(
                "network domain {domain:?}: write a host, such as api.example.com, \
                 *.example.com or localhost:8080, without a scheme or path"
            ));
        } else if loopback && !domain.contains(':') {
            report.error(format!(
                "network domain {domain:?}: localhost needs a port, such as localhost:8080"
            ));
        }
    }
}

/// `[[fyx_ui.processes]]` and `[fyx_ui.process] env` against `process:spawn`, with the host's
/// rules for environment names. The host resolves the programs when it loads the extension.
fn check_processes(table: &FyxUiTable, report: &mut Report) {
    let declares = table.capabilities.iter().any(|name| name == PROCESS_SPAWN);
    if declares && table.processes.is_empty() {
        report.error(format!(
            "capability {PROCESS_SPAWN:?} needs [[fyx_ui.processes]], the programs it may run"
        ));
    }
    if !declares && (!table.processes.is_empty() || table.process.is_some()) {
        report.error(format!(
            "[[fyx_ui.processes]] and [fyx_ui.process] need {PROCESS_SPAWN:?} in \
             [fyx_ui].capabilities"
        ));
    }
    for (index, spec) in table.processes.iter().enumerate() {
        if spec.name.trim().is_empty() || spec.name.contains(['/', '\\']) {
            report.error(format!(
                "[[fyx_ui.processes]] {index}: name {:?} must be a program name, not a path \
                 (use `path` for a program in the worktree)",
                spec.name
            ));
        }
        if spec.args.is_empty() {
            report.warning(format!(
                "[[fyx_ui.processes]] {:?} has no `args`, so it may only run with no arguments; \
                 `args = [\"**\"]` allows any",
                spec.name
            ));
        }
        if table.processes[..index]
            .iter()
            .any(|other| other.name == spec.name)
        {
            report.error(format!(
                "[[fyx_ui.processes]] declares {:?} twice",
                spec.name
            ));
        }
        if spec
            .args
            .iter()
            .rev()
            .skip(1)
            .any(|pattern| pattern == "**")
        {
            report.error(format!(
                "[[fyx_ui.processes]] {:?}: `**` may only be the last argument pattern",
                spec.name
            ));
        }
    }
    for name in table.process.iter().flat_map(|process| &process.env) {
        if let Err(error) = ui_ext::process::check_env_name(name) {
            report.error(format!("[fyx_ui.process] env: {error}"));
        }
    }
}

pub fn supported_versions() -> String {
    SUPPORTED_API_VERSIONS
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Registry ids: `my-panel`. The template turns `my_panel` into this form.
pub fn is_valid_id(id: &str) -> bool {
    let mut parts = id.split('-');
    let starts_with_letter = id.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    starts_with_letter
        && id.len() <= 64
        && parts.all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// The rule for paths inside an extension, the same on every OS: `\` counts as `/`, and every
/// part is a plain name, so there is no root, drive, empty part, `.` or `..`. On Linux `..\x.wasm`
/// is a single file name to `Path`, but it means `../x.wasm` on Windows and inside a zip.
pub fn is_relative_inside(path: &str) -> bool {
    normalized_relative(path).is_some()
}

/// `path` with `/` separators, if it follows [`is_relative_inside`].
pub fn normalized_relative(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let plain_parts = !normalized.is_empty()
        && normalized.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.contains([':', '\0'])
        });
    let plain_components = Path::new(&normalized)
        .components()
        .all(|component| matches!(component, PathComponent::Normal(_)));
    (plain_parts && plain_components).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
id = "my-panel"
name = "My panel"
version = "0.1.0"
schema_version = 1

[fyx_ui]
api_version = "0.1.0"
component = "fyx_ui.wasm"
capabilities = ["ui:panel"]
"#;

    fn check(text: &str) -> (Result<Manifest>, Report) {
        let mut report = Report::default();
        let manifest = parse(Path::new("ext"), text, &mut report);
        (manifest, report)
    }

    #[test]
    fn paths_inside_an_extension_follow_one_rule_on_every_os() {
        for (path, normalized) in [
            ("fyx_ui.wasm", Some("fyx_ui.wasm")),
            ("bin/fyx_ui.wasm", Some("bin/fyx_ui.wasm")),
            (r"bin\fyx_ui.wasm", Some("bin/fyx_ui.wasm")),
            (r"..\fyx_ui.wasm", None),
            ("../fyx_ui.wasm", None),
            (r"bin\..\..\x.wasm", None),
            ("./fyx_ui.wasm", None),
            ("bin//fyx_ui.wasm", None),
            ("/fyx_ui.wasm", None),
            (r"\fyx_ui.wasm", None),
            (r"C:\fyx_ui.wasm", None),
            ("C:fyx_ui.wasm", None),
            ("fyx_ui.wasm:stream", None),
            ("", None),
        ] {
            assert_eq!(normalized_relative(path).as_deref(), normalized, "{path:?}");
            assert_eq!(is_relative_inside(path), normalized.is_some(), "{path:?}");
        }
    }

    #[test]
    fn a_valid_manifest_has_no_findings() {
        let (manifest, report) = check(VALID);
        let manifest = manifest.expect("valid");
        assert!(report.is_empty(), "{report:?}");
        assert_eq!(manifest.package_file_name(), "my-panel-0.1.0.fyxext");
        assert_eq!(
            manifest.component_path(),
            Path::new("ext").join("fyx_ui.wasm")
        );
        assert_eq!(manifest.api_version, Some(Version::new(0, 1, 0)));
    }

    #[test]
    fn missing_fyx_ui_table_is_fatal() {
        let (manifest, _) = check("id = \"x\"\nversion = \"0.1.0\"\n");
        let error = manifest.expect_err("no [fyx_ui]").to_string();
        assert!(error.contains("no [fyx_ui] table"), "{error}");
    }

    #[test]
    fn unknown_fyx_ui_keys_are_fatal_like_on_the_host() {
        let (manifest, _) = check(&VALID.replace("capabilities", "capabilites"));
        assert!(manifest.is_err());
    }

    #[test]
    fn host_rules_are_errors() {
        for (from, to, expected) in [
            (
                "\"fyx_ui.wasm\"",
                "\"../fyx_ui.wasm\"",
                "relative path inside",
            ),
            (
                "\"fyx_ui.wasm\"",
                "\"./fyx_ui.wasm\"",
                "relative path inside",
            ),
            (
                "\"fyx_ui.wasm\"",
                "\"/fyx_ui.wasm\"",
                "relative path inside",
            ),
            (
                "api_version = \"0.1.0\"",
                "api_version = \"0.9.0\"",
                "not supported",
            ),
            (
                "api_version = \"0.1.0\"",
                "api_version = \"latest\"",
                "not a version",
            ),
            ("\"ui:panel\"", "\"ui:panels\"", "unknown capability"),
            ("id = \"my-panel\"", "id = \"My_Panel\"", "lower-case"),
            (
                "version = \"0.1.0\"\nschema",
                "version = \"1\"\nschema",
                "semantic version",
            ),
        ] {
            let (manifest, report) = check(&VALID.replacen(from, to, 1));
            assert!(manifest.is_ok());
            assert!(
                report.errors.iter().any(|error| error.contains(expected)),
                "{to}: {report:?}"
            );
        }
    }

    #[test]
    fn planned_capabilities_are_errors_and_duplicates_warnings() {
        let text = VALID.replace(
            "[\"ui:panel\"]",
            "[\"ui:panel\", \"ui:panel\", \"ui:hover\"]",
        );
        // Every world refuses what is planned for 0.4, the smaller 0.3 (A3) too.
        for version in ["0.1.0", "0.3.0"] {
            let (_, report) = check(&text.replace(
                "api_version = \"0.1.0\"",
                &format!("api_version = \"{version}\""),
            ));
            assert_eq!(report.errors.len(), 1, "{report:?}");
            assert!(
                report.errors[0].contains(&format!(
                    "\"ui:hover\" is not in fyx:ui {version}; it is planned for fyx:ui 0.4"
                )),
                "{report:?}"
            );
            assert!(
                report.warnings.iter().any(|w| w.contains("declared twice")),
                "{report:?}"
            );
        }
    }

    /// The table follows the host: every demo in `extensions/` passes, so `fyx-ext check` skips
    /// none of them (`workspace:diagnostics` was still "planned" here while 0.4 provided it).
    #[test]
    fn shipped_demos_pass_the_manifest_check() {
        let extensions = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
        let mut checked = 0;
        for entry in std::fs::read_dir(&extensions).expect("extensions/ exists") {
            let dir = entry.expect("readable entry").path();
            let Ok(text) = std::fs::read_to_string(dir.join("extension.toml")) else {
                continue;
            };
            if !text.contains("[fyx_ui]") {
                continue;
            }
            let mut report = Report::default();
            let manifest = parse(&dir, &text, &mut report);
            assert!(manifest.is_ok(), "{}: {manifest:?}", dir.display());
            assert!(report.errors.is_empty(), "{}: {report:?}", dir.display());
            assert!(
                !report.warnings.iter().any(|w| w.contains("arrives with")),
                "{}: {report:?}",
                dir.display()
            );
            checked += 1;
        }
        assert!(checked > 1, "found {checked} fyx_ui demos");
        assert!(capability_available(
            integration::CAPABILITY_WORKSPACE_DIAGNOSTICS,
            Some(&Version::new(0, 4, 0))
        ));
        assert!(!capability_available(
            integration::CAPABILITY_WORKSPACE_DIAGNOSTICS,
            Some(&Version::new(0, 3, 0))
        ));
    }

    #[test]
    fn v0_3_and_its_capabilities() {
        let staged = VALID
            .replace("api_version = \"0.1.0\"", "api_version = \"0.3.0\"")
            .replace("[\"ui:panel\"]", "[\"ui:panel\", \"ui:images\"]");
        let (manifest, report) = check(&staged);
        assert!(report.errors.is_empty(), "{report:?}");
        // 0.3 is frozen (integration-6): no staged warning.
        assert!(report.warnings.is_empty(), "{report:?}");
        assert_eq!(
            manifest.expect("parses").api_version,
            Some(Version::new(0, 3, 0))
        );

        // Each world takes its own capabilities (the host's `UiManifest::check_world`).
        let early = VALID.replace("[\"ui:panel\"]", "[\"ui:panel\", \"ui:notifications\"]");
        let (_, report) = check(&early);
        assert_eq!(report.errors.len(), 1, "{report:?}");
        assert!(
            report.errors[0].contains("needs fyx:ui 0.2.0; this extension targets 0.1.0"),
            "{report:?}"
        );
        let frozen = early.replace("api_version = \"0.1.0\"", "api_version = \"0.2.0\"");
        let (_, report) = check(&frozen);
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(report.warnings.is_empty(), "0.2 is frozen: {report:?}");
        let images = frozen.replace("\"ui:notifications\"", "\"ui:images\"");
        let (_, report) = check(&images);
        assert!(
            report.errors[0].contains("needs fyx:ui 0.3.0; this extension targets 0.2.0"),
            "{report:?}"
        );
        // 0.1 takes the 0.2 surfaces unused, so a manifest can move first.
        let ahead = VALID.replace(
            "[\"ui:panel\"]",
            "[\"ui:panel\", \"ui:modal\", \"ui:editor-decorations\"]",
        );
        let (_, report) = check(&ahead);
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(report.warnings.len(), 2, "{report:?}");
        assert!(report.warnings[0].contains("accepted but unused until fyx:ui 0.2.0"));
    }

    #[test]
    fn v0_4_tables_go_with_their_capabilities() {
        const LISTENER: &str = "[[fyx_ui.listeners]]\nname = \"ide\"\nprotocol = \"mcp-websocket\"\n\
             auth = { header = \"x-ide-authorization\" }\nreason = \"lets the CLI connect\"\n";
        let manifest = |api_version: &str, capabilities: &str, tables: &str| {
            VALID
                .replace(
                    "api_version = \"0.1.0\"",
                    &format!("api_version = \"{api_version}\""),
                )
                .replace("[\"ui:panel\"]", &format!("[\"ui:panel\"{capabilities}]"))
                + tables
        };
        let tables = format!(
            "\n[[fyx_ui.terminal_profiles]]\nid = \"cli\"\nname = \"CLI\"\nprogram = \"cli\"\n\
             [[fyx_ui.terminal_programs]]\nname = \"tool\"\nargs = [\"**\"]\nreason = \"runs the tool\"\n\
             [fyx_ui.terminal_env]\nvars = {{ CLI_PORT = \"${{listen.ide.port}}\" }}\nreason = \"connects the CLI\"\n\
             {LISTENER}"
        );
        let capabilities =
            ", \"terminal:profiles\", \"terminal:create\", \"terminal:env\", \"network:listen\"";

        let (_, report) = check(&manifest("0.4.0", capabilities, &tables));
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("0.4.0 is staged"), "{report:?}");

        // Below 0.4 the tables and their capabilities are refused.
        let (_, report) = check(&manifest("0.3.0", capabilities, &tables));
        for table in [
            "[[fyx_ui.terminal_profiles]]",
            "[[fyx_ui.terminal_programs]]",
            "[fyx_ui.terminal_env]",
            "[[fyx_ui.listeners]]",
        ] {
            let expected = format!("{table} needs fyx:ui 0.4.0; this extension targets 0.3.0");
            assert!(
                report.errors.iter().any(|error| error.contains(&expected)),
                "{expected}: {report:?}"
            );
        }
        assert!(
            report.errors.iter().any(|error| error
                .contains("\"terminal:env\" needs fyx:ui 0.4.0; this extension targets 0.3.0")),
            "{report:?}"
        );

        // Each table needs its capability, and each capability its table.
        let (_, report) = check(&manifest(
            "0.4.0",
            ", \"terminal:env\", \"relay:hooks\"",
            &tables,
        ));
        for expected in [
            "[[fyx_ui.terminal_profiles]] needs \"terminal:profiles\"",
            "[[fyx_ui.listeners]] needs \"network:listen\"",
            "capability \"relay:hooks\" needs [[fyx_ui.relay_channels]]",
        ] {
            assert!(
                report.errors.iter().any(|error| error.contains(expected)),
                "{expected}: {report:?}"
            );
        }

        // The host's entry checks: a template naming no listener.
        let unknown = tables.replace("${listen.ide.port}", "${listen.other.port}");
        let (_, report) = check(&manifest("0.4.0", capabilities, &unknown));
        assert_eq!(report.errors.len(), 1, "{report:?}");

        // T2 entries need a reason.
        let (parsed, _) = check(&manifest(
            "0.4.0",
            capabilities,
            &tables.replace("reason = \"runs the tool\"\n", ""),
        ));
        assert!(parsed.is_err());

        // The T3 terminal:read and terminal:write are declarable from 0.4, and not below.
        let (_, report) = check(&manifest(
            "0.4.0",
            ", \"terminal:read\", \"terminal:write\"",
            "",
        ));
        assert!(report.errors.is_empty(), "{report:?}");
        let (_, report) = check(&manifest("0.3.0", ", \"terminal:write\"", ""));
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("\"terminal:write\" needs fyx:ui 0.4")),
            "{report:?}"
        );
    }

    #[test]
    fn t2_items_need_a_reason_from_0_4_and_refuse_it_below() {
        let manifest = |version: &str, network: &str, process: &str| {
            VALID
                .replace(
                    "api_version = \"0.1.0\"",
                    &format!("api_version = \"{version}\""),
                )
                .replace(
                    "[\"ui:panel\"]",
                    "[\"ui:panel\", \"network:fetch\", \"process:spawn\"]",
                )
                + &format!(
                    "
[fyx_ui.network]
domains = [\"api.github.com\"]
{network}
                     [[fyx_ui.processes]]
name = \"git\"
args = [\"status\"]
{process}
"
                )
        };
        let reason = |text: &str| format!("reason = \"{text}\"");
        let errors = |text: &str| check(text).1.errors;

        let both = manifest("0.4.0", &reason("fetches zen"), &reason("shows status"));
        assert!(errors(&both).is_empty(), "{:?}", errors(&both));
        let errors_04 = errors(&manifest("0.4.0", "", ""));
        assert_eq!(errors_04.len(), 2, "{errors_04:?}");
        assert!(errors_04[0].contains("[[fyx_ui.processes]] \"git\": a reason is required"));
        assert!(errors_04[1].contains("[fyx_ui.network]: a reason is required"));
        let long = errors(&manifest("0.4.0", &reason(&"x".repeat(201)), &reason("ok")));
        assert!(
            long[0].contains("one line of at most 200 bytes"),
            "{long:?}"
        );

        let errors_03 = errors(&manifest("0.3.0", &reason("fetches zen"), ""));
        assert_eq!(errors_03.len(), 1, "{errors_03:?}");
        assert!(errors_03[0].contains("reason needs fyx:ui 0.4; this extension targets 0.3.0"));
        assert!(errors(&manifest("0.3.0", "", "")).is_empty());
    }

    #[test]
    fn package_placeholders_need_package_files_and_name_real_paths() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("scripts")).expect("mkdir");
        std::fs::write(dir.path().join("scripts/welcome.py"), "").expect("script");
        let manifest = |capabilities: &str, args: &str| {
            VALID
                .replace("api_version = \"0.1.0\"", "api_version = \"0.4.0\"")
                .replace(
                    "[\"ui:panel\"]",
                    &format!("[\"ui:panel\", \"terminal:profiles\"{capabilities}]"),
                )
                + &format!(
                    "\n[[fyx_ui.terminal_profiles]]\nid = \"py\"\nname = \"Py\"\n\
                     program = \"python\"\nargs = {args}\n"
                )
        };
        let check = |text: &str| {
            let mut report = Report::default();
            let manifest = parse(dir.path(), text, &mut report).expect("parses");
            (manifest, report)
        };
        let staged = |report: &Report| {
            report
                .warnings
                .iter()
                .filter(|warning| !warning.contains("is staged"))
                .cloned()
                .collect::<Vec<_>>()
        };

        let (parsed, report) = check(&manifest(
            ", \"package:files\"",
            "[\"-i\", \"${package}/scripts/welcome.py\"]",
        ));
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(staged(&report).is_empty(), "{report:?}");
        assert_eq!(parsed.package_files, ["scripts"]);

        let (_, report) = check(&manifest("", "[\"${package}/scripts/welcome.py\"]"));
        assert!(
            report.errors[0].contains("${package} needs \"package:files\""),
            "{report:?}"
        );

        let (_, report) = check(&manifest(", \"package:files\"", "[\"${package}/gone.py\"]"));
        assert!(
            report.errors[0].contains("${package}/gone.py is not in the extension folder"),
            "{report:?}"
        );

        let (_, report) = check(&manifest(", \"package:files\"", "[\"-i\"]"));
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(
            staged(&report)[0].contains("\"package:files\" grants nothing"),
            "{report:?}"
        );
    }

    #[test]
    fn processes_need_their_table_and_safe_env_names() {
        let spawn = VALID
            .replace("api_version = \"0.1.0\"", "api_version = \"0.3.0\"")
            .replace("[\"ui:panel\"]", "[\"ui:panel\", \"process:spawn\"]");
        let (_, report) = check(&spawn);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("needs [[fyx_ui.processes]]")),
            "{report:?}"
        );
        let declared = format!(
            "{spawn}\n[[fyx_ui.processes]]\nname = \"git\"\nargs = [\"status\", \"**\"]\n\n[fyx_ui.process]\nenv = [\"NO_COLOR\"]\n"
        );
        let (_, report) = check(&declared);
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(
            report.warnings.iter().all(|w| !w.contains("no `args`")),
            "{report:?}"
        );

        // Without `args` a program runs with no arguments only (integration-5 S6).
        let bare = format!("{spawn}\n[[fyx_ui.processes]]\nname = \"git\"\n");
        let (_, report) = check(&bare);
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(
            report.warnings.iter().any(|w| w.contains("no `args`")),
            "{report:?}"
        );
        // git's directories and PATH are never passed.
        for name in ["GIT_DIR", "PATH", "CARGO_HOME"] {
            let env = format!(
                "{spawn}\n[[fyx_ui.processes]]\nname = \"git\"\nargs = [\"**\"]\n\n[fyx_ui.process]\nenv = [\"{name}\"]\n"
            );
            let (_, report) = check(&env);
            assert_eq!(report.errors.len(), 1, "{name}: {report:?}");
        }

        let bad = format!(
            "{spawn}\n[[fyx_ui.processes]]\nname = \"bin/tool\"\nargs = [\"**\", \"x\"]\n\n[fyx_ui.process]\nenv = [\"LD_PRELOAD\"]\n"
        );
        let (_, report) = check(&bad);
        assert_eq!(report.errors.len(), 3, "{report:?}");
    }

    #[test]
    fn network_needs_its_table_and_0_3() {
        let fetch = VALID
            .replace("api_version = \"0.1.0\"", "api_version = \"0.3.0\"")
            .replace("[\"ui:panel\"]", "[\"ui:panel\", \"network:fetch\"]");
        let (_, report) = check(&fetch);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("needs [fyx_ui.network]")),
            "{report:?}"
        );

        let with_table = format!(
            "{fetch}\n[fyx_ui.network]\ndomains = [\"api.example.com\", \"localhost:8080\"]\n"
        );
        let (_, report) = check(&with_table);
        assert!(report.errors.is_empty(), "{report:?}");

        let bad = format!(
            "{fetch}\n[fyx_ui.network]\ndomains = [\"https://api.example.com\", \"localhost\"]\n"
        );
        let (_, report) = check(&bad);
        assert_eq!(report.errors.len(), 2, "{report:?}");

        let old = with_table.replace("api_version = \"0.3.0\"", "api_version = \"0.2.0\"");
        let (_, report) = check(&old);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("needs fyx:ui 0.3.0; this extension targets 0.2.0")),
            "{report:?}"
        );

        let orphan = format!("{VALID}\n[fyx_ui.network]\ndomains = [\"api.example.com\"]\n");
        let (_, report) = check(&orphan);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("needs \"network:fetch\"")),
            "{report:?}"
        );
    }

    #[test]
    fn ids() {
        for id in ["a", "my-panel", "x2-y3"] {
            assert!(is_valid_id(id), "{id}");
        }
        for id in ["", "-a", "a-", "a--b", "a_b", "A", "2a", &"a".repeat(65)] {
            assert!(!is_valid_id(id), "{id}");
        }
    }
}
