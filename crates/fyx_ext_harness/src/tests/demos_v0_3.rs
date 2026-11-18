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

/// `extensions/<crate_name>` as `fyx-build-demos --fyx-ui-0-3` lays it out: the manifest, the
/// assets and the 0.3 component, in a folder of the test target directory. The 0.3 builds of
/// the demos share one target, so they are built together.
fn demo_package(crate_name: &str) -> PathBuf {
    const DEMOS: [&str; 2] = ["fyx_demo_images", "fyx_demo_charts"];
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    let wasm_dir = BUILT
        .get_or_init(|| {
            let features = DEMOS.map(|demo| format!("{demo}/fyx-ui-0-3")).join(",");
            let mut args = vec!["--features", &features];
            for demo in DEMOS {
                args.extend(["-p", demo]);
            }
            let wasm = build_guest(
                &workspace_dir(),
                "fyx-ui-fixtures-v0_3",
                &args,
                "debug/fyx_demo_images.wasm",
            )?;
            Ok(wasm.parent().ok_or("no output directory")?.to_path_buf())
        })
        .clone()
        .unwrap_or_else(|error| panic!("{error}"));
    let wasm = wasm_dir.join(format!("{crate_name}.wasm"));
    let source = workspace_dir().join("extensions").join(crate_name);
    let package = wasm_dir
        .ancestors()
        .nth(2)
        .expect("a target directory")
        .join(format!("{crate_name}-{}", std::process::id()));
    if package.exists() {
        std::fs::remove_dir_all(&package).expect("clean");
    }
    if source.join("assets").exists() {
        copy_dir(&source.join("assets"), &package.join("assets")).expect("assets");
    }
    std::fs::create_dir_all(&package).expect("package dir");
    for (from, to) in [
        (
            source.join("extension.toml"),
            package.join("extension.toml"),
        ),
        (wasm, package.join("fyx_ui.wasm")),
    ] {
        std::fs::copy(&from, &to).unwrap_or_else(|error| panic!("copying {from:?}: {error}"));
    }
    package
}

fn images_package() -> PathBuf {
    static PACKAGE: OnceLock<PathBuf> = OnceLock::new();
    PACKAGE
        .get_or_init(|| demo_package("fyx_demo_images"))
        .clone()
}

fn mounted() -> Harness {
    let mut harness = Harness::load_package(images_package()).expect("load");
    harness.mount(PANEL).expect("mount");
    harness
}

fn images(harness: &Harness) -> Vec<ImageProps> {
    harness
        .nodes(PANEL)
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
        .find_v0_1(PANEL, |props| matches!(props, Props::List(_)))
        .expect("list");
    let rows = harness.rows(PANEL, list, 0, 10).expect("rows");
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
    let outcome = harness.click(PANEL, "Shuffle").expect("shuffle");
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

mod charts {
    use super::*;

    const PANEL: &str = "charts";

    fn mounted() -> Harness {
        static PACKAGE: OnceLock<PathBuf> = OnceLock::new();
        let package = PACKAGE.get_or_init(|| demo_package("fyx_demo_charts"));
        let mut harness = Harness::load_package(package).expect("load");
        harness.mount(PANEL).expect("mount");
        harness
    }

    fn canvases(harness: &Harness) -> Vec<NodeId> {
        harness
            .nodes(PANEL)
            .unwrap_or_default()
            .into_iter()
            .filter(|node| matches!(node.props, model::Props::Canvas(_)))
            .map(|node| node.id)
            .collect()
    }

    #[test]
    fn a_click_on_a_bar_selects_it() {
        let mut harness = mounted();
        let [chart, spark] = canvases(&harness)[..] else {
            panic!("two canvases");
        };
        assert_eq!(harness.canvas(PANEL, chart).expect("chart").ops.len(), 15);
        // Wednesday's bar is the tallest: x 80 to 120, the bar starts at the top.
        harness
            .canvas_click(PANEL, chart, 100.0, 60.0)
            .expect("click");
        assert!(harness.find_text(PANEL, "Wed: 42 commits").is_some());
        let selected = harness.canvas(PANEL, chart).expect("chart");
        assert!(selected.ops.iter().any(|op| matches!(
            op,
            ui_ext::canvas::DrawOp::Rect {
                stroke: Some(_),
                ..
            }
        )));

        harness
            .canvas_click(PANEL, chart, 100.0, 130.0)
            .expect("click the labels");
        assert!(harness.find_text(PANEL, "Click a bar").is_some());

        let outside = harness.canvas_click(PANEL, chart, 300.0, 10.0).unwrap_err();
        assert!(format!("{outside}").contains("outside"), "{outside}");
        let not_clickable = harness.canvas_click(PANEL, spark, 10.0, 10.0).unwrap_err();
        assert!(
            format!("{not_clickable}").contains("not clickable"),
            "{not_clickable}"
        );
    }

    #[test]
    fn the_sparkline_ticks_until_paused() {
        let mut harness = mounted();
        let spark = canvases(&harness)[1];
        assert_eq!(
            harness.pending_tick(PANEL),
            Some(std::time::Duration::from_millis(500))
        );
        for _ in 0..3 {
            harness.tick(PANEL).expect("tick");
        }
        let ops = &harness.canvas(PANEL, spark).expect("spark").ops;
        assert!(
            matches!(&ops[..], [_, ui_ext::canvas::DrawOp::Path { segments, .. }] if segments.len() == 3),
            "{ops:?}"
        );
        harness.click(PANEL, "Pause").expect("pause");
        assert_eq!(harness.pending_tick(PANEL), None);
        assert!(harness.find_button(PANEL, "Resume").is_some());
        assert!(harness.tick(PANEL).is_err(), "no tick is pending");
        harness.click(PANEL, "Resume").expect("resume");
        assert!(harness.pending_tick(PANEL).is_some());
    }
}
