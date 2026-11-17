use std::{
    path::{Path, PathBuf},
    process::Command,
};

use gpui::TestAppContext;
use ui_ext::{Patch, Props, TreeNode, UiEvent, Update};

use super::*;

/// Node ids of `extensions/fyx_demo_counter`, as its crate doc pins them.
mod counter {
    use ui_ext::NodeId;

    pub const SURFACE: &str = "counter";
    pub const ROOT: NodeId = 1;
    pub const LABEL: NodeId = 2;
    pub const BUTTONS: NodeId = 3;
    pub const DECREMENT: NodeId = 4;
    pub const INCREMENT: NodeId = 5;
    pub const RESET: NodeId = 6;
    pub const LIST: NodeId = 7;
    pub const ROW_ROOT: NodeId = 0x8000_0000;
    pub const ROW_TEXT: NodeId = 0x8000_0001;
}

/// Node ids of `fixtures/probe`.
mod probe {
    use ui_ext::NodeId;

    pub const SURFACE: &str = "probe";
    pub const PUSH: NodeId = 1;
    pub const SPIN: NodeId = 2;
}

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds a guest for `wasm32-wasip2` with `cargo build <args>` in `dir`, into a target directory
/// next to the test binary's own, and returns `<target>/wasm32-wasip2/<output>`.
fn build_guest(dir: &Path, args: &[&str], output: &str) -> Result<PathBuf, String> {
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
        .current_dir(dir)
        .status()
        .map_err(|error| format!("failed to run cargo: {error}"))?;
    if !status.success() {
        return Err(format!("cargo build {args:?} in {dir:?} failed: {status}"));
    }
    Ok(target_dir.join("wasm32-wasip2").join(output))
}

fn read_guest(
    built: &'static OnceLock<Result<PathBuf, String>>,
    build: fn() -> Result<PathBuf, String>,
) -> Vec<u8> {
    let path = built
        .get_or_init(build)
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn demo_counter_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    read_guest(&BUILT, || {
        build_guest(
            &workspace_dir(),
            &["-p", "fyx_demo_counter"],
            "debug/fyx_demo_counter.wasm",
        )
    })
}

fn probe_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    read_guest(&BUILT, || {
        build_guest(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wasm_host/fyx_ui/fixtures/probe"),
            &["--release"],
            "release/fyx_ui_test_probe.wasm",
        )
    })
}

async fn load(
    name: &str,
    wasm: Vec<u8>,
    cx: &mut TestAppContext,
) -> (UiExtension, mpsc::UnboundedReceiver<PushedUpdate>) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    cx.update(|cx| UiExtension::load(name.into(), wasm, &cx.to_async()))
        .await
        .unwrap_or_else(|error| panic!("{name} failed to load: {error:#}"))
}

fn text_of(tree: &Tree, id: ui_ext::NodeId) -> String {
    match tree.get(id).map(TreeNode::props) {
        Some(Props::Text(text)) => text.content.clone(),
        other => panic!("node {id} is not a text node: {other:?}"),
    }
}

fn is_disabled(tree: &Tree, id: ui_ext::NodeId) -> bool {
    match tree.get(id).map(TreeNode::props) {
        Some(Props::Button(button)) => button.disabled,
        other => panic!("node {id} is not a button: {other:?}"),
    }
}

fn row_count(tree: &Tree, id: ui_ext::NodeId) -> u32 {
    match tree.get(id).map(TreeNode::props) {
        Some(Props::List(list)) => list.row_count,
        other => panic!("node {id} is not a list: {other:?}"),
    }
}

fn set_props_ids(update: &Update) -> Vec<ui_ext::NodeId> {
    match update {
        Update::Patches(patches) => patches
            .iter()
            .map(|patch| match patch {
                Patch::SetProps(id, _) => *id,
                other => panic!("expected only set-props, got {other:?}"),
            })
            .collect(),
        other => panic!("expected patches, got {other:?}"),
    }
}

