//! `fyx-ext package`: `<id>-<version>.fyxext`, a zip of `extension.toml`, the component and an
//! optional `assets/` directory (platform spec, decision 12). `signature.json` is added by the
//! registry's CI, never by the author.
//!
//! Nothing from outside the extension folder is ever packed (triage-3 #6). Every file is
//! resolved, links included, and must lie inside the resolved extension folder.
//! - A link inside `assets/` whose target is inside the folder is packed under the link's name,
//!   with the target's contents.
//! - A link that points outside the folder, a broken link and a link loop stop the packaging.
//! - Names in the zip always use `/`. A file name that contains `\`, which only Unix allows,
//!   stops it too, since a Windows unzip would read the `\` as a separator.
//! - A file reached twice (a component inside `assets/`) is packed once.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result};
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use futures::{AsyncWrite, AsyncWriteExt as _};

use crate::manifest::{MANIFEST_FILE, Manifest};

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

    let entries = collect_entries(dir, &manifest)?;
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {out_dir:?}"))?;
    let destination = out_dir.join(manifest.package_file_name());
    if let Err(error) = smol::block_on(write_zip(&destination, &entries)) {
        std::fs::remove_file(&destination).ok();
        return Err(error);
    }

    let size = std::fs::metadata(&destination)?.len();
    eprintln!("fyx-ext: packaged {} ({size} bytes)", destination.display());
    Ok(destination)
}

/// The archive name and the resolved file of everything to pack, in packing order.
fn collect_entries(dir: &Path, manifest: &Manifest) -> Result<Vec<(String, PathBuf)>> {
    let root = std::fs::canonicalize(dir).with_context(|| format!("resolving {dir:?}"))?;
    let mut entries = Entries {
        root,
        by_name: BTreeMap::new(),
        order: Vec::new(),
    };
    entries.add(MANIFEST_FILE.to_string(), &dir.join(MANIFEST_FILE))?;
    let component_name = manifest.component_archive_name().with_context(|| {
        format!(
            "[fyx_ui].component {:?} is not a relative path inside the extension",
            manifest.component
        )
    })?;
    entries.add(component_name, &manifest.component_path())?;

    let assets = dir.join(ASSETS_DIR);
    if std::fs::symlink_metadata(&assets).is_ok() {
        // Links are followed so that they can be resolved and checked; walkdir reports a loop
        // and a broken link as errors.
        for entry in walkdir::WalkDir::new(&assets)
            .follow_links(true)
            .sort_by_file_name()
        {
            let entry =
                entry.with_context(|| format!("reading {assets:?} (a broken link or a loop?)"))?;
            if entry.file_type().is_dir() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(dir)
                .context("an asset path outside the extension")?;
            let mut parts = Vec::new();
            for part in relative.components() {
                let part = part
                    .as_os_str()
                    .to_str()
                    .with_context(|| format!("{relative:?}: a file name that is not UTF-8"))?;
                anyhow::ensure!(
                    !part.contains('\\'),
                    "{relative:?}: a file name with a backslash, which Windows reads as a folder \
                     separator"
                );
                parts.push(part);
            }
            anyhow::ensure!(
                entry.file_type().is_file(),
                "{relative:?} is not a regular file"
            );
            entries.add(parts.join("/"), entry.path())?;
        }
    }
    Ok(entries.order)
}

struct Entries {
    root: PathBuf,
    by_name: BTreeMap<String, PathBuf>,
    order: Vec<(String, PathBuf)>,
}

impl Entries {
    /// Adds `path` under `name` after resolving it inside the extension. The same file under the
    /// same name is added once; a different file under a taken name is an error.
    fn add(&mut self, name: String, path: &Path) -> Result<()> {
        let resolved =
            std::fs::canonicalize(path).with_context(|| format!("resolving {path:?}"))?;
        anyhow::ensure!(
            resolved.starts_with(&self.root),
            "{name} resolves to {resolved:?}, outside the extension folder {:?}; a package never \
             contains files from outside it",
            self.root
        );
        anyhow::ensure!(resolved.is_file(), "{name} is not a regular file");
        match self.by_name.get(&name) {
            Some(existing) if *existing == resolved => Ok(()),
            Some(existing) => {
                anyhow::bail!("{name} would be packed twice, from {existing:?} and {resolved:?}")
            }
            None => {
                self.by_name.insert(name.clone(), resolved.clone());
                self.order.push((name, resolved));
                Ok(())
            }
        }
    }
}

/// Counts the bytes written through it, so the package cap stops a growing archive early.
struct Counted<W> {
    inner: W,
    written: Rc<Cell<u64>>,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for Counted<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let poll = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = &poll {
            self.written.set(self.written.get() + *written as u64);
        }
        poll
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

fn check_size(written: u64, cap: u64) -> Result<()> {
    anyhow::ensure!(
        written <= cap,
        "the package is over {cap} bytes, the cap (50 MiB for a package)"
    );
    Ok(())
}

async fn write_zip(destination: &Path, entries: &[(String, PathBuf)]) -> Result<()> {
    write_zip_capped(destination, entries, MAX_PACKAGE_BYTES).await
}

async fn write_zip_capped(
    destination: &Path,
    entries: &[(String, PathBuf)],
    cap: u64,
) -> Result<()> {
    let file = smol::fs::File::create(destination)
        .await
        .with_context(|| format!("creating {destination:?}"))?;
    let written = Rc::new(Cell::new(0));
    let mut writer = ZipFileWriter::new(Counted {
        inner: file,
        written: written.clone(),
    });
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
        check_size(written.get(), cap)?;
    }
    let mut counted = writer.close().await.context("finishing the zip")?;
    counted.flush().await?;
    check_size(written.get(), cap)
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

