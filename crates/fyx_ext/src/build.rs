//! `fyx-ext build` and `fyx-ext dev`: cargo for `wasm32-wasip2`, the component copied next to the
//! manifest, and a dev loop that rebuilds on change while Fyx runs.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::Report;
use crate::manifest::{self, MANIFEST_FILE, Manifest};

pub const TARGET: &str = "wasm32-wasip2";
/// The variable the dev build reads at startup (`ui_ext_host` registry).
pub const DEV_EXTENSIONS_VARIABLE: &str = "FYX_UI_DEV_EXTENSIONS";
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    pub release: bool,
    /// Passed to `cargo build` as is, such as `-j 2`.
    pub cargo_args: Vec<String>,
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    manifest_path: Option<PathBuf>,
    target: Option<CargoTarget>,
    #[serde(default)]
    filenames: Vec<PathBuf>,
}

#[derive(Deserialize)]
struct CargoTarget {
    crate_types: Vec<String>,
}

/// Builds the extension in `dir` and copies the component to the path its manifest names.
pub fn build(dir: &Path, options: &BuildOptions) -> Result<Manifest> {
    let manifest = manifest::read(dir, &mut Report::default())?;
    let cargo_toml = dir.join("Cargo.toml");
    anyhow::ensure!(
        cargo_toml.is_file(),
        "{} has no Cargo.toml; fyx-ext builds Rust extensions",
        dir.display()
    );

    let mut command = smol::process::Command::new(cargo());
    command
        .args([
            "build",
            "--target",
            TARGET,
            "--message-format=json-render-diagnostics",
        ])
        .arg("--manifest-path")
        .arg(&cargo_toml)
        .args(options.release.then_some("--release"))
        .args(&options.cargo_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let output = smol::block_on(command.output()).context("running cargo")?;
    anyhow::ensure!(output.status.success(), "cargo build failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let artifact = find_component(&stdout, &cargo_toml)?;
    let destination = manifest.component_path();
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {parent:?}"))?;
    }
    std::fs::copy(&artifact, &destination)
        .with_context(|| format!("copying {artifact:?} to {destination:?}"))?;
    eprintln!(
        "fyx-ext: built {} -> {}",
        artifact.display(),
        destination.display()
    );
    Ok(manifest)
}

/// The `.wasm` of the `cdylib` that cargo built for `cargo_toml`, from its JSON messages.
pub fn find_component(messages: &str, cargo_toml: &Path) -> Result<PathBuf> {
    let wanted = normalize(cargo_toml);
    let mut candidates = Vec::new();
    for line in messages.lines() {
        let Ok(message) = serde_json::from_str::<CargoMessage>(line) else {
            continue;
        };
        let is_cdylib = message
            .target
            .as_ref()
            .is_some_and(|target| target.crate_types.iter().any(|kind| kind == "cdylib"));
        if message.reason != "compiler-artifact" || !is_cdylib {
            continue;
        }
        let ours = message
            .manifest_path
            .as_deref()
            .is_some_and(|path| normalize(path) == wanted);
        for file in message.filenames {
            if file
                .extension()
                .is_some_and(|extension| extension == "wasm")
            {
                candidates.push((ours, file));
            }
        }
    }
    candidates.sort_by_key(|(ours, _)| !*ours);
    candidates
        .into_iter()
        .next()
        .map(|(_, file)| file)
        .context("cargo built no .wasm; the crate needs `crate-type = [\"cdylib\"]`")
}

fn normalize(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").unwrap_or(&text).to_lowercase()
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

#[derive(Debug, Clone, Default)]
pub struct DevOptions {
    pub build: BuildOptions,
    /// The Fyx binary; found as `FYX_BIN` or `fyx` on `PATH` when not given.
    pub fyx: Option<PathBuf>,
    pub launch: bool,
    pub watch: bool,
    /// Passed to Fyx, such as `--user-data-dir <dir>`.
    pub fyx_args: Vec<String>,
}

/// Builds and checks, starts Fyx with the extension in `FYX_UI_DEV_EXTENSIONS`, then rebuilds
/// whenever a source file changes until Fyx exits. The host reloads the component when the file
/// changes (plan task 1.3); before that lands, restart Fyx to see a rebuild.
pub fn dev(dir: &Path, options: &DevOptions) -> Result<()> {
    let dir = PathBuf::from(display_path(
        &dir.canonicalize()
            .with_context(|| format!("opening {}", dir.display()))?,
    ));
    build(&dir, &options.build)?;
    let (_, report) = crate::check(&dir, crate::CheckDepth::Load)?;
    report.print();
    anyhow::ensure!(!report.has_errors(), "fix the errors above first");

    let extensions = dev_extensions_value(
        &display_path(&dir),
        std::env::var(DEV_EXTENSIONS_VARIABLE).ok(),
    );
    let mut child = if options.launch {
        let fyx = find_fyx(options.fyx.as_deref())?;
        eprintln!(
            "fyx-ext: starting {} with {DEV_EXTENSIONS_VARIABLE}={extensions}",
            fyx.display()
        );
        Some(
            smol::process::Command::new(&fyx)
                .args(&options.fyx_args)
                .env(DEV_EXTENSIONS_VARIABLE, &extensions)
                .spawn()
                .with_context(|| format!("starting {}", fyx.display()))?,
        )
    } else {
        eprintln!("fyx-ext: start Fyx with {DEV_EXTENSIONS_VARIABLE}={extensions}");
        None
    };

    if !options.watch {
        return Ok(());
    }
    eprintln!(
        "fyx-ext: watching {} for changes (Ctrl-C to stop)",
        dir.display()
    );
    let mut snapshot = source_snapshot(&dir);
    loop {
        std::thread::sleep(WATCH_INTERVAL);
        if let Some(child) = child.as_mut()
            && let Some(status) = child.try_status().context("waiting for Fyx")?
        {
            eprintln!("fyx-ext: Fyx exited ({status})");
            return Ok(());
        }
        let current = source_snapshot(&dir);
        if current == snapshot {
            continue;
        }
        snapshot = current;
        match build(&dir, &options.build).and_then(|_| crate::check(&dir, crate::CheckDepth::Load))
        {
            Ok((_, report)) => report.print(),
            Err(error) => eprintln!("error: {error:#}"),
        }
    }
}

/// `FYX_UI_DEV_EXTENSIONS` with `dir` added in front of what is already set.
pub fn dev_extensions_value(dir: &str, existing: Option<String>) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    match existing.filter(|existing| !existing.is_empty()) {
        Some(existing) => format!("{dir}{separator}{existing}"),
        None => dir.to_string(),
    }
}

fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

fn find_fyx(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("FYX_BIN") {
        return Ok(PathBuf::from(path));
    }
    let name = if cfg!(windows) { "fyx.exe" } else { "fyx" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .context("no Fyx binary: pass --fyx <path>, set FYX_BIN, or use --no-launch")
}

/// Modification times of the files a rebuild depends on: everything except build output.
fn source_snapshot(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut files = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            entry.depth() == 0 || !(name == "target" || name.starts_with('.'))
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy();
            path.extension().is_none_or(|extension| extension != "wasm")
                && !name.ends_with(".fyxext")
                && (name == MANIFEST_FILE
                    || name == "Cargo.toml"
                    || path.extension().is_some_and(|extension| extension == "rs")
                    || path
                        .components()
                        .any(|component| component.as_os_str() == "assets"))
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((entry.into_path(), modified))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_cdylib_of_the_built_crate() {
        let dir = tempfile::tempdir().expect("temp dir");
        let ours = dir.path().join("Cargo.toml");
        std::fs::write(&ours, "").expect("write");
        let ours_json = serde_json::to_string(&ours).expect("json");
        let messages = format!(
            r#"{{"reason":"compiler-artifact","manifest_path":"/elsewhere/Cargo.toml","target":{{"crate_types":["cdylib"]}},"filenames":["/elsewhere/dep.wasm"]}}
{{"reason":"compiler-artifact","manifest_path":{ours_json},"target":{{"crate_types":["rlib"]}},"filenames":["/x/libours.rlib"]}}
{{"reason":"compiler-artifact","manifest_path":{ours_json},"target":{{"crate_types":["cdylib"]}},"filenames":["/x/ours.wasm"]}}
{{"reason":"build-finished","success":true}}
not json"#
        );
        assert_eq!(
            find_component(&messages, &ours).expect("found"),
            PathBuf::from("/x/ours.wasm")
        );
        assert!(find_component("", &ours).is_err());
    }

    #[test]
    fn dev_extensions_prepends() {
        let separator = if cfg!(windows) { ";" } else { ":" };
        assert_eq!(dev_extensions_value("a", None), "a");
        assert_eq!(dev_extensions_value("a", Some(String::new())), "a");
        assert_eq!(
            dev_extensions_value("a", Some("b".into())),
            format!("a{separator}b")
        );
    }

    #[test]
    fn snapshot_tracks_sources_not_output() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::create_dir_all(root.join("target/debug")).expect("mkdir");
        for file in [
            "src/lib.rs",
            "Cargo.toml",
            "extension.toml",
            "fyx_ui.wasm",
            "target/debug/x.rs",
            "README.md",
        ] {
            std::fs::write(root.join(file), "x").expect("write");
        }
        let names = source_snapshot(root)
            .into_iter()
            .map(|(path, _)| {
                path.strip_prefix(root)
                    .expect("inside")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["Cargo.toml", "extension.toml", "src/lib.rs"]);
    }
}
