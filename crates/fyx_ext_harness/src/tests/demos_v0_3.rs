//! The images demo's `fyx-ui-0-3` build, loaded as a package (plan 4.12): asset and generated
//! images, and the decisions of `tasks/progress/W4-4.12-prep.md` (D1 to D4).

use std::sync::OnceLock;

use ui_ext::image::{ImageFit, ImageProps, ImageSource};

use super::*;

const PANEL: &str = "images";
const MISSING: &str = "assets/missing.png";

fn copy_dir(from: &Path, to: &Path) -> std::result::Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| format!("creating {to:?}: {error}"))?;
    let entries = std::fs::read_dir(from).map_err(|error| format!("reading {from:?}: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|error| format!("copying {:?}: {error}", entry.path()))?;
        }
    }
    Ok(())
}

/// `extensions/fyx_demo_images` as `fyx-build-demos --fyx-ui-0-3` lays it out: the manifest, the
/// assets and the 0.3 component, in a folder of the test target directory.
fn images_package() -> PathBuf {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let wasm = build_guest(
                &workspace_dir(),
                "fyx-ui-fixtures-v0_3",
                &[
                    "-p",
                    "fyx_demo_images",
                    "--features",
                    "fyx_demo_images/fyx-ui-0-3",
                ],
                "debug/fyx_demo_images.wasm",
            )?;
            let source = workspace_dir().join("extensions/fyx_demo_images");
            let package = wasm
                .ancestors()
                .nth(3)
                .ok_or("no target directory")?
                .join("fyx_demo_images");
            if package.exists() {
                std::fs::remove_dir_all(&package).map_err(|error| error.to_string())?;
            }
            copy_dir(&source.join("assets"), &package.join("assets"))?;
            for (from, to) in [
                (
                    source.join("extension.toml"),
                    package.join("extension.toml"),
                ),
                (wasm, package.join("fyx_ui.wasm")),
            ] {
                std::fs::copy(&from, &to).map_err(|error| format!("copying {from:?}: {error}"))?;
            }
            Ok(package)
        })
        .clone()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn mounted() -> Harness {
    let mut harness = Harness::load_package(images_package()).expect("load");
    harness.mount(PANEL).expect("mount");
    harness
}

fn images(harness: &Harness) -> Vec<ImageProps> {
    harness
        .nodes_v0_2(PANEL)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|node| match node.props {
            model::Props::Image(image) => Some(image),
            _ => None,
        })
        .collect()
}

fn generated(image: &ImageProps) -> Option<u32> {
    match image.source {
        ImageSource::Generated(handle) => Some(handle),
        ImageSource::Asset(_) => None,
    }
}

#[test]
fn the_images_demo_draws_assets_and_a_generated_chart() {
    let mut harness = mounted();
    assert_eq!(harness.api_version(), &Version::new(0, 3, 0));
    let shown = images(&harness);
    assert_eq!(shown.len(), 6, "{shown:#?}");
    assert!(
        shown
            .iter()
            .any(|image| image.fit == ImageFit::Cover && image.tint.is_none())
    );
    assert_eq!(shown.iter().filter_map(generated).collect::<Vec<_>>(), [1]);
    assert!(matches!(
        harness.effects(),
        [HostEffect::AddImage {
            handle: 1,
            format: ui_ext::image::ImageFormat::Svg,
            ..
        }]
    ));
    // D1: the missing asset is the only one the renderer draws as alt text, and it is not an
    // update error.
    assert_eq!(
        harness.image_failures().len(),
        1,
        "{:?}",
        harness.image_failures()
    );
    assert!(harness.image_failures()[0].contains(MISSING));
    assert!(
        harness
            .logs()
            .iter()
            .any(|(level, line)| { *level == ui_ext::LogLevel::Warn && line.contains(MISSING) })
    );

    let list = harness
        .find(PANEL, |props| matches!(props, Props::List(_)))
        .expect("list");
    let rows = harness.rows_v0_2(PANEL, list, 0, 10).expect("rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        harness.image_failures().len(),
        1,
        "the thumbnails are valid SVGs"
    );
}

