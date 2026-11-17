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

fn gallery_wasm() -> Vec<u8> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    build_guest(
        &BUILT,
        workspace_dir(),
        &["-p", "fyx_demo_gallery"],
        "debug/fyx_demo_gallery.wasm",
    )
}

/// The nodes under `root` (included), parents first.
fn subtree(
    state: &Entity<SurfaceState>,
    root: NodeId,
    cx: &mut TestAppContext,
) -> Vec<(NodeId, Props)> {
    state.read_with(cx, |state, _| {
        let tree = state.tree().expect("mounted");
        let mut nodes = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = tree.get(id) else {
                continue;
            };
            nodes.push((id, node.props().clone()));
            stack.extend(node.children().iter().rev());
        }
        nodes
    })
}

fn find(nodes: &[(NodeId, Props)], matches: impl Fn(&Props) -> bool) -> NodeId {
    nodes
        .iter()
        .find(|(_, props)| matches(props))
        .map(|(id, _)| *id)
        .expect("node exists")
}

fn button_labelled(nodes: &[(NodeId, Props)], label: &str, disabled: bool) -> NodeId {
    find(
        nodes,
        |props| matches!(props, Props::Button(button) if button.label == label && button.disabled == disabled),
    )
}

fn text_starting(nodes: &[(NodeId, Props)], prefix: &str) -> NodeId {
    find(
        nodes,
        |props| matches!(props, Props::Text(TextProps { content, .. }) if content.starts_with(prefix)),
    )
}

fn leak(selector: String) -> &'static str {
    selector.leak()
}

