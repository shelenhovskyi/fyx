//! `fyx-ext check` on the `assets/` folder: the PNG and SVG files image nodes can show
//! (`docs/fyx/wit-0.3/design.md`, section 12). The renderer reads them when it draws, so a bad
//! file is not an update error but an image that shows its alt text; only the size cap is an
//! error, because nothing over 2 MiB is ever drawn.
//!
//! Which assets the component uses is in its code, so this checks every image file in the
//! folder; `fyx-ext test` reports the ones it actually shows (`Harness::image_failures`).

use std::path::Path;

use ui_ext::image::{self, ImageFormat};

use crate::Report;
use crate::package::ASSETS_DIR;

/// SVG parts the host drops when it draws an asset: no external or nested images, no scripts.
const DROPPED_SVG_PARTS: [&str; 3] = ["<image", "<script", "<foreignobject"];

pub fn check_assets(dir: &Path, report: &mut Report) {
    let assets = dir.join(ASSETS_DIR);
    if !assets.is_dir() {
        return;
    }
    for entry in walkdir::WalkDir::new(&assets).sort_by_file_name() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.error(format!("reading {}: {error}", assets.display()));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(path) = asset_path(dir, entry.path()) else {
            continue;
        };
        let lowercase = path.to_ascii_lowercase();
        if !lowercase.ends_with(".png") && !lowercase.ends_with(".svg") {
            continue;
        }
        let format = match image::check_asset_path(&path) {
            Ok(format) => format,
            Err(error) => {
                report.warning(format!("{error}: an image node cannot name it"));
                continue;
            }
        };
        let len = entry.metadata().map_or(0, |metadata| metadata.len());
        if len > image::MAX_ASSET_BYTES as u64 {
            report.error(format!(
                "{path} has {len} bytes; assets are at most {} bytes (2 MiB), and Fyx draws the \
                 alt text instead",
                image::MAX_ASSET_BYTES
            ));
            continue;
        }
        let bytes = match std::fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.error(format!("reading {path}: {error}"));
                continue;
            }
        };
        if let Err(error) = image::check_image_bytes(format, &bytes, image::MAX_ASSET_BYTES) {
            report.warning(format!("{path}: {error}; Fyx draws the alt text instead"));
            continue;
        }
        if format == ImageFormat::Svg {
            let text = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
            let dropped = DROPPED_SVG_PARTS
                .iter()
                .filter(|part| text.contains(*part))
                .map(|part| format!("{part}>"))
                .collect::<Vec<_>>();
            if !dropped.is_empty() {
                report.warning(format!(
                    "{path} contains {}; Fyx drops those parts when it draws the SVG",
                    dropped.join(", ")
                ));
            }
        }
    }
}

/// `assets/...` with `/` separators, as an image node names the file.
fn asset_path(dir: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(dir).ok()?;
    let parts = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_4X4: [u8; 33] = [
        0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0,
        0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0, 0, 0, 0, 0,
    ];

    fn check(files: &[(&str, &[u8])]) -> Report {
        let dir = tempfile::tempdir().expect("tempdir");
        for (path, bytes) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, bytes).expect("write");
        }
        let mut report = Report::default();
        check_assets(dir.path(), &mut report);
        report
    }

    #[test]
    fn valid_assets_and_other_files_pass() {
        let report = check(&[
            (
                "assets/logo.svg",
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            ),
            ("assets/icons/small.PNG", &PNG_4X4),
            ("assets/notes.txt", b"not an image"),
            ("README.md", b"# not in assets"),
        ]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn an_asset_over_2_mib_is_an_error() {
        let mut big = PNG_4X4.to_vec();
        big.resize(image::MAX_ASSET_BYTES + 1, 0);
        let report = check(&[("assets/big.png", &big)]);
        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(report.errors[0].contains("assets/big.png"));
    }

    #[test]
    fn bad_and_risky_files_are_warnings() {
        let report = check(&[
            ("assets/fake.png", b"GIF89a"),
            ("assets/huge.png", &{
                let mut png = PNG_4X4;
                png[16..20].copy_from_slice(&65535u32.to_be_bytes());
                png
            }),
            (
                "assets/script.svg",
                b"<svg><script>alert(1)</script><foreignObject/></svg>",
            ),
            ("assets/entities.svg", b"<!ENTITY a \"b\"><svg/>"),
        ]);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let warnings = report.warnings.join("\n");
        assert_eq!(report.warnings.len(), 4, "{warnings}");
        for path in ["fake.png", "huge.png", "script.svg", "entities.svg"] {
            assert!(warnings.contains(path), "{path}: {warnings}");
        }
        assert!(warnings.contains("<script>, <foreignobject>"), "{warnings}");
    }

    #[test]
    fn the_images_demo_passes() {
        let mut report = Report::default();
        check_assets(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/fyx_demo_images"),
            &mut report,
        );
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }
}
