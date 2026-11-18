//! `extension.toml`: the fields Fyx reads, checked with the host's rules
//! (`extension_host::wasm_host::fyx_ui::UiManifest`) plus the registry's naming rules.

use std::path::{Component as PathComponent, Path, PathBuf};

use anyhow::{Context as _, Result};
use semver::Version;
use serde::Deserialize;

use crate::Report;

pub const MANIFEST_FILE: &str = "extension.toml";

/// The `fyx:ui` versions this `fyx-ext` knows how to check, matching the host's
/// `MIN_VERSION..=MAX_VERSION`.
pub const SUPPORTED_API_VERSIONS: &[Version] = &[
    Version::new(0, 1, 0),
    Version::new(0, 2, 0),
    Version::new(0, 3, 0),
];

/// The newest frozen version, the host's `MAX_FROZEN_VERSION`. Newer ones are staged: dev builds
/// of Fyx load them (debug builds, or a process started with `FYX_UI_DEV_EXTENSIONS`), releases
/// refuse them until they are frozen.
pub const MAX_FROZEN_API_VERSION: Version = Version::new(0, 1, 0);

/// `[fyx_ui.network] domains` entries per manifest, the host's `network::MAX_DOMAINS`.
pub const MAX_NETWORK_DOMAINS: usize = 64;

const NETWORK_FETCH: &str = "network:fetch";
const PROCESS_SPAWN: &str = "process:spawn";

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
        "workspace:diagnostics",
        CapabilityStatus::Planned("a later release"),
    ),
    (
        "workspace:edit",
        CapabilityStatus::Planned("a later release"),
    ),
    (
        PROCESS_SPAWN,
        CapabilityStatus::Since(Version::new(0, 3, 0)),
    ),
    (
        "agent:observe",
        CapabilityStatus::Planned("a later release"),
    ),
];

/// Capabilities that a surface kind needs, so they are used by `surfaces()` rather than by an
/// import.
pub const SURFACE_CAPABILITIES: &[&str] = &["ui:panel", "ui:status-bar", "ui:modal"];

/// fyx:ui 0.3 parts the host does not run yet: their imports trap and their nodes are refused.
const NOT_BUILT: &str = "a later build of fyx:ui 0.3";

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
            Some((_, CapabilityStatus::Planned(release))) => report.warning(format!(
                "capability {capability:?} arrives with {release}; the current host ignores it"
            )),
            Some((_, CapabilityStatus::Since(since))) => match &api_version {
                // The host refuses a T2 capability it cannot grant at that version.
                Some(version)
                    if version < since
                        && (capability == NETWORK_FETCH || capability == PROCESS_SPAWN) =>
                {
                    report.error(format!(
                        "capability {capability:?} needs [fyx_ui].api_version {since} or newer"
                    ))
                }
                Some(version) if version < since => report.warning(format!(
                    "capability {capability:?} needs fyx:ui {since}; an extension that targets \
                     {version} cannot use it"
                )),
                _ => {}
            },
        }
    }
    check_network(&table, report);
    check_processes(&table, report);

    Ok(Manifest {
        dir: dir.to_path_buf(),
        id: toml.id,
        version: toml.version,
        api_version,
        component: table.component,
        capabilities: table.capabilities,
    })
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
    fn planned_and_duplicate_capabilities_are_warnings() {
        let text = VALID.replace(
            "[\"ui:panel\"]",
            "[\"ui:panel\", \"ui:panel\", \"ui:hover\"]",
        );
        let (_, report) = check(&text);
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(report.warnings.len(), 2, "{report:?}");
    }

    #[test]
    fn staged_versions_and_their_capabilities() {
        let staged = VALID
            .replace("api_version = \"0.1.0\"", "api_version = \"0.3.0\"")
            .replace("[\"ui:panel\"]", "[\"ui:panel\", \"ui:images\"]");
        let (manifest, report) = check(&staged);
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("staged"), "{report:?}");
        assert_eq!(
            manifest.expect("parses").api_version,
            Some(Version::new(0, 3, 0))
        );

        let early = VALID.replace("[\"ui:panel\"]", "[\"ui:panel\", \"ui:notifications\"]");
        let (_, report) = check(&early);
        assert!(report.errors.is_empty(), "{report:?}");
        assert!(
            report.warnings[0].contains("needs fyx:ui 0.2.0"),
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
            "{spawn}\n[[fyx_ui.processes]]\nname = \"git\"\nargs = [\"status\", \"**\"]\n\n[fyx_ui.process]\nenv = [\"GIT_DIR\"]\n"
        );
        let (_, report) = check(&declared);
        assert!(report.errors.is_empty(), "{report:?}");

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
                .any(|error| error.contains("needs [fyx_ui].api_version 0.3.0")),
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
