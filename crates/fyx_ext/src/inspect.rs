//! What a built component imports, exports and declares, read with `wasmparser` without running
//! it, and the checks `fyx-ext check` makes on that.

use anyhow::{Context as _, Result};
use semver::Version;
use wasmparser::{Encoding, Parser, Payload};

use crate::Report;
use crate::manifest::{Manifest, SURFACE_CAPABILITIES};

/// The custom section the SDK's `register_extension!` writes: three big-endian `u16`s.
pub const API_VERSION_SECTION: &str = "fyx:ui-api-version";

/// The host refuses larger components (platform spec, decision 12).
pub const MAX_COMPONENT_BYTES: u64 = 32 * 1024 * 1024;

/// Exports every `fyx:ui` 0.1 component must have.
pub const REQUIRED_EXPORTS: &[&str] = &["surfaces", "mount", "handle-event", "list-rows"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComponentInfo {
    /// Top-level import names, such as `fyx:ui/host@0.1.0`.
    pub imports: Vec<String>,
    /// Top-level export names, such as `mount`.
    pub exports: Vec<String>,
    /// Every `fyx:ui-api-version` section found, nested modules included.
    pub api_version_sections: Vec<Vec<u8>>,
}

/// What an import needs from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportNeed {
    /// Always linked, no capability.
    Nothing,
    /// Linked only for extensions that declare this capability.
    Capability(&'static str),
    /// WASI that the host links but backs with nothing (no preopens, sockets or HTTP): every call
    /// fails at run time.
    InertWasi,
    /// The host does not provide it: the component would fail to instantiate.
    Unavailable,
}

/// Which capability, if any, an import needs. Interfaces are matched without their version.
pub fn import_need(import: &str) -> ImportNeed {
    let interface = import.split('@').next().unwrap_or(import);
    match interface {
        "fyx:ui/types" | "fyx:ui/host" => ImportNeed::Nothing,
        // Linked for every 0.2 or 0.3 component, with the capability (`ui:editor-decorations`,
        // `ui:notifications`, `ui:images`, `network:fetch`) checked per call, so importing them
        // needs nothing. The SDK imports them whether or not the extension calls them.
        "fyx:ui/decorations"
        | "fyx:ui/notifications"
        | "fyx:ui/images"
        | "fyx:ui/network"
        | "fyx:ui/process"
        | "fyx:ui/workspace-edit"
        | "fyx:ui/agent-observe" => ImportNeed::Nothing,
        // The rest of the staged 0.3 world: the host links each interface it does not run yet
        // with functions that trap, so a component loads, and the SDK's 0.3 build imports their
        // types. A call traps, which `fyx-ext test` shows.
        _ if interface.starts_with("fyx:ui/") => ImportNeed::Nothing,
        _ if interface.starts_with("wasi:filesystem/")
            || interface.starts_with("wasi:sockets/")
            || interface.starts_with("wasi:http/") =>
        {
            ImportNeed::InertWasi
        }
        // What Rust's std pulls in on wasm32-wasip2; Zed's extension host links all of WASI 0.2.
        _ if interface.starts_with("wasi:") => ImportNeed::Nothing,
        _ => ImportNeed::Unavailable,
    }
}

pub fn inspect(bytes: &[u8]) -> Result<ComponentInfo> {
    let mut info = ComponentInfo::default();
    let mut depth = 0usize;
    let mut top_level_is_component = None;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.context("parsing the component")? {
            Payload::Version { encoding, .. } => {
                depth += 1;
                if depth == 1 {
                    top_level_is_component = Some(encoding == Encoding::Component);
                }
            }
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(reader) if depth == 1 => {
                for import in reader {
                    let import = import.context("reading an import")?;
                    // A world's `use types.{...}` also imports each type by name; those come with
                    // the interface and need nothing from the host on their own.
                    if !matches!(import.ty, wasmparser::ComponentTypeRef::Type(_)) {
                        info.imports.push(import.name.name.to_string());
                    }
                }
            }
            Payload::ComponentExportSection(reader) if depth == 1 => {
                for export in reader {
                    info.exports
                        .push(export.context("reading an export")?.name.name.to_string());
                }
            }
            Payload::CustomSection(section) if section.name() == API_VERSION_SECTION => {
                info.api_version_sections.push(section.data().to_vec());
            }
            _ => {}
        }
    }
    anyhow::ensure!(
        top_level_is_component == Some(true),
        "this is a core WebAssembly module, not a component; build for wasm32-wasip2"
    );
    Ok(info)
}

