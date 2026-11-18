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
pub const SUPPORTED_API_VERSIONS: &[Version] = &[Version::new(0, 1, 0)];

/// Capabilities a manifest may declare (platform spec, decision 21). Ones the current world does
/// not provide yet are accepted with a warning, so a manifest can be written ahead of the host.
pub const KNOWN_CAPABILITIES: &[(&str, CapabilityStatus)] = &[
    ("ui:panel", CapabilityStatus::Available),
    ("ui:status-bar", CapabilityStatus::Available),
    ("ui:modal", CapabilityStatus::Planned("fyx:ui 0.2")),
    (
        "ui:editor-decorations",
        CapabilityStatus::Planned("fyx:ui 0.2"),
    ),
    ("ui:notifications", CapabilityStatus::Planned("fyx:ui 0.2")),
    ("ui:picker", CapabilityStatus::Planned("fyx:ui 0.3")),
    ("ui:code-lens", CapabilityStatus::Planned("fyx:ui 0.3")),
    ("ui:hover", CapabilityStatus::Planned("fyx:ui 0.3")),
    (
        "ui:virtual-documents",
        CapabilityStatus::Planned("fyx:ui 0.3"),
    ),
    ("ui:commands", CapabilityStatus::Planned("fyx:ui 0.3")),
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
        "network:fetch",
        CapabilityStatus::Planned("a later release"),
    ),
    (
        "process:spawn",
        CapabilityStatus::Planned("a later release"),
    ),
    (
        "agent:observe",
        CapabilityStatus::Planned("a later release"),
    ),
];

/// Capabilities that a surface kind needs, so they are used by `surfaces()` rather than by an
/// import.
pub const SURFACE_CAPABILITIES: &[&str] = &["ui:panel", "ui:status-bar", "ui:modal"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityStatus {
    Available,
    Planned(&'static str),
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
        Ok(version) if SUPPORTED_API_VERSIONS.contains(&version) => Some(version),
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
            Some((_, CapabilityStatus::Available)) => {}
        }
    }

    Ok(Manifest {
        dir: dir.to_path_buf(),
        id: toml.id,
        version: toml.version,
        api_version,
        component: table.component,
        capabilities: table.capabilities,
    })
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
            "[\"ui:panel\", \"ui:panel\", \"network:fetch\"]",
        );
        let (_, report) = check(&text);
        assert!(report.errors.is_empty(), "{report:?}");
        assert_eq!(report.warnings.len(), 2, "{report:?}");
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
