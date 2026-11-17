//! The renderer against real WASM components: the counter demo in an `ExtensionPanel` and the
//! status-item fixture in an `ExtensionStatusItem`, clicked through GPUI. This covers the
//! "click → handle-event → patch → repaint" loop without building and running the editor.

use std::{
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::OnceLock,
    time::Duration,
};

use extension_host::wasm_host::fyx_ui::UiExtension;
use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use ui_ext::{NodeId, Props, TextProps};

use crate::{ExtensionPanel, ExtensionStatusItem, SurfaceInfo, SurfaceState, backend::UiBackend};

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds a guest for `wasm32-wasip2` with `cargo build <args>` in `dir`, into
/// `<target>/fyx-ui-fixtures` (shared with extension_host's tests), and reads
/// `<that>/wasm32-wasip2/<output>`. Once per test process.
fn build_guest(
    built: &'static OnceLock<Result<PathBuf, String>>,
    dir: PathBuf,
    args: &'static [&'static str],
    output: &'static str,
) -> Vec<u8> {
    let path = built
        .get_or_init(|| {
            // <target>/debug/deps/<test binary>
            let target_dir = std::env::current_exe()
                .map_err(|error| error.to_string())?
                .ancestors()
                .nth(3)
                .ok_or("test binary is not inside a cargo target directory")?
                .join("fyx-ui-fixtures");
            // Blocking is fine: this runs once per test process, before any executor work.
            #[allow(clippy::disallowed_methods)]
            let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
                .arg("build")
                .args(args)
                .args(["--locked", "--target", "wasm32-wasip2", "--target-dir"])
                .arg(&target_dir)
                .current_dir(&dir)
                .status()
                .map_err(|error| format!("failed to run cargo: {error}"))?;
            if !status.success() {
                return Err(format!("cargo build {args:?} in {dir:?} failed: {status}"));
            }
            Ok(target_dir.join("wasm32-wasip2").join(output))
        })
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn counter_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    build_guest(
        &BUILT,
        workspace_dir(),
        &["-p", "fyx_demo_counter"],
        "debug/fyx_demo_counter.wasm",
    )
}

fn status_item_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    build_guest(
        &BUILT,
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/status_item"),
        &["--release"],
        "release/fyx_ui_test_status_item.wasm",
    )
}

async fn load(name: &str, wasm: Vec<u8>, cx: &mut TestAppContext) -> UiExtension {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let (extension, _pushed) = cx
        .update(|cx| UiExtension::load(name.into(), wasm, &cx.to_async()))
        .await
        .expect("component loads");
    extension
}

/// Extension calls run on tokio threads, outside the test scheduler, so run it until the
/// condition holds instead of only until it parks.
fn run_until(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    for _ in 0..500 {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn text_of(state: &Entity<SurfaceState>, id: NodeId, cx: &mut TestAppContext) -> Option<String> {
    state.read_with(cx, |state, _| match state.tree()?.get(id)?.props() {
        Props::Text(TextProps { content, .. }) => Some(content.clone()),
        _ => None,
    })
}

fn new_state(extension: &UiExtension, cx: &mut TestAppContext) -> Entity<SurfaceState> {
    let surface = extension.surfaces()[0].clone();
    let backend: Rc<dyn UiBackend> = Rc::new(extension.clone());
    let state = cx.new(|cx| SurfaceState::new(backend, surface, None, cx));
    run_until(cx, "mount", |cx| {
        state.read_with(cx, |state, _| state.tree().is_some())
    });
    state
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
}

#[gpui::test]
async fn test_counter_panel_increments(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let extension = load("fyx-demo-counter", counter_wasm(), cx).await;
    let state = new_state(&extension, cx);
    // Ids pinned by the counter's crate doc.
    const LABEL: NodeId = 2;
    const LIST: NodeId = 7;
    assert_eq!(text_of(&state, LABEL, cx).as_deref(), Some("Count: 0"));

    let info = Rc::new(SurfaceInfo::new(
        "fyx-demo-counter",
        &extension.surfaces()[0],
        100,
    ));
    let (_panel, vcx) = cx
        .add_window_view(|window, cx| ExtensionPanel::new(info.clone(), state.clone(), window, cx));

    click("fyx-node-5", vcx);
    run_until(vcx, "the first increment", |cx| {
        text_of(&state, LABEL, cx).as_deref() == Some("Count: 1")
    });
    click("fyx-node-5", vcx);
    run_until(vcx, "the second increment", |cx| {
        text_of(&state, LABEL, cx).as_deref() == Some("Count: 2")
    });

    // The list's row count changed with each click, so its visible rows were fetched again.
    run_until(vcx, "the rows", |cx| {
        state.read_with(cx, |state, _| {
            let row = |index| {
                let tree = state.row(LIST, index)?;
                let text = tree.get(tree.children(Some(tree.roots()[0]))[0])?;
                match text.props() {
                    Props::Text(TextProps { content, .. }) => Some(content.clone()),
                    _ => None,
                }
            };
            row(0).as_deref() == Some("Row 1 of 2") && row(1).as_deref() == Some("Row 2 of 2")
        })
    });
}

#[gpui::test]
async fn test_status_item_click_updates_label(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let extension = load("fyx-test-status-item", status_item_wasm(), cx).await;
    assert!(matches!(
        extension.surfaces()[0].kind,
        ui_ext::SurfaceKind::StatusItem(_)
    ));
    let state = new_state(&extension, cx);
    // Ids pinned by the fixture's crate doc.
    const LABEL: NodeId = 3;
    assert_eq!(text_of(&state, LABEL, cx).as_deref(), Some("Pings: 0"));

    let (_item, vcx) =
        cx.add_window_view(|window, cx| ExtensionStatusItem::new(state.clone(), window, cx));
    for expected in ["Pings: 1", "Pings: 2", "Pings: 3"] {
        click("fyx-node-4", vcx);
        run_until(vcx, expected, |cx| {
            text_of(&state, LABEL, cx).as_deref() == Some(expected)
        });
    }
}