pub fn decode_api_version(data: &[u8]) -> Option<Version> {
    let [
        major_high,
        major_low,
        minor_high,
        minor_low,
        patch_high,
        patch_low,
    ] = *data
    else {
        return None;
    };
    Some(Version::new(
        u16::from_be_bytes([major_high, major_low]).into(),
        u16::from_be_bytes([minor_high, minor_low]).into(),
        u16::from_be_bytes([patch_high, patch_low]).into(),
    ))
}

/// Checks a built component against its manifest.
pub fn check_component(manifest: &Manifest, bytes: &[u8], report: &mut Report) {
    if bytes.len() as u64 > MAX_COMPONENT_BYTES {
        report.error(format!(
            "the component is {} bytes; the cap is {MAX_COMPONENT_BYTES} (32 MiB). Build with --release",
            bytes.len()
        ));
    }
    if let Err(error) = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(bytes)
    {
        report.error(format!("the component does not validate: {error}"));
        return;
    }
    let info = match inspect(bytes) {
        Ok(info) => info,
        Err(error) => {
            report.error(format!("{error:#}"));
            return;
        }
    };

    check_api_version(manifest, &info, report);

    for export in REQUIRED_EXPORTS {
        if !info.exports.iter().any(|name| name == export) {
            report.error(format!(
                "the component does not export `{export}`; register the extension with register_extension!"
            ));
        }
    }

    check_capabilities(&manifest.capabilities, &info.imports, report);
}

fn check_api_version(manifest: &Manifest, info: &ComponentInfo, report: &mut Report) {
    match info.api_version_sections.as_slice() {
        [] => {
            report.warning(format!(
                "no {API_VERSION_SECTION} section: the SDK predates it, so the host falls back to the versions of the fyx:ui imports"
            ));
            check_import_versions(manifest, info, report);
        }
        [data] => match (decode_api_version(data), &manifest.api_version) {
            (None, _) => report.error(format!(
                "the {API_VERSION_SECTION} section is not 6 bytes: {data:?}"
            )),
            (Some(component), Some(declared)) if &component != declared => report.error(format!(
                "the component implements fyx:ui {component}, but [fyx_ui].api_version says {declared}; the host refuses the mismatch"
            )),
            _ => {}
        },
        _ => report.error(format!(
            "{} {API_VERSION_SECTION} sections; the host reads them as one invalid section. Remove the extra one (only register_extension! should write it)",
            info.api_version_sections.len()
        )),
    }
}

/// The versions in the `@x.y.z` of the component's `fyx:ui/*` imports, in import order without
/// repeats.
pub fn import_versions(info: &ComponentInfo) -> Vec<String> {
    let mut versions = Vec::new();
    for import in &info.imports {
        if let Some((interface, version)) = import.split_once('@')
            && interface.starts_with("fyx:ui/")
            && !versions.iter().any(|known| known == version)
        {
            versions.push(version.to_string());
        }
    }
    versions
}

/// Without a version section the host takes the version from the `fyx:ui` imports (bugs-1 #25),
/// so they must agree with each other and with `[fyx_ui].api_version`.
fn check_import_versions(manifest: &Manifest, info: &ComponentInfo, report: &mut Report) {
    match import_versions(info).as_slice() {
        [] => report.error(
            "the component imports no fyx:ui interface and has no version section, so the host cannot tell which world it implements",
        ),
        [version] => match (Version::parse(version), &manifest.api_version) {
            (Err(_), _) => report.error(format!(
                "the fyx:ui imports carry the version {version:?}, which is not a version"
            )),
            (Ok(component), Some(declared)) if &component != declared => {
                report.error(format!(
                    "the component imports fyx:ui {component}, but [fyx_ui].api_version says {declared}; the host refuses the mismatch"
                ))
            }
            _ => {}
        },
        versions => report.error(format!(
            "the component imports fyx:ui interfaces of several versions ({}); a component implements one world",
            versions.join(", ")
        )),
    }
}

/// Capabilities checked when their functions are called, not by an import: a component imports
/// their interfaces anyway, so nothing here shows whether it uses them.
const PER_CALL_CAPABILITIES: &[&str] = &[
    "ui:editor-decorations",
    "ui:notifications",
    "ui:images",
    "ui:canvas",
    "network:fetch",
    "process:spawn",
    "workspace:edit",
    "agent:observe",
    // 0.4: checked per call, or used by a manifest table (profiles, env, listeners, discovery
    // files, relay channels, `${package}`) that the manifest check pairs with its capability.
    "terminal:profiles",
    "terminal:pseudo",
    "terminal:create",
    "terminal:env",
    "terminal:status",
    "terminal:observe",
    // `request-read` and `request-write` are functions of `fyx:ui/terminals`, not interfaces of
    // their own, so no import names them.
    "terminal:read",
    "terminal:write",
    "package:files",
    "network:listen",
    "fs:discovery-file",
    "relay:hooks",
    "review:propose",
    "context:editor",
    "workspace:open",
    "ui:approvals",
    "workspace:diagnostics",
    "diagnostics:publish",
    "workspace:file-decorations",
];