#[gpui::test]
async fn test_demo_counter_round_trip(cx: &mut TestAppContext) {
    use counter::*;

    let (extension, _pushed) = load("fyx_demo_counter", demo_counter_wasm(), cx).await;

    assert_eq!(extension.api_version(), &Version::new(0, 1, 0));
    assert_eq!(
        extension.surfaces(),
        &[ui_ext::Surface {
            id: SURFACE.into(),
            title: "Counter".into(),
            kind: ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
                position: ui_ext::DockPosition::Right,
                icon: "hash".into(),
                default_size: 320,
            }),
        }]
    );

    let mut tree = extension.mount(SURFACE).await.expect("mount");
    assert_eq!(tree.len(), 7);
    assert_eq!(tree.roots(), &[ROOT]);
    assert_eq!(tree.children(Some(ROOT)), &[LABEL, BUTTONS, LIST]);
    assert_eq!(tree.children(Some(BUTTONS)), &[DECREMENT, INCREMENT, RESET]);
    assert_eq!(text_of(&tree, LABEL), "Count: 0");
    assert!(is_disabled(&tree, DECREMENT));
    assert!(is_disabled(&tree, RESET));
    assert_eq!(row_count(&tree, LIST), 0);

    let click = async |id| {
        extension
            .handle_event(SURFACE, UiEvent::Click(id))
            .await
            .unwrap_or_else(|error| panic!("click({id}): {error:#}"))
    };

    // Decrement at 0 changes nothing.
    let update = click(DECREMENT).await;
    assert_eq!(update, Update::Unchanged);
    assert!(!tree.apply_update(update).expect("valid update"));

    let update = click(INCREMENT).await;
    assert_eq!(set_props_ids(&update), [LABEL, DECREMENT, RESET, LIST]);
    assert!(tree.apply_update(update).expect("valid patches"));
    assert_eq!(text_of(&tree, LABEL), "Count: 1");
    assert!(!is_disabled(&tree, DECREMENT));
    assert!(!is_disabled(&tree, RESET));
    assert_eq!(row_count(&tree, LIST), 1);

    let update = click(INCREMENT).await;
    assert_eq!(set_props_ids(&update), [LABEL, LIST]);
    tree.apply_update(update).expect("valid patches");
    assert_eq!(text_of(&tree, LABEL), "Count: 2");
    assert_eq!(row_count(&tree, LIST), 2);

    let rows = extension
        .list_rows(SURFACE, LIST, 1, 10)
        .await
        .expect("rows");
    assert_eq!(rows.len(), 1);
    let (index, row) = &rows[0];
    assert_eq!(*index, 1);
    assert_eq!(row.roots(), &[ROW_ROOT]);
    assert_eq!(row.children(Some(ROW_ROOT)), &[ROW_TEXT]);
    assert_eq!(text_of(row, ROW_TEXT), "Row 2 of 2");

    // Errors the extension reports are not fatal.
    let error = extension
        .handle_event("other", UiEvent::Click(INCREMENT))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("from fyx:ui extension"),
        "{error:#}"
    );

    let update = click(RESET).await;
    assert_eq!(set_props_ids(&update), [LABEL, DECREMENT, RESET, LIST]);
    tree.apply_update(update).expect("valid patches");
    assert_eq!(text_of(&tree, LABEL), "Count: 0");
    assert!(is_disabled(&tree, DECREMENT));
    assert_eq!(click(RESET).await, Update::Unchanged);
    assert!(
        extension
            .list_rows(SURFACE, LIST, 0, 10)
            .await
            .expect("rows")
            .is_empty()
    );
}

#[gpui::test]
async fn test_pushed_update_reaches_the_host(cx: &mut TestAppContext) {
    let (extension, mut pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let update = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .expect("click");
    assert_eq!(update, Update::Unchanged);
    assert_eq!(
        pushed.next().await.expect("a pushed update"),
        PushedUpdate {
            surface_id: probe::SURFACE.into(),
            update: Update::Patches(vec![Patch::Remove(probe::PUSH)]),
        }
    );
}

#[gpui::test]
async fn test_runaway_call_is_interrupted_and_disables_extension(cx: &mut TestAppContext) {
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let started = Instant::now();
    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::SPIN))
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
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
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
        parse_api_version("probe", &probe_wasm()).expect("parses"),
        Some(Version::new(0, 1, 0))
    );
    let empty_core_module = b"\0asm\x01\0\0\0";
    assert_eq!(
        parse_api_version("empty", empty_core_module).expect("parses"),
        None
    );

    let version = [0, 0, 0, 1, 0, 0];
    assert_eq!(
        parse_api_version("one", &module_with_version_section(&version)).expect("parses"),
        Some(Version::new(0, 1, 0))
    );
    let error =
        parse_api_version("two", &module_with_version_section(&version.repeat(2))).unwrap_err();
    assert!(
        format!("{error:#}").contains("2 copies of the fyx:ui-api-version section"),
        "{error:#}"
    );
    let error = parse_api_version("short", &module_with_version_section(&[0, 1])).unwrap_err();
    assert!(
        format!("{error:#}").contains("invalid fyx:ui-api-version section"),
        "{error:#}"
    );
}

/// A core module whose only content is an `fyx:ui-api-version` custom section holding `data`.
fn module_with_version_section(data: &[u8]) -> Vec<u8> {
    let name = API_VERSION_SECTION.as_bytes();
    let payload_len = u8::try_from(1 + name.len() + data.len()).expect("fits one LEB128 byte");
    let name_len = u8::try_from(name.len()).expect("fits one LEB128 byte");
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    module.extend([0, payload_len, name_len]);
    module.extend(name);
    module.extend(data);
    module
}
