//! `fyx-ext package`: `<id>-<version>.fyxext`, a zip of `extension.toml`, the component and an
//! optional `assets/` directory (platform spec, decision 12). `signature.json` is added by the
//! registry's CI, never by the author.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::AsyncWriteExt as _;

use crate::manifest::MANIFEST_FILE;

pub const MAX_PACKAGE_BYTES: u64 = 50 * 1024 * 1024;
pub const ASSETS_DIR: &str = "assets";

/// Packs the extension in `dir` into `out_dir`, after `check` passes. Returns the package path.
pub fn package(dir: &Path, out_dir: &Path, depth: crate::CheckDepth) -> Result<PathBuf> {
    let (manifest, report) = crate::check(dir, depth)?;
    report.print();
    anyhow::ensure!(
        !report.has_errors(),
        "not packaging an extension that fails `fyx-ext check`"
    );

    let mut entries = vec![
        (MANIFEST_FILE.to_string(), dir.join(MANIFEST_FILE)),
        (
            manifest.component.replace('\\', "/"),
            manifest.component_path(),
        ),
    ];
    let assets = dir.join(ASSETS_DIR);
    if assets.is_dir() {
        for entry in walkdir::WalkDir::new(&assets).sort_by_file_name() {
            let entry = entry.with_context(|| format!("reading {assets:?}"))?;
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(dir)
                    .context("asset outside the extension")?;
                let name = relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                entries.push((name, entry.into_path()));
            }
        }
    }

    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {out_dir:?}"))?;
    let destination = out_dir.join(manifest.package_file_name());
    smol::block_on(write_zip(&destination, &entries))?;

    let size = std::fs::metadata(&destination)?.len();
    if size > MAX_PACKAGE_BYTES {
        std::fs::remove_file(&destination).ok();
        anyhow::bail!("the package is {size} bytes; the cap is {MAX_PACKAGE_BYTES} (50 MiB)");
    }
    eprintln!("fyx-ext: packaged {} ({size} bytes)", destination.display());
    Ok(destination)
}

async fn write_zip(destination: &Path, entries: &[(String, PathBuf)]) -> Result<()> {
    let mut file = smol::fs::File::create(destination)
        .await
        .with_context(|| format!("creating {destination:?}"))?;
    let mut writer = ZipFileWriter::new(&mut file);
    for (name, path) in entries {
        let data = smol::fs::read(path)
            .await
            .with_context(|| format!("reading {path:?}"))?;
        writer
            .write_entry_whole(
                ZipEntryBuilder::new(name.clone().into(), Compression::Deflate),
                &data,
            )
            .await
            .with_context(|| format!("adding {name}"))?;
    }
    writer.close().await.context("finishing the zip")?;
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use async_zip::base::read::seek::ZipFileReader;
    use futures::io::BufReader;

    use super::*;
    use crate::inspect::API_VERSION_SECTION;
    use crate::inspect::tests::{component_v0_1, with_custom_section};

    fn entry_names(path: &Path) -> Vec<String> {
        smol::block_on(async {
            let file = smol::fs::File::open(path).await.expect("open");
            let reader = ZipFileReader::new(BufReader::new(file))
                .await
                .expect("read zip");
            reader
                .file()
                .entries()
                .iter()
                .map(|entry| entry.filename().as_str().expect("utf-8").to_string())
                .collect()
        })
    }

    #[test]
    fn packages_manifest_component_and_assets() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::write(
            root.join(MANIFEST_FILE),
            "id = \"my-panel\"\nname = \"P\"\nversion = \"1.2.3\"\nschema_version = 1\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"bin/fyx_ui.wasm\"\ncapabilities = [\"ui:panel\"]\n",
        )
        .expect("manifest");
        std::fs::create_dir_all(root.join("bin")).expect("mkdir");
        std::fs::create_dir_all(root.join("assets/icons")).expect("mkdir");
        std::fs::write(
            root.join("bin/fyx_ui.wasm"),
            with_custom_section(component_v0_1(), API_VERSION_SECTION, &[0, 0, 0, 1, 0, 0]),
        )
        .expect("component");
        std::fs::write(root.join("assets/icons/a.svg"), "<svg/>").expect("asset");
        std::fs::write(root.join("README.md"), "not packed").expect("readme");

        let out = root.join("dist");
        let package_path = package(root, &out, crate::CheckDepth::Static).expect("package");
        assert_eq!(package_path, out.join("my-panel-1.2.3.fyxext"));
        assert_eq!(
            entry_names(&package_path),
            ["extension.toml", "bin/fyx_ui.wasm", "assets/icons/a.svg"]
        );
    }

    #[test]
    fn refuses_an_extension_that_fails_check() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            "id = \"x\"\nversion = \"0.1.0\"\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"missing.wasm\"\n",
        )
        .expect("manifest");
        let error = package(
            dir.path(),
            &dir.path().join("dist"),
            crate::CheckDepth::Static,
        )
        .expect_err("no component");
        assert!(
            error.to_string().contains("fails `fyx-ext check`"),
            "{error}"
        );
        assert!(!dir.path().join("dist/x-0.1.0.fyxext").exists());
    }
}