#[test]
fn shuffle_swaps_the_generated_image() {
    let mut harness = mounted();
    let outcome = harness.click_v0_2(PANEL, "Shuffle").expect("shuffle");
    assert!(
        matches!(
            &outcome.effects[..],
            [
                HostEffect::AddImage { handle: 2, .. },
                HostEffect::RemoveImage { handle: 1 }
            ]
        ),
        "{:?}",
        outcome.effects
    );
    assert_eq!(
        images(&harness)
            .iter()
            .filter_map(generated)
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        harness.image_failures().len(),
        1,
        "the reply never shows the freed handle: {:?}",
        harness.image_failures()
    );
}

#[test]
fn a_bad_asset_is_alt_text_not_an_error() {
    // D1: an SVG path whose bytes are a PNG fails at decode time only.
    let package = images_package();
    let broken = package.with_file_name("fyx_demo_images_broken");
    if broken.exists() {
        std::fs::remove_dir_all(&broken).expect("clean");
    }
    copy_dir(&package, &broken).expect("copy");
    std::fs::copy(
        broken.join("assets/sample.png"),
        broken.join("assets/logo.svg"),
    )
    .expect("replace the logo");
    let mut harness = Harness::load_package(&broken).expect("load");
    harness
        .mount(PANEL)
        .expect("a bad asset does not fail the mount");
    let failures = harness.image_failures();
    assert_eq!(
        failures
            .iter()
            .filter(|failure| failure.contains("assets/logo.svg"))
            .count(),
        2,
        "the plain and the tinted logo: {failures:?}"
    );
}

#[test]
fn without_a_package_assets_are_reported_unchecked() {
    let wasm = std::fs::read(images_package().join("fyx_ui.wasm")).expect("wasm");
    let mut harness = Harness::load(&wasm).expect("load");
    harness.mount(PANEL).expect("mount");
    assert!(
        harness
            .image_failures()
            .iter()
            .all(|failure| failure.contains("load_package")),
        "{:?}",
        harness.image_failures()
    );
}

#[test]
fn tint_on_a_png_is_ignored_with_a_warning() {
    // D4.
    let harness = mounted();
    let mut image = ImageProps {
        source: ImageSource::Asset("assets/sample.png".into()),
        alt: "sample".into(),
        width: None,
        height: None,
        fit: ImageFit::Contain,
        tint: Some(ui_ext::ColorToken::TextAccent),
    };
    let failure = harness
        .check_asset("assets/sample.png", &image)
        .expect_err("tint on a PNG");
    assert!(failure.contains("tint is ignored"), "{failure}");
    image.tint = None;
    harness
        .check_asset("assets/sample.png", &image)
        .expect("an untinted PNG");
    assert!(
        harness
            .check_asset("assets/../extension.toml", &image)
            .is_err()
    );
}

#[test]
fn generated_handles_follow_d2_and_d3() {
    let mut harness = mounted();
    let image = |handle| {
        model::Props::Image(ImageProps {
            source: ImageSource::Generated(handle),
            alt: "chart".into(),
            width: None,
            height: None,
            fit: ImageFit::Contain,
            tint: None,
        })
    };
    let api = &harness.store.data().api;
    // D3: a handle add-image has not returned yet is a reference error.
    let error = api
        .check_images(PANEL, [&image(2)])
        .expect_err("a future handle");
    assert_eq!(error.code, model::HostErrorCode::InvalidArgument);
    api.check_images(PANEL, [&image(1)])
        .expect("the live chart");

    // D2: a handle freed since is fine for the update; the renderer draws the alt text.
    harness.store.data_mut().api.remove_image(1);
    harness
        .store
        .data()
        .api
        .check_images(PANEL, [&image(1)])
        .expect("a dead handle is not an update error");
    let before = harness.image_failures().len();
    harness.inspect_images([&image(1)]);
    assert_eq!(harness.image_failures().len(), before + 1);
    assert!(
        harness.image_failures()[before].contains("not live"),
        "{:?}",
        harness.image_failures()
    );
}