/// Every leaf of the shown section has a non-empty box; every list and table has fetched rows.
fn assert_section_drawn(
    name: &str,
    state: &Entity<SurfaceState>,
    body_scroll: &gpui::ScrollHandle,
    section: NodeId,
    cx: &mut VisualTestContext,
) {
    let nodes = subtree(state, section, cx);
    let window_height = cx.update(|window, _| window.bounds().size.height);
    let mut clipped = Vec::new();
    for (id, props) in &nodes {
        match props {
            Props::Flex(_) => {}
            Props::List(_) | Props::Table(_) => {
                let id = *id;
                run_until(cx, &format!("{name}: rows of {id}"), |cx| {
                    state.read_with(cx, |state, _| state.row(id, 0).is_some())
                });
                cx.update(|window, _| window.refresh());
                cx.run_until_parked();
                let row = cx
                    .debug_bounds(leak(format!("fyx-row-{id}-0")))
                    .unwrap_or_else(|| panic!("{name}: row 0 of {id} is not drawn"));
                assert!(
                    row.size.width > gpui::Pixels::ZERO,
                    "{name}: row 0 of {id} is empty"
                );
            }
            _ => {
                let bounds = cx
                    .debug_bounds(leak(format!("fyx-node-{id}")))
                    .unwrap_or_else(|| panic!("{name}: node {id} ({props:?}) is not drawn"));
                assert!(
                    bounds.size.width > gpui::Pixels::ZERO
                        && bounds.size.height > gpui::Pixels::ZERO,
                    "{name}: node {id} ({props:?}) has an empty box: {bounds:?}"
                );
                if bounds.bottom() > window_height {
                    clipped.push(*id);
                }
            }
        }
    }
    if clipped.is_empty() {
        return;
    }
    // Taller than the panel: the body must scroll far enough to bring every node into view.
    let max = body_scroll.max_offset().y;
    assert!(
        max > gpui::Pixels::ZERO,
        "{name}: nodes {clipped:?} lie below the {window_height:?} window and the body does not scroll"
    );
    body_scroll.set_offset(gpui::point(gpui::Pixels::ZERO, -max));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    for id in clipped {
        let bounds = cx
            .debug_bounds(leak(format!("fyx-node-{id}")))
            .unwrap_or_else(|| panic!("{name}: node {id} is not drawn after scrolling"));
        assert!(
            bounds.top() >= gpui::Pixels::ZERO && bounds.bottom() <= window_height,
            "{name}: node {id} is still out of sight after scrolling to the end: {bounds:?}"
        );
    }
    body_scroll.set_offset(gpui::point(gpui::Pixels::ZERO, gpui::Pixels::ZERO));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
async fn test_gallery_renders_every_section(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let extension = load("fyx-demo-gallery", gallery_wasm(), cx).await;
    // Through the registry, so a push for the status item made during a panel event is routed.
    let registry = cx.new(|_| crate::UiExtensionRegistry::default());
    registry.update(cx, |registry, cx| {
        registry.register_backend(Rc::new(extension.clone()), extension.surfaces(), cx)
    });
    let (panel_info, panel_state, status_state) = registry.read_with(cx, |registry, _| {
        let panel = registry.surface("fyx-demo-gallery/gallery").expect("panel");
        let status = registry
            .surface("fyx-demo-gallery/gallery-status")
            .expect("status item");
        (
            panel.info.clone(),
            panel.state.clone(),
            status.state.clone(),
        )
    });
    run_until(cx, "both mounts", |cx| {
        panel_state.read_with(cx, |state, _| state.tree().is_some())
            && status_state.read_with(cx, |state, _| state.tree().is_some())
    });

    let (_status, status_cx) =
        cx.add_window_view(|window, cx| ExtensionStatusItem::new(status_state.clone(), window, cx));
    status_cx.run_until_parked();
    // Ids pinned by the gallery's contract (tasks/progress/W4b.md).
    const STATUS_LABEL: NodeId = 3;
    assert_eq!(
        text_of(&status_state, STATUS_LABEL, status_cx).as_deref(),
        Some("Gallery: none")
    );
    let _ = status_cx
        .debug_bounds("fyx-node-3")
        .expect("the status item is drawn");

    let (panel, cx) = cx.add_window_view(|window, cx| {
        ExtensionPanel::new(panel_info.clone(), panel_state.clone(), window, cx)
    });
    // A tall dock panel.
    cx.simulate_resize(gpui::size(gpui::px(420.), gpui::px(800.)));
    cx.run_until_parked();
    const SLOT: NodeId = 14;
    let body_scroll = panel.read_with(cx, |panel, cx| panel.view().read(cx).body_scroll.clone());
    let shown = |cx: &mut TestAppContext| {
        panel_state.read_with(cx, |state, _| {
            state.tree().expect("mounted").children(Some(SLOT))[0]
        })
    };
    assert_section_drawn("Layout", &panel_state, &body_scroll, shown(cx), cx);

    // Navigation buttons, by the section they show.
    for (nav, name) in [
        (5, "Text"),
        (6, "Buttons"),
        (7, "Input"),
        (8, "List"),
        (10, "Table"),
        (11, "Icons"),
        (12, "Markdown"),
        (13, "Diff"),
        (4, "Layout"),
    ] {
        let before = shown(cx);
        click(leak(format!("fyx-node-{nav}")), cx);
        run_until(cx, &format!("the {name} section"), |cx| shown(cx) != before);
        cx.run_until_parked();
        let section = shown(cx);
        assert_section_drawn(name, &panel_state, &body_scroll, section, cx);
        let nodes = subtree(&panel_state, section, cx);

        match name {
            "Buttons" => {
                let last = text_starting(&nodes, "Last clicked");
                for (label, expected) in [
                    ("Danger", "Last clicked: Danger"),
                    ("", "Last clicked: Icon only (settings)"),
                ] {
                    click(
                        leak(format!(
                            "fyx-node-{}",
                            button_labelled(&nodes, label, false)
                        )),
                        cx,
                    );
                    run_until(cx, expected, |cx| {
                        text_of(&panel_state, last, cx).as_deref() == Some(expected)
                    });
                }
                // A disabled button sends nothing, so the label stays.
                click(
                    leak(format!(
                        "fyx-node-{}",
                        button_labelled(&nodes, "Default", true)
                    )),
                    cx,
                );
                std::thread::sleep(Duration::from_millis(100));
                cx.run_until_parked();
                assert_eq!(
                    text_of(&panel_state, last, cx).as_deref(),
                    Some("Last clicked: Icon only (settings)")
                );
            }
            "Input" => {
                let input = find(&nodes, |props| matches!(props, Props::Input(_)));
                let submitted = text_starting(&nodes, "Submitted");
                let editor = panel.read_with(cx, |panel, cx| {
                    panel.view().read(cx).inputs[&input].editor.clone()
                });
                cx.update(|window, cx| {
                    use gpui::Focusable as _;
                    editor.focus_handle(cx).focus(window, cx)
                });
                cx.run_until_parked();
                cx.simulate_input("hello");
                cx.simulate_keystrokes("enter");
                run_until(cx, "the submit echo", |cx| {
                    text_of(&panel_state, submitted, cx).as_deref() == Some("Submitted: hello")
                });
                // The gallery does not patch the input: the text stays the user's.
                assert_eq!(editor.read_with(cx, |editor, cx| editor.text(cx)), "hello");
            }
            "List" => {
                let list = find(&nodes, |props| matches!(props, Props::List(_)));
                let selected = text_starting(&nodes, "Selected");
                click(leak(format!("fyx-row-{list}-2")), cx);
                run_until(cx, "the row click", |cx| {
                    text_of(&panel_state, selected, cx).as_deref() == Some("Selected: Row 3")
                });
                // The gallery pushed to its status item during that call; the registry routed it.
                run_until(cx, "the routed push", |cx| {
                    text_of(&status_state, STATUS_LABEL, cx).as_deref() == Some("Gallery: Row 3")
                });
            }
            "Table" => {
                let table = find(&nodes, |props| matches!(props, Props::Table(_)));
                let row = panel_state
                    .read_with(cx, |state, _| state.row(table, 0).cloned())
                    .expect("row 0");
                assert_eq!(
                    row.children(Some(row.roots()[0])).len(),
                    3,
                    "one cell per column"
                );
            }
            _ => {}
        }
    }
}
