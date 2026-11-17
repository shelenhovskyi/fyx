//! What `fyx-ext` learns by running the component in [`fyx_ext_harness`]: the surface-kind
//! capabilities `check` cannot see statically, and the headless smoke run of `fyx-ext test`.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context as _, Result};
use fyx_ext_harness::{COMPONENT_VARIABLE, Harness};
use ui_ext::{Props, SurfaceKind};

use crate::build::{self, BuildOptions};
use crate::manifest::{Manifest, SURFACE_CAPABILITIES};
use crate::{CheckDepth, Report};

/// Rows the smoke run asks for, per list or table.
pub const ROW_SAMPLE: u32 = 50;

pub fn capability_for(kind: &SurfaceKind) -> &'static str {
    match kind {
        SurfaceKind::Panel(_) => "ui:panel",
        SurfaceKind::StatusItem(_) => "ui:status-bar",
    }
}

/// Loads the component the way the host does; a failure is what the host would report.
pub fn load(bytes: &[u8], report: &mut Report) -> Option<Harness> {
    match Harness::load(bytes) {
        Ok(harness) => Some(harness),
        Err(error) => {
            report.error(format!("Fyx would fail to load the component: {error:#}"));
            None
        }
    }
}

/// Every surface needs the capability of its kind (the host refuses the extension otherwise); a
/// surface capability no surface uses is a warning.
pub fn check_surfaces(manifest: &Manifest, harness: &Harness, report: &mut Report) {
    let surfaces = harness.surfaces();
    for surface in surfaces {
        let needed = capability_for(&surface.kind);
        if !manifest
            .capabilities
            .iter()
            .any(|capability| capability == needed)
        {
            report.error(format!(
                "surface {:?} needs {needed:?} in [fyx_ui].capabilities; the host refuses the extension without it",
                surface.id
            ));
        }
    }
    for capability in &manifest.capabilities {
        let used = surfaces
            .iter()
            .any(|surface| capability_for(&surface.kind) == capability);
        if SURFACE_CAPABILITIES.contains(&capability.as_str())
            && !used
            && crate::manifest::KNOWN_CAPABILITIES
                .iter()
                .any(|(name, status)| {
                    name == capability && *status == crate::manifest::CapabilityStatus::Available
                })
        {
            report.warning(format!(
                "{capability:?} is declared but no surface needs it"
            ));
        }
    }
    if surfaces.is_empty() {
        report.warning("surfaces() returns no surfaces, so the extension shows nothing");
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SmokeSummary {
    pub surfaces: usize,
    pub nodes: usize,
    pub rows: usize,
}

/// Mounts every surface and fetches the first rows of every list and table, as the host would
/// when the user opens them. Problems the host would hit become errors.
pub fn smoke(harness: &mut Harness, report: &mut Report) -> SmokeSummary {
    let mut summary = SmokeSummary::default();
    for surface in harness.surfaces().to_vec() {
        let tree = match harness.mount(&surface.id) {
            Ok(tree) => tree,
            Err(error) => {
                report.error(format!("mounting {:?} fails: {error:#}", surface.id));
                continue;
            }
        };
        summary.surfaces += 1;
        summary.nodes += tree.len();
        let lists = tree
            .to_nodes()
            .into_iter()
            .filter_map(|node| match node.props {
                Props::List(list) => Some((node.id, list.row_count)),
                Props::Table(table) => Some((node.id, table.row_count)),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (list, row_count) in lists {
            let count = row_count.min(ROW_SAMPLE);
            if count == 0 {
                continue;
            }
            match harness.rows(&surface.id, list, 0, count) {
                Ok(rows) if rows.is_empty() => report.warning(format!(
                    "list {list} on {:?} has {row_count} rows, but list-rows(0, {count}) returned none",
                    surface.id
                )),
                Ok(rows) => summary.rows += rows.len(),
                Err(error) => report.error(format!(
                    "rows of list {list} on {:?}: {error:#}",
                    surface.id
                )),
            }
        }
    }
    for (level, message) in harness.logs() {
        if *level == ui_ext::LogLevel::Error {
            report.warning(format!("the extension logged an error: {message}"));
        }
    }
    summary
}

/// `fyx-ext test`: build, check, the smoke run, then the crate's own `cargo test` with
/// `FYX_EXT_COMPONENT` set to the built component. Returns whether everything passed.
pub fn test(dir: &Path, cargo_args: &[String]) -> Result<bool> {
    build::build(dir, &BuildOptions::default())?;
    let (manifest, mut report, harness) = crate::check_loaded(dir, CheckDepth::Load)?;
    if let Some(mut harness) = harness {
        let summary = smoke(&mut harness, &mut report);
        eprintln!(
            "fyx-ext: mounted {} surface(s) with {} node(s), fetched {} row(s)",
            summary.surfaces, summary.nodes, summary.rows
        );
    }
    report.print();
    if report.has_errors() {
        return Ok(false);
    }

    let component = manifest
        .component_path()
        .canonicalize()
        .context("resolving the built component")?;
    let mut command =
        smol::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    command
        .arg("test")
        .arg("--manifest-path")
        .arg(dir.join("Cargo.toml"))
        .args(cargo_args)
        .env(COMPONENT_VARIABLE, &component)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = smol::block_on(command.status()).context("running cargo test")?;
    Ok(status.success())
}
