use std::path::{Path, PathBuf};

use gpui::TestAppContext;
use ui_ext::{Patch, Props, Tree, TreeNode, UiEvent, Update};

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
    pub const SLEEP: NodeId = 3;
    pub const PUSH_THEN_PATCH: NodeId = 4;
    pub const PUSH_THEN_FAIL: NodeId = 5;
    pub const PUSH_FLOOD: NodeId = 6;
    pub const ALLOCATE: NodeId = 7;
    pub const PUSH_BULK: NodeId = 8;
    pub const CONNECT: NodeId = 9;
    pub const PUSH_TEXT: NodeId = 10;
    pub const LOG_LONG: NodeId = 11;
    pub const SLOW_SURFACE: &str = "slow";
}

fn demo_counter_wasm() -> Vec<u8> {
    crate::test_support::demo_wasm("fyx_demo_counter")
}

fn probe_wasm() -> Vec<u8> {
    crate::test_support::fixture_wasm("fyx_ui_test_probe")
}

async fn load(
    name: &str,
    wasm: Vec<u8>,
    cx: &mut TestAppContext,
) -> (UiExtension, mpsc::UnboundedReceiver<PushedUpdate>) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    load_another(name, wasm, cx).await
}

/// Like `load`, for a second extension in the same test: initializing tokio again would replace
/// the runtime the first extension's queue runs on.
async fn load_another(
    name: &str,
    wasm: Vec<u8>,
    cx: &mut TestAppContext,
) -> (UiExtension, mpsc::UnboundedReceiver<PushedUpdate>) {
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

    let mut tree = extension.mount(SURFACE).await.expect("mount").value;
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
            .value
    };

    // Decrement at 0 changes nothing.
    let update = click(DECREMENT).await;
    assert_eq!(update, Update::Unchanged);
    assert!(tree.apply_update(update).expect("valid update").is_empty());

    let update = click(INCREMENT).await;
    assert_eq!(set_props_ids(&update), [LABEL, DECREMENT, RESET, LIST]);
    // The list node is among the touched ids, so the renderer knows to re-request its rows.
    assert!(
        tree.apply_update(update)
            .expect("valid patches")
            .contains(&LIST)
    );
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
        .expect("rows")
        .value;
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
            .value
            .is_empty()
    );
}

#[gpui::test]
async fn test_load_from_dir(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let fs = fs::FakeFs::new(cx.executor());
    let manifest = |fyx_ui: &str| {
        format!(
            "id = \"fyx-demo-counter\"\nname = \"Counter demo\"\nversion = \"0.1.0\"\n\
             schema_version = 1\n{fyx_ui}"
        )
        .into_bytes()
    };
    let wasm = demo_counter_wasm();
    let write = async |dir: &str, fyx_ui: &str| {
        fs.create_dir(Path::new(dir)).await.expect("creates dir");
        fs.insert_file(format!("{dir}/extension.toml"), manifest(fyx_ui))
            .await;
        fs.insert_file(format!("{dir}/fyx_ui.wasm"), wasm.clone())
            .await;
        PathBuf::from(dir)
    };
    let table = |capabilities: &str, component: &str, api_version: &str| {
        format!(
            "[fyx_ui]\napi_version = \"{api_version}\"\ncomponent = \"{component}\"\n\
             capabilities = [{capabilities}]\n"
        )
    };

    let counter = write(
        "/extensions/counter",
        &table("\"ui:panel\"", "fyx_ui.wasm", "0.1.0"),
    )
    .await;
    let plain = write("/extensions/plain", "").await;
    let escaping = write(
        "/extensions/escaping",
        &table("\"ui:panel\"", "../counter/fyx_ui.wasm", "0.1.0"),
    )
    .await;
    let future = write(
        "/extensions/future",
        &table("\"ui:panel\"", "fyx_ui.wasm", "0.4.0"),
    )
    .await;
    let mismatch = write(
        "/extensions/mismatch",
        &table("\"ui:panel\"", "fyx_ui.wasm", "0.2.0"),
    )
    .await;
    let no_panel = write("/extensions/no-panel", &table("", "fyx_ui.wasm", "0.1.0")).await;

    let load = |dir: PathBuf| {
        let fs = fs.clone();
        cx.update(|cx| {
            let cx = cx.to_async();
            async move { UiExtension::load_from_dir(&dir, fs, &cx).await }
        })
    };

    let (extension, _pushed) = load(counter)
        .await
        .expect("loads")
        .expect("has a [fyx_ui] table");
    assert_eq!(extension.extension_id().as_ref(), "fyx-demo-counter");
    assert_eq!(extension.surfaces()[0].id, counter::SURFACE);

    assert!(load(plain).await.expect("reads").is_none());
    for (dir, message) in [
        (escaping, "must be a relative path inside the extension"),
        (future, "targets fyx:ui 0.4.0"),
        (
            mismatch,
            "extension.toml says fyx:ui 0.2.0, the component targets 0.1.0",
        ),
        (no_panel, "needs \"ui:panel\""),
    ] {
        let error = load(dir).await.err().expect("rejected");
        assert!(format!("{error:#}").contains(message), "{error:#}");
    }
}