    /// An extension in `root` whose `[fyx_ui].component` is `component` (TOML-escaped), with the
    /// component written at `component_file`.
    fn extension(root: &Path, component: &str, component_file: &str) {
        let component = component.replace('\\', "\\\\");
        std::fs::write(
            root.join(MANIFEST_FILE),
            format!(
                "id = \"my-panel\"\nname = \"P\"\nversion = \"1.2.3\"\nschema_version = 1\n\n[fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"{component}\"\ncapabilities = [\"ui:panel\"]\n"
            ),
        )
        .expect("manifest");
        let path = root.join(component_file);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            path,
            with_custom_section(component_v0_1(), API_VERSION_SECTION, &[0, 0, 0, 1, 0, 0]),
        )
        .expect("component");
    }

    /// Creates a link, or returns `false` where the OS does not allow it (Windows without
    /// Developer Mode), so the test can skip.
    fn link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match made {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipped: cannot create a link here: {error}");
                false
            }
        }
    }

    fn package_error(root: &Path) -> String {
        let error =
            package(root, &root.join("dist"), crate::CheckDepth::Static).expect_err("refused");
        format!("{error:#}")
    }

    #[test]
    fn an_assets_link_out_of_the_extension_is_refused() {
        let outside = tempfile::tempdir().expect("temp dir");
        std::fs::write(outside.path().join("secret.txt"), "private").expect("secret");
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        extension(root, "fyx_ui.wasm", "fyx_ui.wasm");
        if !link(outside.path(), &root.join(ASSETS_DIR)) {
            return;
        }
        let error = package_error(root);
        assert!(error.contains("outside the extension folder"), "{error}");
        assert!(!root.join("dist/my-panel-1.2.3.fyxext").exists());
    }

    #[test]
    fn a_file_link_out_of_the_extension_is_refused() {
        let outside = tempfile::tempdir().expect("temp dir");
        std::fs::write(outside.path().join("secret.txt"), "private").expect("secret");
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        extension(root, "fyx_ui.wasm", "fyx_ui.wasm");
        std::fs::create_dir_all(root.join("assets")).expect("mkdir");
        if !link(
            &outside.path().join("secret.txt"),
            &root.join("assets/logo.svg"),
        ) {
            return;
        }
        let error = package_error(root);
        assert!(error.contains("outside the extension folder"), "{error}");
    }

    #[test]
    fn a_link_inside_the_extension_is_packed_under_its_own_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        extension(root, "fyx_ui.wasm", "fyx_ui.wasm");
        std::fs::create_dir_all(root.join("assets")).expect("mkdir");
        std::fs::create_dir_all(root.join("shared")).expect("mkdir");
        std::fs::write(root.join("shared/logo.svg"), "<svg/>").expect("logo");
        if !link(&root.join("shared/logo.svg"), &root.join("assets/logo.svg")) {
            return;
        }
        let package_path =
            package(root, &root.join("dist"), crate::CheckDepth::Static).expect("package");
        assert_eq!(
            entry_names(&package_path),
            ["extension.toml", "fyx_ui.wasm", "assets/logo.svg"],
            "the target's contents under the link's name; shared/ itself is not packed"
        );
    }

    #[test]
    fn a_broken_link_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        extension(root, "fyx_ui.wasm", "fyx_ui.wasm");
        std::fs::create_dir_all(root.join("assets")).expect("mkdir");
        if !link(&root.join("gone.svg"), &root.join("assets/logo.svg")) {
            return;
        }
        let error = package_error(root);
        assert!(error.contains("broken link"), "{error}");
    }

    #[test]
    fn a_backslashed_component_means_the_same_on_every_os() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        extension(root, r"bin\fyx_ui.wasm", "bin/fyx_ui.wasm");
        let package_path =
            package(root, &root.join("dist"), crate::CheckDepth::Static).expect("package");
        assert_eq!(
            entry_names(&package_path),
            ["extension.toml", "bin/fyx_ui.wasm"]
        );

        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("ext");
        std::fs::create_dir_all(&root).expect("mkdir");
        extension(&root, r"..\fyx_ui.wasm", "fyx_ui.wasm");
        std::fs::write(dir.path().join("fyx_ui.wasm"), "outside").expect("outside");
        let error = package_error(&root);
        assert!(error.contains("fails `fyx-ext check`"), "{error}");
    }

    #[test]
    fn a_component_inside_assets_is_packed_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        extension(root, "assets/fyx_ui.wasm", "assets/fyx_ui.wasm");
        std::fs::write(root.join("assets/a.svg"), "<svg/>").expect("asset");
        let package_path =
            package(root, &root.join("dist"), crate::CheckDepth::Static).expect("package");
        assert_eq!(
            entry_names(&package_path),
            ["extension.toml", "assets/fyx_ui.wasm", "assets/a.svg"]
        );
    }

    #[test]
    fn the_size_cap_stops_a_growing_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let big = dir.path().join("big.bin");
        // Bytes that do not compress: a simple xorshift stream.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let bytes = (0..64 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect::<Vec<_>>();
        std::fs::write(&big, bytes).expect("big");
        let destination = dir.path().join("out.fyxext");
        let entries = vec![("assets/big.bin".to_string(), big)];
        let error = smol::block_on(write_zip_capped(&destination, &entries, 16 * 1024))
            .expect_err("over the cap");
        assert!(
            format!("{error:#}").contains("over 16384 bytes"),
            "{error:#}"
        );
    }
}
