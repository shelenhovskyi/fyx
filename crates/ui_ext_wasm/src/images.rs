//! Package assets of `image` nodes (fyx:ui 0.3, docs/fyx/wit-0.3/design.md section 12), checked
//! on disk. The props and byte checks are in [`ui_ext::image`]; generated images are
//! `images.add-image`, checked by the host import ([`crate::HostEffect::AddImage`]).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use ui_ext::image::{self, ImageInfo};

/// The file an `asset(path)` source names in the extension's installed version folder: the path
/// rule of [`image::check_asset_path`], then canonicalized and required to stay inside
/// `version_dir` (which also stops a link out of it), a regular file of at most
/// [`image::MAX_ASSET_BYTES`].
pub fn resolve_asset(version_dir: &Path, path: &str) -> Result<PathBuf> {
    image::check_asset_path(path)?;
    let root = std::fs::canonicalize(version_dir)
        .with_context(|| format!("resolving {}", version_dir.display()))?;
    let file = std::fs::canonicalize(root.join(path))
        .with_context(|| format!("image asset {path:?} not found"))?;
    if !file.starts_with(&root) {
        bail!("image asset {path:?} resolves outside the extension");
    }
    let metadata = std::fs::metadata(&file).with_context(|| format!("reading {path:?}"))?;
    if !metadata.is_file() {
        bail!("image asset {path:?} is not a file");
    }
    if metadata.len() > image::MAX_ASSET_BYTES as u64 {
        bail!(
            "image asset {path:?} has {} bytes, the cap is {}",
            metadata.len(),
            image::MAX_ASSET_BYTES
        );
    }
    Ok(file)
}

/// Reads an asset and runs the byte checks of [`image::check_image_bytes`] on it. Blocking: call
/// it off the main thread, from the decode queue.
pub fn read_asset(version_dir: &Path, path: &str) -> Result<(Vec<u8>, ImageInfo)> {
    let format = image::check_asset_path(path)?;
    let file = resolve_asset(version_dir, path)?;
    let bytes = std::fs::read(&file).with_context(|| format!("reading {path:?}"))?;
    let info = image::check_image_bytes(format, &bytes, image::MAX_ASSET_BYTES)
        .with_context(|| format!("image asset {path:?}"))?;
    Ok((bytes, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn test_assets_stay_in_the_version_folder() {
        let temp = tempfile::tempdir().expect("temp dir");
        let version = temp.path().join("acme.logo/1.0.0");
        std::fs::create_dir_all(version.join("assets/icons")).expect("dirs");
        std::fs::write(version.join("assets/logo.png"), png(64, 64)).expect("write");
        std::fs::write(version.join("assets/icons/big.png"), png(4096, 16)).expect("write");
        std::fs::write(
            version.join("assets/huge.svg"),
            vec![b' '; image::MAX_ASSET_BYTES + 1],
        )
        .expect("write");
        std::fs::create_dir_all(version.join("assets/dir.svg")).expect("dir");
        std::fs::write(temp.path().join("acme.logo/secret.png"), png(1, 1)).expect("write");

        let (_, info) = read_asset(&version, "assets/logo.png").expect("reads");
        assert_eq!(info.size, Some((64, 64)));

        for (path, message) in [
            ("assets/../../secret.png", "'..'"),
            ("../secret.png", "assets/"),
            ("assets/missing.png", "not found"),
            ("assets/dir.svg", "not a file"),
            ("assets/huge.svg", "the cap is"),
            ("assets/icons/big.png", "4096 x 16"),
        ] {
            let error = read_asset(&version, path).expect_err(path);
            assert!(format!("{error:#}").contains(message), "{path}: {error:#}");
        }
    }

    /// A link inside `assets/` that points out of the version folder is refused.
    #[cfg(unix)]
    #[test]
    fn test_asset_links_out_are_refused() {
        let temp = tempfile::tempdir().expect("temp dir");
        let version = temp.path().join("v");
        std::fs::create_dir_all(version.join("assets")).expect("dirs");
        std::fs::write(temp.path().join("outside.png"), png(1, 1)).expect("write");
        std::os::unix::fs::symlink(
            temp.path().join("outside.png"),
            version.join("assets/x.png"),
        )
        .expect("link");
        let error = resolve_asset(&version, "assets/x.png").unwrap_err();
        assert!(format!("{error:#}").contains("outside"), "{error:#}");
    }
}