/// Declared capabilities against the imports that need them: an import whose capability is not
/// declared is an error, a declared capability nothing uses is a warning (platform spec,
/// decision 5).
pub fn check_capabilities(declared: &[String], imports: &[String], report: &mut Report) {
    let mut used = Vec::new();
    for import in imports {
        match import_need(import) {
            ImportNeed::Nothing => {}
            ImportNeed::Capability(capability) => {
                used.push(capability);
                if !declared.iter().any(|name| name == capability) {
                    report.error(format!(
                        "the component imports {import}, which needs {capability:?} in [fyx_ui].capabilities"
                    ));
                }
            }
            ImportNeed::InertWasi => report.warning(format!(
                "the component imports {import}; the host grants no files, sockets or HTTP to extensions, so every call fails"
            )),
            ImportNeed::Unavailable => report.error(format!(
                "the component imports {import}, which Fyx does not provide; it would fail to load"
            )),
        }
    }
    for capability in declared {
        // Used by `surfaces()`, not by an import: checked by loading the component
        // (`CheckDepth::Load`).
        if SURFACE_CAPABILITIES.contains(&capability.as_str())
            || PER_CALL_CAPABILITIES.contains(&capability.as_str())
        {
            continue;
        }
        if !used.contains(&capability.as_str())
            && crate::manifest::KNOWN_CAPABILITIES
                .iter()
                .any(|(name, _)| name == capability)
        {
            report.warning(format!(
                "{capability:?} is declared but nothing in the component uses it"
            ));
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Components that import and export everything in a world, built with
    /// `wasm-tools component embed --dummy` (see `fixtures/README.md`).
    pub fn component_v0_1() -> Vec<u8> {
        include_bytes!("../fixtures/dummy-v0.1.wasm").to_vec()
    }

    pub fn component_v0_2() -> Vec<u8> {
        include_bytes!("../fixtures/dummy-v0.2.wasm").to_vec()
    }

    /// Appends a custom section to a component, as `register_extension!` does inside its module.
    pub fn with_custom_section(mut bytes: Vec<u8>, name: &str, data: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        leb128(&mut payload, name.len());
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(data);
        bytes.push(0);
        leb128(&mut bytes, payload.len());
        bytes.extend_from_slice(&payload);
        bytes
    }

    fn leb128(out: &mut Vec<u8>, mut value: usize) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn manifest(capabilities: &[&str]) -> Manifest {
        Manifest {
            dir: PathBuf::from("ext"),
            id: "x".into(),
            version: "0.1.0".into(),
            api_version: Some(Version::new(0, 1, 0)),
            component: "fyx_ui.wasm".into(),
            capabilities: capabilities.iter().map(ToString::to_string).collect(),
            package_files: Vec::new(),
        }
    }

    #[test]
    fn reads_the_world_of_a_0_1_component() {
        let info = inspect(&component_v0_1()).expect("inspect");
        assert!(
            info.imports.contains(&"fyx:ui/host@0.1.0".to_string()),
            "{info:?}"
        );
        for export in REQUIRED_EXPORTS {
            assert!(info.exports.iter().any(|name| name == export), "{export}");
        }
        assert!(info.api_version_sections.is_empty());
    }

    #[test]
    fn a_0_1_component_with_its_section_passes() {
        let bytes = with_custom_section(component_v0_1(), API_VERSION_SECTION, &[0, 0, 0, 1, 0, 0]);
        let mut report = Report::default();
        check_component(&manifest(&["ui:panel"]), &bytes, &mut report);
        assert!(
            report.errors.is_empty() && report.warnings.is_empty(),
            "{report:?}"
        );
        assert!(
            report.is_empty(),
            "ui:panel is checked by loading the component: {report:?}"
        );
    }

    #[test]
    fn version_section_problems() {
        let mut report = Report::default();
        check_component(&manifest(&[]), &component_v0_1(), &mut report);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("no fyx:ui-api-version")),
            "{report:?}"
        );

        let wrong = with_custom_section(component_v0_1(), API_VERSION_SECTION, &[0, 0, 0, 2, 0, 0]);
        let mut report = Report::default();
        check_component(&manifest(&[]), &wrong, &mut report);
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("implements fyx:ui 0.2.0")),
            "{report:?}"
        );

        let twice = with_custom_section(wrong, API_VERSION_SECTION, &[0, 0, 0, 1, 0, 0]);
        let mut report = Report::default();
        check_component(&manifest(&[]), &twice, &mut report);
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("2 fyx:ui-api-version")),
            "{report:?}"
        );
    }

    /// bugs-1 #25: without a section the host reads the import versions, and so does `check`.
    #[test]
    fn without_a_section_the_import_versions_must_match_the_manifest() {
        let mut report = Report::default();
        check_component(&manifest(&[]), &component_v0_1(), &mut report);
        assert!(report.errors.is_empty(), "{report:?}");

        let mut report = Report::default();
        check_component(&manifest(&[]), &component_v0_2(), &mut report);
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("imports fyx:ui 0.2.0, but [fyx_ui].api_version says 0.1.0")),
            "{report:?}"
        );

        let info = ComponentInfo {
            imports: vec!["fyx:ui/types@0.1.0".into(), "fyx:ui/host@0.2.0".into()],
            ..ComponentInfo::default()
        };
        assert_eq!(import_versions(&info), ["0.1.0", "0.2.0"]);
        let mut report = Report::default();
        check_api_version(&manifest(&[]), &info, &mut report);
        assert!(
            report.errors.iter().any(|e| e.contains("several versions")),
            "{report:?}"
        );
    }

    #[test]
    fn a_core_module_is_not_a_component() {
        let error = inspect(b"\0asm\x01\0\0\0").expect_err("a core module");
        assert!(error.to_string().contains("not a component"), "{error}");
    }

    #[test]
    fn decorations_are_checked_per_call_not_at_import() {
        let info = inspect(&component_v0_2()).expect("inspect");
        assert!(
            info.imports
                .iter()
                .any(|name| name == "fyx:ui/decorations@0.2.0"),
            "{info:?}"
        );
        let mut report = Report::default();
        check_capabilities(&["ui:panel".into()], &info.imports, &mut report);
        assert!(report.errors.is_empty(), "{report:?}");
    }

    /// Every capability the host provides is used by an import, a surface, a call or a manifest
    /// table, so `fyx-ext check` never calls a provided capability unused because of how it is
    /// reached (`terminal:read` and `terminal:write` are functions, not interfaces).
    #[test]
    fn every_provided_capability_is_reachable() {
        let by_import: Vec<&str> = crate::manifest::KNOWN_CAPABILITIES
            .iter()
            .filter(|(name, _)| {
                matches!(
                    import_need(&format!("fyx:ui/{name}")),
                    ImportNeed::Capability(capability) if capability == *name
                )
            })
            .map(|(name, _)| *name)
            .collect();
        for (capability, status) in crate::manifest::KNOWN_CAPABILITIES {
            if !matches!(status, crate::manifest::CapabilityStatus::Since(_)) {
                continue;
            }
            assert!(
                SURFACE_CAPABILITIES.contains(capability)
                    || PER_CALL_CAPABILITIES.contains(capability)
                    || by_import.contains(capability),
                "{capability:?} would be reported as declared but unused"
            );
        }
        // The probe's case: a 0.4 component that imports the terminals interface and declares
        // terminal:read and terminal:write.
        let mut report = Report::default();
        check_capabilities(
            &["terminal:read".to_owned(), "terminal:write".to_owned()],
            &["fyx:ui/terminals@0.4.0".to_owned()],
            &mut report,
        );
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
    }

    #[test]
    fn declared_but_unused_and_unknown_imports() {
        let imports = vec![
            "fyx:ui/host@0.1.0".to_string(),
            "wasi:filesystem/preopens@0.2.9".to_string(),
            "acme:secret/api@1.0.0".to_string(),
        ];
        let mut report = Report::default();
        check_capabilities(&["ui:hover".into()], &imports, &mut report);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("declared but nothing")),
            "{report:?}"
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("wasi:filesystem")),
            "{report:?}"
        );
        assert!(
            report.errors.iter().any(|e| e.contains("acme:secret/api")),
            "{report:?}"
        );
    }

    #[test]
    fn api_version_bytes() {
        assert_eq!(
            decode_api_version(&[0, 0, 0, 1, 0, 0]),
            Some(Version::new(0, 1, 0))
        );
        assert_eq!(
            decode_api_version(&[1, 0, 0, 2, 0, 3]),
            Some(Version::new(256, 2, 3))
        );
        assert_eq!(decode_api_version(&[0, 1]), None);
    }
}
