//! `fyx-ext`: create, build, check, run and package Fyx UI extensions (platform spec,
//! decisions 4, 5 and 12; plan task 1.4).
//!
//! Apache-2.0, like the SDK, because extension authors run it on their own code.

pub mod build;
pub mod inspect;
pub mod manifest;
pub mod package;
pub mod run;
pub mod template;

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Result;

use crate::manifest::Manifest;

/// What a check found, most severe first when printed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// The host would refuse the extension, or it would not work.
    pub errors: Vec<String>,
    /// It loads, but something is likely wrong.
    pub warnings: Vec<String>,
    /// Facts the author should know, such as checks that only the host can make.
    pub notes: Vec<String>,
}

impl Report {
    pub fn error(&mut self, message: impl Into<String>) {
        self.errors.push(message.into());
    }

    pub fn warning(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.notes.push(message.into());
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty() && self.notes.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn print(&self) {
        for error in &self.errors {
            eprintln!("error: {error}");
        }
        for warning in &self.warnings {
            eprintln!("warning: {warning}");
        }
        for note in &self.notes {
            eprintln!("note: {note}");
        }
    }
}

/// How far `check` goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckDepth {
    /// The manifest and what can be read from the component without running it.
    Static,
    /// Also load the component in [`fyx_ext_harness`] and check the capabilities its surfaces
    /// need, as the host does when it loads the extension.
    Load,
}

/// Checks the extension in `dir`: its manifest, and the built component it names.
pub fn check(dir: &Path, depth: CheckDepth) -> Result<(Manifest, Report)> {
    let (manifest, report, _) = check_loaded(dir, depth)?;
    Ok((manifest, report))
}

/// [`check`], also returning the loaded component when `depth` is [`CheckDepth::Load`] and it
/// loaded.
pub fn check_loaded(
    dir: &Path,
    depth: CheckDepth,
) -> Result<(Manifest, Report, Option<fyx_ext_harness::Harness>)> {
    let mut report = Report::default();
    let manifest = manifest::read(dir, &mut report)?;
    let component = manifest.component_path();
    let mut harness = None;
    match std::fs::read(&component) {
        Ok(bytes) => {
            inspect::check_component(&manifest, &bytes, &mut report);
            if depth == CheckDepth::Load && !report.has_errors() {
                harness = run::load(&bytes, &mut report);
                if let Some(harness) = &harness {
                    run::check_surfaces(&manifest, harness, &mut report);
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => report.error(format!(
            "{} does not exist yet; run `fyx-ext build`",
            component.display()
        )),
        Err(error) => report.error(format!("reading {}: {error}", component.display())),
    }
    Ok((manifest, report, harness))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::tests::{component_v0_1, with_custom_section};

    #[test]
    fn check_reads_the_component_the_manifest_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("extension.toml"),
            "id = \"x\"\nname = \"X\"\nversion = \"0.1.0\"\nschema_version = 1\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"out/x.wasm\"\ncapabilities = [\"ui:panel\"]\n",
        )
        .expect("write the manifest");

        let (_, report) = check(dir.path(), CheckDepth::Static).expect("check");
        assert!(
            report.errors.iter().any(|e| e.contains("fyx-ext build")),
            "{report:?}"
        );

        std::fs::create_dir(dir.path().join("out")).expect("mkdir");
        let bytes = with_custom_section(
            component_v0_1(),
            inspect::API_VERSION_SECTION,
            &[0, 0, 0, 1, 0, 0],
        );
        std::fs::write(dir.path().join("out/x.wasm"), bytes).expect("write the component");
        let (manifest, report) = check(dir.path(), CheckDepth::Static).expect("check");
        assert_eq!(manifest.id, "x");
        assert!(
            !report.has_errors() && report.warnings.is_empty(),
            "{report:?}"
        );
    }
}