#[gpui::test]
async fn test_pushes_come_back_with_the_reply_in_order(cx: &mut TestAppContext) {
    let (extension, mut pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let reply = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .expect("click");
    assert_eq!(
        reply,
        Reply {
            pushed: vec![PushedUpdate {
                surface_id: probe::SURFACE.into(),
                update: Update::Patches(vec![Patch::Remove(probe::PUSH)]),
            }],
            value: Update::Unchanged,
        }
    );

    // The reply inserts under a node the push inserted: it applies only after the push.
    let reply = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH_THEN_PATCH))
        .await
        .expect("click");
    let mut tree = Tree::default();
    for pushed in reply.pushed {
        tree.apply_update(pushed.update).expect("push applies");
    }
    tree.apply_update(reply.value)
        .expect("reply applies after the push");
    assert_eq!(tree.children(Some(10)), &[11]);

    // A call that fails after pushing carries its pushes in the error, not in the channel.
    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH_THEN_FAIL))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("failed after pushing"),
        "{error:#}"
    );
    assert_eq!(
        error
            .downcast_ref::<ReplyError>()
            .map(|error| &error.pushed),
        Some(&vec![PushedUpdate {
            surface_id: probe::SURFACE.into(),
            update: Update::Patches(vec![Patch::Remove(probe::PUSH)]),
        }])
    );
    // The probe pushes nothing while it is instantiated, so the channel is closed and empty.
    assert_eq!(pushed.next().await, None);

    // A failed call is not fatal: the extension still answers.
    extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .expect("still works");
}

#[gpui::test]
async fn test_push_flood_traps_and_disables_extension(cx: &mut TestAppContext) {
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH_FLOOD))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(&format!("more than {MAX_PUSHES_PER_CALL} updates")),
        "{error:#}"
    );
    assert!(error.downcast_ref::<ReplyError>().is_none());

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("disabled"), "{error:#}");
}

/// Sandbox: a component cannot grow its memory past `UI_MEMORY_LIMIT` (256 MiB); allocating
/// 300 MiB traps the call with an explicit error and disables that extension only.
#[gpui::test]
async fn test_memory_past_the_limit_traps_and_disables_extension(cx: &mut TestAppContext) {
    assert_eq!(UI_MEMORY_LIMIT, 256 << 20);
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;
    let (bystander, _bystander_pushed) =
        load_another("fyx_ui_test_bystander", probe_wasm(), cx).await;

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::ALLOCATE))
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("memory"), "{message}");
    assert!(
        error.downcast_ref::<ReplyError>().is_none(),
        "a trap, not a rejection"
    );

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("disabled"), "{error:#}");

    // Another extension's instance is untouched.
    bystander
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .expect("the other extension still works");
}

/// Sandbox: the text of one call's pushes and logs is capped at `MAX_LIFTED_TEXT_PER_CALL` in
/// total, though each piece is within guest memory: five pushes of the same 4 MiB string trap.
/// One long log message is cut, not refused.
#[gpui::test]
async fn test_lifted_text_past_the_cap_traps(cx: &mut TestAppContext) {
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::LOG_LONG))
        .await
        .expect("a 1 MiB log message is within the cap");

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH_TEXT))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(&format!(
            "more than {MAX_LIFTED_TEXT_PER_CALL} bytes of text"
        )),
        "{error:#}"
    );
    assert!(error.downcast_ref::<ReplyError>().is_none());
}

