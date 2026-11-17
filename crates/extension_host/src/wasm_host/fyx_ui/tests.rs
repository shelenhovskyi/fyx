use std::{path::PathBuf, process::Command};

use gpui::TestAppContext;
use ui_ext::{Patch, Props, TreeNode, UiEvent, Update};

use super::*;

const SURFACE: &str = "counter";
const ROOT: ui_ext::NodeId = 1;
const LABEL: ui_ext::NodeId = 2;
const INCREMENT: ui_ext::NodeId = 3;
const RESET: ui_ext::NodeId = 4;
const LIST: ui_ext::NodeId = 5;
const FIRST_CLICK_NOTE: ui_ext::NodeId = 10;
const SPIN: ui_ext::NodeId = 99;

/// Builds `fixtures/counter` (a guest on `ui_ext_sdk`) for `wasm32-wasip2`, once per test process,
/// next to the test binary's own target directory.
fn counter_wasm() -> Vec<u8> {
    static WASM: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let path = WASM
        .get_or_init(|| {
            let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/wasm_host/fyx_ui/fixtures/counter");
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
                .args([
                    "build",
                    "--release",
                    "--locked",
                    "--target",
                    "wasm32-wasip2",
                ])
                .arg("--target-dir")
                .arg(&target_dir)
                .current_dir(&fixture_dir)
                .status()
                .map_err(|error| format!("failed to run cargo: {error}"))?;
            if !status.success() {
                return Err(format!("building {fixture_dir:?} failed: {status}"));
            }
            Ok(target_dir.join("wasm32-wasip2/release/fyx_ui_test_counter.wasm"))
        })
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

async fn load_counter(
    cx: &mut TestAppContext,
) -> (UiExtension, mpsc::UnboundedReceiver<PushedUpdate>) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let wasm = counter_wasm();
    cx.update(|cx| UiExtension::load("fyx-ui-test-counter".into(), wasm, &cx.to_async()))
        .await
        .expect("fixture loads")
}

fn text_of(tree: &Tree, id: ui_ext::NodeId) -> String {
    match tree.get(id).map(TreeNode::props) {
        Some(Props::Text(text)) => text.content.clone(),
        other => panic!("node {id} is not a text node: {other:?}"),
    }
}

#[gpui::test]
async fn test_counter_round_trip(cx: &mut TestAppContext) {
    let (extension, mut pushed) = load_counter(cx).await;

    assert_eq!(extension.api_version(), &Version::new(0, 1, 0));
    assert_eq!(
        extension.surfaces(),
        &[ui_ext::Surface {
            id: SURFACE.into(),
            title: "Counter".into(),
            kind: ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
                position: ui_ext::DockPosition::Left,
                icon: "hash".into(),
                default_size: 240,
            }),
        }]
    );

    let mut tree = extension.mount(SURFACE).await.expect("mount");
    assert_eq!(tree.roots(), &[ROOT]);
    assert_eq!(tree.children(Some(ROOT)), &[LABEL, INCREMENT, RESET, LIST]);
    assert_eq!(text_of(&tree, LABEL), "Count: 0");

    let update = extension
        .handle_event(SURFACE, UiEvent::Click(INCREMENT))
        .await
        .expect("increment");
    assert!(tree.apply_update(update).expect("valid patches"));
    assert_eq!(text_of(&tree, LABEL), "Count: 1");
    assert_eq!(
        tree.children(Some(ROOT)),
        &[LABEL, FIRST_CLICK_NOTE, INCREMENT, RESET, LIST]
    );

    let update = extension
        .handle_event(SURFACE, UiEvent::Click(INCREMENT))
        .await
        .expect("increment");
    tree.apply_update(update).expect("valid patches");
    assert_eq!(text_of(&tree, LABEL), "Count: 2");

    let update = extension
        .handle_event(SURFACE, UiEvent::Click(RESET))
        .await
        .expect("reset");
    tree.apply_update(update).expect("valid patches");
    assert_eq!(text_of(&tree, LABEL), "Count: 0");

    // Reset also pushed an update through the host import.
    let pushed_update = pushed.next().await.expect("a pushed update");
    assert_eq!(
        pushed_update,
        PushedUpdate {
            surface_id: SURFACE.into(),
            update: Update::Patches(vec![Patch::Remove(FIRST_CLICK_NOTE)]),
        }
    );
    tree.apply_update(pushed_update.update)
        .expect("valid patches");
    assert!(!tree.contains(FIRST_CLICK_NOTE));

    assert_eq!(
        extension
            .handle_event(SURFACE, UiEvent::InputChanged(LABEL, "x".into()))
            .await
            .expect("input"),
        Update::Unchanged
    );

    let rows = extension
        .list_rows(SURFACE, LIST, 1, 5)
        .await
        .expect("rows");
    assert_eq!(
        rows.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(text_of(&rows[0].1, 1001), "Row 1");

    // Errors the extension reports are not fatal.
    let error = extension
        .handle_event(SURFACE, UiEvent::Click(42))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("no handler for node 42"),
        "{error:#}"
    );
    let error = extension.mount("nope").await.unwrap_err();
    assert!(
        format!("{error:#}").contains("unknown surface nope"),
        "{error:#}"
    );
    let update = extension
        .handle_event(SURFACE, UiEvent::Click(INCREMENT))
        .await
        .expect("still works");
    tree.apply_update(update).expect("valid patches");
    assert_eq!(text_of(&tree, LABEL), "Count: 1");
}

#[gpui::test]
async fn test_runaway_call_is_interrupted_and_disables_extension(cx: &mut TestAppContext) {
    let (extension, _pushed) = load_counter(cx).await;

    let started = Instant::now();
    let error = extension
        .handle_event(SURFACE, UiEvent::Click(SPIN))
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= UI_CALL_BUDGET && elapsed < UI_CALL_BUDGET * 10,
        "interrupted after {elapsed:?}"
    );
    assert!(
        error
            .chain()
            .any(|cause| cause.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::Interrupt)),
        "{error:#}"
    );

    let error = extension
        .handle_event(SURFACE, UiEvent::Click(INCREMENT))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("disabled"), "{error:#}");
}

#[test]
fn test_api_version_gating() {
    assert!(is_supported_api_version(&Version::new(0, 1, 0)));
    assert!(!is_supported_api_version(&Version::new(0, 2, 0)));
    assert!(!is_supported_api_version(&Version::new(0, 0, 9)));

    assert_eq!(
        parse_api_version("counter", &counter_wasm()).expect("parses"),
        Some(Version::new(0, 1, 0))
    );
    let empty_core_module = b"\0asm\x01\0\0\0";
    assert_eq!(
        parse_api_version("empty", empty_core_module).expect("parses"),
        None
    );
}