/// The first mount of a surface gets `UI_FIRST_MOUNT_BUDGET` (2 s); a later mount of the same
/// surface gets the regular `UI_CALL_BUDGET`. The probe's `slow` surface takes a second to mount.
#[gpui::test]
async fn test_first_mount_gets_the_longer_budget(cx: &mut TestAppContext) {
    assert_eq!(UI_FIRST_MOUNT_BUDGET, Duration::from_secs(2));
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let reply = extension
        .mount(probe::SLOW_SURFACE)
        .await
        .expect("the first mount may take a second");
    assert_eq!(text_of(&reply.value, 1), "slow");

    let error = extension.mount(probe::SLOW_SURFACE).await.unwrap_err();
    assert!(format!("{error:#}").contains("time budget"), "{error:#}");
}

/// Sandbox: the pushes of one call may carry at most `MAX_PUSHED_ITEMS_PER_CALL` patches and
/// nodes together, even when each push is small and there are fewer than `MAX_PUSHES_PER_CALL`.
#[gpui::test]
async fn test_pushed_items_past_the_cap_trap(cx: &mut TestAppContext) {
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH_BULK))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains(&format!(
            "more than {MAX_PUSHED_ITEMS_PER_CALL} patches and nodes"
        )),
        "{error:#}"
    );
    assert!(error.downcast_ref::<ReplyError>().is_none());
}

/// Sandbox: WASI sockets are linked but every network use is denied, so a guest's
/// `TcpStream::connect` fails inside the guest. The extension reports the error and stays usable.
#[gpui::test]
async fn test_guest_cannot_open_a_socket(cx: &mut TestAppContext) {
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::CONNECT))
        .await
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("connect failed"), "{message}");
    // Denied by the sandbox, not refused by a closed port.
    let lower = message.to_lowercase();
    assert!(
        lower.contains("denied") || lower.contains("not permitted"),
        "{message}"
    );
    assert!(
        error.downcast_ref::<ReplyError>().is_some(),
        "the guest's own error, not a trap: {message}"
    );
    // Not disabled: the next call works.
    extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .expect("the extension is still usable");
}

#[gpui::test]
async fn test_version_falls_back_to_imports(cx: &mut TestAppContext) {
    // Renaming the section in place keeps every size field valid; the probe still imports
    // fyx:ui/host@0.1.0 and fyx:ui/types@0.1.0.
    let mut wasm = probe_wasm();
    let name = API_VERSION_SECTION.as_bytes();
    let mut renamed = 0;
    let mut start = 0;
    while let Some(offset) = wasm[start..]
        .windows(name.len())
        .position(|window| window == name)
    {
        let end = start + offset + name.len();
        wasm[end - 1] = b'N';
        renamed += 1;
        start = end;
    }
    assert!(renamed > 0, "the probe has a version section to rename");
    assert_eq!(parse_api_version("probe", &wasm).expect("parses"), None);

    let (extension, _pushed) = load("fyx_ui_test_probe", wasm, cx).await;
    assert_eq!(extension.api_version(), &Version::new(0, 1, 0));
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

#[gpui::test]
async fn test_call_blocked_in_host_is_cut_off_and_disables_extension(cx: &mut TestAppContext) {
    let (extension, _pushed) = load("fyx_ui_test_probe", probe_wasm(), cx).await;

    let started = Instant::now();
    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::SLEEP))
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(
        elapsed >= UI_CALL_BUDGET && elapsed < Duration::from_millis(600),
        "cut off after {elapsed:?}"
    );
    assert!(format!("{error:#}").contains("time budget"), "{error:#}");

    let error = extension
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("disabled"), "{error:#}");
}

/// Plan 1.1: the first load compiles and writes a `.cwasm`; the next uses it; a broken copy is
/// compiled again and replaced; another component gets a file of its own.
#[gpui::test]
async fn test_precompiled_cache(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let dir = tempfile::tempdir().expect("temp dir");
    let cache = PrecompiledCache {
        dir: dir.path().join("probe"),
        stem: "0.1.0".into(),
    };
    let wasm = probe_wasm();
    let file = cache.file_for(&wasm).expect("cache file");
    assert!(file.starts_with(dir.path().join("probe")));
    assert!(
        file.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("0.1.0-")
    );

    let load = |cx: &mut TestAppContext| {
        let wasm = wasm.clone();
        let cache = cache.clone();
        cx.update(|cx| {
            UiExtension::load_with_cache("probe".into(), wasm, Some(cache), &cx.to_async())
        })
    };
    let (first, _) = load(cx).await.expect("compiles");
    assert!(!first.from_precompiled());
    assert!(file.is_file(), "written after compiling");

    let (second, _) = load(cx).await.expect("loads the copy");
    assert!(second.from_precompiled());
    second
        .handle_event(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .await
        .expect("the precompiled component works");

    std::fs::write(&file, b"not a cwasm").expect("break the copy");
    let (third, _) = load(cx).await.expect("compiles again");
    assert!(!third.from_precompiled());
    let (fourth, _) = load(cx).await.expect("uses the rewritten copy");
    assert!(fourth.from_precompiled());

    let other = cache
        .file_for(&demo_counter_wasm())
        .expect("other cache file");
    assert_ne!(other, file, "keyed by the component");
}

#[test]
fn test_api_version_gating() {
    assert!(is_supported_api_version(&Version::new(0, 1, 0)));
    assert!(is_supported_api_version(&Version::new(0, 2, 0)));
    for unknown in [(0, 0, 9), (0, 1, 1), (0, 4, 0), (1, 0, 0)] {
        let version = Version::new(unknown.0, unknown.1, unknown.2);
        assert!(!is_supported_api_version(&version), "{version}");
    }
    assert_eq!(
        known_api_version(&Version::new(0, 2, 0)),
        Some(ApiVersion::V0_2_0)
    );
    assert_eq!(known_api_version(&"0.2.0-rc.1".parse().unwrap()), None);
    // Staged: 0.2 loads only where staged versions are allowed (Stable/Preview stay on 0.1).
    assert!(!api_version_range(false).contains(&Version::new(0, 2, 0)));
    assert!(api_version_range(false).contains(&Version::new(0, 1, 0)));
    assert!(api_version_range(true).contains(&Version::new(0, 2, 0)));
    assert!(!staged_api_allowed(false, false));
    assert!(staged_api_allowed(true, false));
    assert!(staged_api_allowed(false, true));

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

/// Sandbox: extension and surface ids are restricted (they name panels, settings keys and files,
/// and `<extension>/<surface>` must stay unambiguous).
#[test]
fn test_id_rules() {
    for good in ["fyx-demo-counter", "a", "a1_b-2", "9lives"] {
        validate_id("extension", good).unwrap_or_else(|error| panic!("{good}: {error:#}"));
    }
    let too_long = "a".repeat(MAX_ID_LEN + 1);
    for bad in [
        "",
        "Counter",
        "a/b",
        "../x",
        "-dash",
        "_under",
        "with space",
        "dot.ted",
        "ünï",
        too_long.as_str(),
    ] {
        let error = validate_id("extension", bad).expect_err(bad);
        assert!(format!("{error:#}").contains("extension id"), "{error:#}");
    }

    let surface = |id: &str| ui_ext::Surface {
        id: id.into(),
        title: "T".into(),
        kind: ui_ext::SurfaceKind::StatusItem(ui_ext::StatusItemDescriptor {
            side: ui_ext::StatusBarSide::Left,
            priority: 0,
        }),
    };
    validate_surfaces(&[surface("main"), surface("status")]).expect("valid");
    let error = validate_surfaces(&[surface("main"), surface("main")]).expect_err("duplicate");
    assert!(format!("{error:#}").contains("declared twice"), "{error:#}");
    let error = validate_surfaces(&[surface("Main")]).expect_err("bad id");
    assert!(format!("{error:#}").contains("surface id"), "{error:#}");
}

/// A component cannot be loaded under an invalid id, and a manifest with one is refused with its
/// path in the error.
#[gpui::test]
async fn test_invalid_ids_are_refused_at_load(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let error = cx
        .update(|cx| UiExtension::load("Bad/Id".into(), probe_wasm(), &cx.to_async()))
        .await
        .err()
        .expect("refused");
    assert!(
        format!("{error:#}").contains("extension id \"Bad/Id\""),
        "{error:#}"
    );

    let fs = fs::FakeFs::new(cx.executor());
    fs.create_dir(Path::new("/ext/bad")).await.expect("dir");
    fs.insert_file(
        "/ext/bad/extension.toml",
        b"id = \"../escape\"\nname = \"Bad\"\nversion = \"0.1.0\"\nschema_version = 1\n\
          [fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"fyx_ui.wasm\"\n"
            .to_vec(),
    )
    .await;
    let error = UiManifest::read(fs.as_ref(), Path::new("/ext/bad"))
        .await
        .err()
        .expect("refused");
    let message = format!("{error:#}");
    assert!(message.contains("extension id \"../escape\""), "{message}");
    assert!(message.contains("extension.toml"), "{message}");
}

/// Triage-3, PERF 7: the epoch thread parks while no guest call runs and ticks during one.
#[test]
fn test_epoch_thread_parks_without_calls() {
    ui_runtime().expect("runtime");
    std::thread::sleep(UI_EPOCH_INTERVAL * 5);
    let idle = EPOCH_TICKS.load(Ordering::Relaxed);
    std::thread::sleep(UI_EPOCH_INTERVAL * 10);
    assert_eq!(
        EPOCH_TICKS.load(Ordering::Relaxed),
        idle,
        "no ticks while idle"
    );

    // A call waiting on the epoch thread's clock finishes, so the thread woke for it.
    futures::executor::block_on(with_budget(Duration::from_secs(5), async {
        // Ready only after some time, and woken only by the epoch thread.
        let ready_at = Instant::now() + UI_EPOCH_INTERVAL * 5;
        futures::future::poll_fn(|_| {
            if Instant::now() >= ready_at {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        anyhow::Ok(())
    }))
    .expect("the call finishes in its budget");
    assert!(EPOCH_TICKS.load(Ordering::Relaxed) > idle);
}

/// `fyx_ext_harness` (Apache-2.0) runs guests for extension tests without this GPL runtime, so it
/// mirrors the runtime's call contract instead of depending on it. These run the same guests
/// through both and compare every call, so the two cannot drift apart unnoticed.
mod harness_conformance {
    use fyx_ext_harness::{Harness, Pushed};

    use super::*;

    fn pushes(host: &[PushedUpdate]) -> Vec<Pushed> {
        host.iter()
            .map(|push| Pushed {
                surface_id: push.surface_id.clone(),
                update: push.update.clone(),
            })
            .collect()
    }

    #[test]
    fn limits_match() {
        assert_eq!(fyx_ext_harness::MAX_PUSHES_PER_CALL, MAX_PUSHES_PER_CALL);
        assert_eq!(
            fyx_ext_harness::MAX_PUSHED_ITEMS_PER_CALL,
            MAX_PUSHED_ITEMS_PER_CALL
        );
        assert_eq!(
            fyx_ext_harness::MAX_LIFTED_TEXT_PER_CALL,
            MAX_LIFTED_TEXT_PER_CALL
        );
        assert_eq!(fyx_ext_harness::MAX_LOG_MESSAGE_LEN, MAX_LOG_MESSAGE_LEN);
        assert_eq!(fyx_ext_harness::CALL_BUDGET, UI_CALL_BUDGET);
        assert_eq!(fyx_ext_harness::INIT_BUDGET, UI_INIT_BUDGET);
        assert_eq!(fyx_ext_harness::EPOCH_INTERVAL, UI_EPOCH_INTERVAL);
        assert_eq!(fyx_ext_harness::API_VERSION_SECTION, API_VERSION_SECTION);
        assert_eq!(fyx_ext_harness::MEMORY_LIMIT, UI_MEMORY_LIMIT);
        assert_eq!(
            fyx_ext_harness::TABLE_ELEMENTS_LIMIT,
            UI_TABLE_ELEMENTS_LIMIT
        );
        assert_eq!(fyx_ext_harness::INSTANCES_LIMIT, UI_INSTANCES_LIMIT);
        assert_eq!(fyx_ext_harness::TABLES_LIMIT, UI_TABLES_LIMIT);
        assert_eq!(fyx_ext_harness::MEMORIES_LIMIT, UI_MEMORIES_LIMIT);
        assert_eq!(
            fyx_ext_harness::MAX_PUSHED_ITEMS_PER_CALL_V0_2,
            MAX_PUSHED_ITEMS_PER_CALL_V0_2
        );
        assert_eq!(fyx_ext_harness::MIN_TICK_DELAY, MIN_TICK_DELAY);
        assert_eq!(fyx_ext_harness::MAX_TICK_DELAY, MAX_TICK_DELAY);
        assert_eq!(fyx_ext_harness::ACTIVATION_WINDOW, ACTIVATION_WINDOW);
        assert_eq!(
            fyx_ext_harness::MAX_CONSECUTIVE_TICK_ERRORS,
            crate::ticks::MAX_CONSECUTIVE_TICK_ERRORS
        );
        // The harness runs every version the host runs (plan 4.12): the ranges are equal again.
        let harness_versions = fyx_ext_harness::SUPPORTED_API_VERSIONS;
        assert_eq!(
            harness_versions,
            [MIN_VERSION, Version::new(0, 2, 0), MAX_VERSION]
        );
        assert!(harness_versions.iter().all(is_supported_api_version));
        assert!(harness_versions.contains(&MAX_FROZEN_VERSION));
        assert_eq!(MAX_VERSION, Version::new(0, 3, 0));
    }

    #[gpui::test]
    async fn test_counter_matches_the_harness(cx: &mut TestAppContext) {
        use counter::*;

        let wasm = demo_counter_wasm();
        let (host, _pushed) = load("fyx-demo-counter", wasm.clone(), cx).await;
        let mut harness = Harness::load(&wasm).expect("the harness loads the counter");
        assert_eq!(host.surfaces(), harness.surfaces());

        let mut tree = host.mount(SURFACE).await.expect("mount").value;
        assert_eq!(&tree, harness.mount(SURFACE).expect("mount"));

        for id in [INCREMENT, INCREMENT, DECREMENT, RESET, ROOT, INCREMENT] {
            let reply = host
                .handle_event(SURFACE, UiEvent::Click(id))
                .await
                .expect("event");
            let outcome = harness.event(SURFACE, UiEvent::Click(id)).expect("event");
            assert_eq!(pushes(&reply.pushed), outcome.pushed, "click {id}");
            assert_eq!(reply.value, outcome.update, "click {id}");
            tree.apply_update(reply.value).expect("apply");
            assert_eq!(Some(&tree), harness.tree(SURFACE), "click {id}");
        }

        let host_rows = host
            .list_rows(SURFACE, LIST, 0, 10)
            .await
            .expect("rows")
            .value;
        let harness_rows = harness.rows(SURFACE, LIST, 0, 10).expect("rows");
        assert_eq!(host_rows, harness_rows);
        assert!(!host_rows.is_empty());
    }

    #[gpui::test]
    async fn test_probe_matches_the_harness(cx: &mut TestAppContext) {
        let wasm = probe_wasm();
        let (host, _pushed) = load("probe", wasm.clone(), cx).await;
        let mut harness = Harness::load(&wasm).expect("the harness loads the probe");
        assert_eq!(host.surfaces(), harness.surfaces());

        for id in [probe::PUSH, probe::PUSH_THEN_PATCH, probe::PUSH_THEN_FAIL] {
            let event = UiEvent::Click(id);
            let (host_pushed, host_value) =
                match host.handle_event(probe::SURFACE, event.clone()).await {
                    Ok(reply) => (pushes(&reply.pushed), Ok(reply.value)),
                    Err(error) => {
                        let error = error
                            .downcast_ref::<ReplyError>()
                            .unwrap_or_else(|| panic!("click {id}: not a guest error: {error:#}"));
                        (pushes(&error.pushed), Err(()))
                    }
                };
            let reply = harness
                .handle_event_raw(probe::SURFACE, event)
                .expect("no trap");
            assert_eq!(host_pushed, reply.pushed, "click {id}");
            assert_eq!(host_value, reply.value.map_err(|_| ()), "click {id}");
        }

        let flood = UiEvent::Click(probe::PUSH_FLOOD);
        assert!(
            host.handle_event(probe::SURFACE, flood.clone())
                .await
                .is_err()
        );
        assert!(harness.handle_event_raw(probe::SURFACE, flood).is_err());
        let next = UiEvent::Click(probe::PUSH);
        assert!(
            host.handle_event(probe::SURFACE, next.clone())
                .await
                .is_err(),
            "disabled"
        );
        assert!(
            harness.handle_event_raw(probe::SURFACE, next).is_err(),
            "disabled"
        );
    }
}
