//! The renderer against real WASM components: the counter demo in an `ExtensionPanel` and the
//! status-item fixture in an `ExtensionStatusItem`, clicked through GPUI. This covers the
//! "click → handle-event → patch → repaint" loop without building and running the editor.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};
use ui_ext_wasm::permissions::Tier;

use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use ui_ext::v0_2::{NodeId, Props, TextProps, UiEvent};
use ui_ext_wasm::UiExtension;

use crate::{ExtensionPanel, ExtensionStatusItem, SurfaceInfo, SurfaceState, backend::UiBackend};

fn counter_wasm() -> Vec<u8> {
    ui_ext_wasm::test_support::demo_wasm("fyx_demo_counter")
}

fn status_item_wasm() -> Vec<u8> {
    ui_ext_wasm::test_support::fixture_wasm("fyx_ui_test_status_item")
}

/// The status-item fixture's manifest, next to its source in `ui_ext_wasm`'s fixture workspace.
fn status_item_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui_ext_wasm/fixtures/status_item/extension.toml")
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
    // Wall-clock, and generous: under a loaded full gate, compiling a component alone can take
    // several seconds (W0, integration de84e8d4b3).
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
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
    let surface: ui_ext::v0_2::Surface = extension.surfaces()[0].clone().into();
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
        &extension.surfaces()[0].clone().into(),
        100,
    ));
    let (_panel, vcx) = cx.add_window_view(|window, cx| {
        ExtensionPanel::<0>::new(info.clone(), state.clone(), window, cx)
    });

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
        ui_ext::v0_2::Surface::from(extension.surfaces()[0].clone()).kind,
        ui_ext::v0_2::SurfaceKind::StatusItem(_)
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
    ui_ext_wasm::test_support::demo_wasm("fyx_demo_gallery")
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
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    registry.update(cx, |registry, cx| {
        registry.register_backend(
            Rc::new(extension.clone()),
            &extension
                .surfaces()
                .iter()
                .cloned()
                .map(Into::into)
                .collect::<Vec<_>>(),
            cx,
        )
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
        ExtensionPanel::<0>::new(panel_info.clone(), panel_state.clone(), window, cx)
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

                // The mouse wheel over the list scrolls the list (24 px rows): 20 rows down, row 0
                // leaves the view and row 20 is fetched and drawn at the top.
                let row0 = cx
                    .debug_bounds(leak(format!("fyx-row-{list}-0")))
                    .expect("row 0 is drawn");
                cx.simulate_event(gpui::ScrollEvent {
                    position: row0.center(),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-480.))),
                    ..Default::default()
                });
                run_until(cx, "rows after scrolling", |cx| {
                    panel_state.read_with(cx, |state, _| state.row(list, 25).is_some())
                });
                cx.update(|window, _| window.refresh());
                cx.run_until_parked();
                let row20 = cx
                    .debug_bounds(leak(format!("fyx-row-{list}-20")))
                    .expect("row 20 is drawn after scrolling");
                assert!(
                    (row20.top() - row0.top()).abs() < gpui::px(1.),
                    "row 20 took row 0's place: {row20:?} vs {row0:?}"
                );
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

/// Plan 1.1: an extension installed from a `.fyxext` package loads from the install store and
/// writes its precompiled component; an update loads the new version in place of the old (the
/// surface remounts), and a rollback brings the previous one back.
#[gpui::test]
async fn test_store_installs_load_update_and_roll_back(cx: &mut TestAppContext) {
    use crate::install_store::{InstallStore, test_support::package};
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let wasm = status_item_wasm();
    let root = tempfile::tempdir().expect("temp dir");
    let installed = root.path().join("installed");
    let store = InstallStore::new(root.path().join("fyx"));
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    let install = |version: &str| {
        let path = root.path().join(format!("{version}.fyxext"));
        let store = store.clone();
        let fs = fs.clone();
        let wasm = wasm.clone();
        let version = version.to_string();
        async move {
            std::fs::write(
                &path,
                package("fyx-test-status-item", &version, &wasm).await,
            )
            .expect("package");
            store.install_package(&path, fs.as_ref()).await
        }
    };
    let cwasm_files = || -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.root().join("fyx-test-status-item"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".cwasm"))
            .map(|name| name.split('-').next().unwrap().to_string())
            .collect();
        names.sort();
        names
    };

    install("0.1.0").await.expect("installs");
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting().with_store(store.clone()));
    registry.update(cx, |registry, cx| {
        registry.scan_installed(installed.clone(), fs.clone(), cx)
    });
    const KEY: &str = "fyx-test-status-item/status";
    const LABEL: NodeId = 3;
    const PING: NodeId = 4;
    let state = |cx: &mut TestAppContext| {
        registry.read_with(cx, |registry, _| {
            registry.surface(KEY).map(|surface| surface.state.clone())
        })
    };
    run_until(cx, "the store install", |cx| {
        state(cx).is_some_and(|state| text_of(&state, LABEL, cx).is_some())
    });
    run_until(cx, "the precompiled component", |_| {
        cwasm_files() == ["0.1.0"]
    });
    let surface = state(cx).unwrap();
    surface.update(cx, |state, cx| state.dispatch(UiEvent::Click(PING), cx));
    run_until(cx, "a ping", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 1")
    });

    // An update: the same surface remounts on the new version.
    install("0.2.0").await.expect("updates");
    registry.update(cx, |registry, cx| registry.rescan(cx));
    run_until(cx, "the updated extension", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 0")
            && cwasm_files() == ["0.1.0", "0.2.0"]
    });
    assert_eq!(
        state(cx).map(|state| state.entity_id()),
        Some(surface.entity_id()),
        "the surface is kept, its backend replaced"
    );

    // Rollback.
    surface.update(cx, |state, cx| state.dispatch(UiEvent::Click(PING), cx));
    run_until(cx, "a ping on 0.2.0", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 1")
    });
    store
        .rollback("fyx-test-status-item", fs.as_ref())
        .await
        .unwrap();
    registry.update(cx, |registry, cx| registry.rescan(cx));
    run_until(cx, "the rolled back extension", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 0")
    });
}

/// Integration-4 finding 3: an installed extension without consent is not loaded; it waits with
/// its capabilities until "Enable", which is remembered across restarts (the consent file). When
/// `extension.toml` later asks for more, it waits again.
#[gpui::test]
async fn test_installed_extensions_wait_for_consent(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let root = tempfile::tempdir().expect("temp dir");
    let installed = root.path().join("installed");
    let consent_path = root.path().join("data").join("fyx-ui-consent.json");
    let ui_dir = installed.join("fyx-test-status-item");
    std::fs::create_dir_all(&ui_dir).expect("ui dir");
    std::fs::copy(status_item_manifest(), ui_dir.join("extension.toml")).expect("manifest");
    std::fs::write(ui_dir.join("fyx_ui.wasm"), status_item_wasm()).expect("component");
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    const KEY: &str = "fyx-test-status-item/status";

    // A start: a fresh registry reading the same consent file.
    let start = |cx: &mut TestAppContext| {
        let registry = cx.new(|_| crate::UiExtensionRegistry::new(consent_path.clone()));
        registry.update(cx, |registry, cx| {
            registry.scan_installed(installed.clone(), fs.clone(), cx)
        });
        registry
    };
    let pending = |registry: &Entity<crate::UiExtensionRegistry>, cx: &mut TestAppContext| {
        registry.read_with(cx, |registry, _| {
            registry
                .pending_consent()
                .iter()
                .map(|pending| {
                    (
                        pending.extension_id.to_string(),
                        pending.capabilities.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    let loaded = |registry: &Entity<crate::UiExtensionRegistry>, cx: &mut TestAppContext| {
        registry.read_with(cx, |registry, _| registry.surface(KEY).is_some())
    };

    let registry = start(cx);
    run_until(cx, "the consent request", |cx| {
        !pending(&registry, cx).is_empty()
    });
    assert_eq!(
        pending(&registry, cx),
        [(
            "fyx-test-status-item".to_string(),
            vec!["ui:status-bar".to_string()]
        )]
    );
    assert!(!loaded(&registry, cx), "not loaded without consent");

    registry.update(cx, |registry, cx| {
        registry.enable("fyx-test-status-item", fs.clone(), cx)
    });
    run_until(cx, "the enabled extension", |cx| loaded(&registry, cx));
    assert!(pending(&registry, cx).is_empty());
    run_until(cx, "the consent file", |_| consent_path.exists());

    // The next start remembers it.
    let registry = start(cx);
    run_until(cx, "the remembered extension", |cx| loaded(&registry, cx));
    assert!(pending(&registry, cx).is_empty());

    // Plan 1.5: an update that adds only T0 loads silently (and a changed component needs
    // nothing either) ...
    let manifest = std::fs::read_to_string(ui_dir.join("extension.toml")).expect("manifest");
    let with = |capabilities: &str| {
        std::fs::write(
            ui_dir.join("extension.toml"),
            manifest.replace(
                r#"capabilities = ["ui:status-bar"]"#,
                &format!("capabilities = [{capabilities}]"),
            ),
        )
        .expect("edited manifest");
    };
    with(r#""ui:status-bar", "ui:panel""#);
    let registry = start(cx);
    run_until(cx, "the T0 update", |cx| loaded(&registry, cx));
    assert!(pending(&registry, cx).is_empty());

    // ... while one that adds T1 is held for approval, and approving grants it.
    with(r#""ui:status-bar", "workspace:read-buffers""#);
    let registry = start(cx);
    run_until(cx, "the held update", |cx| {
        !pending(&registry, cx).is_empty()
    });
    let held = registry.read_with(cx, |registry, _| registry.pending_consent()[0].clone());
    assert_eq!(held.new_capabilities, ["workspace:read-buffers"]);
    assert!(!loaded(&registry, cx));
    registry.update(cx, |registry, cx| {
        registry.enable("fyx-test-status-item", fs.clone(), cx)
    });
    run_until(cx, "the approved update", |cx| loaded(&registry, cx));
    let check = registry.read_with(cx, |registry, _| registry.capability_check());
    assert!(
        check
            .check("fyx-test-status-item", "workspace:read-buffers")
            .is_ok()
    );
    assert_eq!(
        registry.read_with(cx, |registry, _| registry
            .declared_capabilities("fyx-test-status-item")),
        [
            ("ui:status-bar".to_string(), Tier::T0, true),
            ("workspace:read-buffers".to_string(), Tier::T1, true),
        ]
    );

    // Revoking applies to the next check at once, and the next start asks again.
    registry
        .update(cx, |registry, cx| {
            registry.revoke(
                "fyx-test-status-item",
                "workspace:read-buffers",
                fs.clone(),
                cx,
            )
        })
        .expect("revokes");
    let denied = check
        .check("fyx-test-status-item", "workspace:read-buffers")
        .unwrap_err();
    assert_eq!(denied.reason, "not granted");
    cx.run_until_parked();
    let registry = start(cx);
    run_until(cx, "the request after revoking", |cx| {
        !pending(&registry, cx).is_empty()
    });
}

/// Plan 1.7: the Extension Inspector shows, for the counter demo, its tree, the events and
/// patches, and every call with its duration against the budget (2 s for the first mount, 200 ms
/// for an event).
#[gpui::test]
async fn test_inspector_shows_the_counter_demo(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let extension = load("fyx_demo_counter", counter_wasm(), cx).await;
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let (_, pushed) = futures::channel::mpsc::unbounded();
    registry.update(cx, |registry, cx| {
        registry.register_extension(extension, pushed, cx)
    });
    const LABEL: NodeId = 2;
    const INCREMENT: NodeId = 5;
    let state = registry.read_with(cx, |registry, _| {
        registry
            .surface("fyx_demo_counter/counter")
            .unwrap()
            .state
            .clone()
    });
    run_until(cx, "the mount", |cx| text_of(&state, LABEL, cx).is_some());
    state.update(cx, |state, cx| {
        state.dispatch(UiEvent::Click(INCREMENT), cx)
    });
    run_until(cx, "the increment", |cx| {
        text_of(&state, LABEL, cx).as_deref() == Some("Count: 1")
    });

    let snapshot = cx.update(|cx| crate::inspector::snapshot(registry.read(cx), None, cx));
    assert_eq!(snapshot.surfaces, ["fyx_demo_counter/counter"]);
    assert_eq!(
        snapshot.selected.as_deref(),
        Some("fyx_demo_counter/counter")
    );
    assert!(snapshot.phase.starts_with("ready"), "{}", snapshot.phase);
    assert!(
        snapshot.tree[0].starts_with("#1 flex"),
        "{:?}",
        snapshot.tree
    );
    assert!(
        snapshot
            .tree
            .contains(&r#"  #2 text "Count: 1""#.to_string()),
        "{:?}",
        snapshot.tree
    );
    let has = |lines: &[String], needle: &str| lines.iter().any(|line| line.contains(needle));
    assert!(
        has(&snapshot.history, "event Click(5)"),
        "{:?}",
        snapshot.history
    );
    assert!(
        has(&snapshot.history, "patches: set-props 2 (text)"),
        "{:?}",
        snapshot.history
    );
    assert!(has(&snapshot.history, "mount:"), "{:?}", snapshot.history);
    assert!(
        has(&snapshot.calls, "mount counter:") && has(&snapshot.calls, "ms of 2000 ms"),
        "{:?}",
        snapshot.calls
    );
    assert!(
        has(&snapshot.calls, "handle-event counter:") && has(&snapshot.calls, "ms of 200 ms"),
        "{:?}",
        snapshot.calls
    );

    // The panel renders it.
    let (inspector, vcx) =
        cx.add_window_view(|_, cx| crate::inspector::ExtensionInspector::new(registry.clone(), cx));
    vcx.run_until_parked();
    let rendered = vcx.update(|_, cx| inspector.read(cx).snapshot(cx));
    assert_eq!(rendered, snapshot);
}

/// Plan 1.5: a manifest with an unknown capability, or a T2 one (not available yet), is refused
/// before anything loads, with the capability named.
#[gpui::test]
async fn test_unknown_and_t2_capabilities_are_refused(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    let root = tempfile::tempdir().expect("temp dir");
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    for (capability, expected) in [
        ("ui:pannel", "unknown capability \"ui:pannel\""),
        ("network:fetch", "not available in this version"),
    ] {
        let source = root.path().join(capability.replace(':', "-"));
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(
            source.join("extension.toml"),
            format!(
                "id = \"capabilities\"\nname = \"X\"\nversion = \"0.1.0\"\nschema_version = 1\n\n\
                 [fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"fyx_ui.wasm\"\n\
                 capabilities = [\"ui:panel\", {capability:?}]\n"
            ),
        )
        .expect("manifest");
        std::fs::write(source.join("fyx_ui.wasm"), b"c").expect("component");
        let error = crate::check_dev_folder(&source, fs.as_ref())
            .await
            .expect_err(capability);
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
}

/// integration-3 finding 2: UI extensions load from the installed-extensions directory (any
/// subdirectory with a `[fyx_ui]` table; a regular Zed extension is skipped), reload when their
/// component changes, and stop when they are uninstalled.
#[gpui::test]
async fn test_installed_extensions_load_reload_and_stop(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let wasm = status_item_wasm();
    let root = tempfile::tempdir().expect("temp dir");
    let installed = root.path().join("installed");
    let ui_dir = installed.join("fyx-test-status-item");
    let plain_dir = installed.join("plain");
    std::fs::create_dir_all(&ui_dir).expect("ui dir");
    std::fs::create_dir_all(&plain_dir).expect("plain dir");
    std::fs::copy(status_item_manifest(), ui_dir.join("extension.toml")).expect("manifest");
    std::fs::write(ui_dir.join("fyx_ui.wasm"), &wasm).expect("component");
    std::fs::write(
        plain_dir.join("extension.toml"),
        "id = \"plain\"\nname = \"Plain\"\nversion = \"0.1.0\"\nschema_version = 1\n",
    )
    .expect("plain manifest");

    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let scan = |cx: &mut TestAppContext| {
        registry.update(cx, |registry, cx| {
            registry.scan_installed(installed.clone(), fs.clone(), cx)
        })
    };
    const KEY: &str = "fyx-test-status-item/status";
    const LABEL: NodeId = 3;
    const PING: NodeId = 4;
    let state = |cx: &mut TestAppContext| {
        registry.read_with(cx, |registry, _| {
            registry.surface(KEY).map(|surface| surface.state.clone())
        })
    };

    scan(cx);
    run_until(cx, "the installed extension", |cx| {
        state(cx).is_some_and(|state| text_of(&state, LABEL, cx).is_some())
    });
    assert_eq!(
        registry.read_with(cx, |registry, _| registry.surfaces().len()),
        1,
        "the extension without [fyx_ui] is not a UI extension"
    );
    let surface = state(cx).expect("loaded");
    surface.update(cx, |state, cx| state.dispatch(UiEvent::Click(PING), cx));
    run_until(cx, "a ping", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 1")
    });

    // Nothing changed: a rescan keeps the running instance (and its count).
    scan(cx);
    std::thread::sleep(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(text_of(&surface, LABEL, cx).as_deref(), Some("Pings: 1"));

    // Reinstall: a new component file. The same surface moves to a fresh instance.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(ui_dir.join("fyx_ui.wasm"), &wasm).expect("component");
    scan(cx);
    run_until(cx, "the reload", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 0")
    });
    assert!(
        state(cx).is_some_and(|state| state == surface),
        "the panel or status item keeps its state entity"
    );

    // Uninstall: the surface stops for good.
    std::fs::remove_dir_all(&ui_dir).expect("uninstall");
    scan(cx);
    run_until(cx, "the uninstall", |cx| {
        surface.read_with(cx, |state, _| {
            matches!(
                state.phase(),
                crate::Phase::Failed { message, retryable: false } if message.contains("uninstalled")
            )
        })
    });
}

/// Plan 1.2: a dev folder is checked (a `[fyx_ui]` table, a built component), loads from
/// `fyx.extensions.dev_paths`, loads again on the next start without `FYX_UI_DEV_EXTENSIONS`, and
/// stops when it leaves the list. The setting is read from the user's settings.
#[gpui::test]
async fn test_dev_paths_load_and_survive_restart(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let wasm = status_item_wasm();
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("my-status-item");
    let installed = root.path().join("extensions").join("installed");
    std::fs::create_dir_all(&source).expect("source");
    std::fs::copy(status_item_manifest(), source.join("extension.toml")).expect("manifest");
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());

    // Not built yet.
    let error = crate::check_dev_folder(&source, fs.as_ref())
        .await
        .expect_err("no component");
    assert!(format!("{error:#}").contains("not built"), "{error:#}");
    std::fs::write(source.join("fyx_ui.wasm"), &wasm).expect("component");
    let (id, manifest) = crate::check_dev_folder(&source, fs.as_ref())
        .await
        .expect("a dev folder");
    assert_eq!(id.as_ref(), "fyx-test-status-item");
    // The capabilities shown in the confirmation.
    assert_eq!(
        crate::describe_capabilities(&manifest),
        ["ui:status-bar: add status bar items"]
    );

    // The setting, from the user's settings.
    let setting = format!(
        r#"{{"fyx": {{"extensions": {{"dev_paths": [{:?}]}}}}}}"#,
        source.to_string_lossy()
    );
    cx.update(|cx| {
        gpui::BorrowAppContext::update_global::<settings::SettingsStore, _>(cx, |store, cx| {
            let _ = store.set_user_settings(&setting, cx);
        })
    });
    let dev_paths = cx.update(|cx| crate::install::dev_paths_setting(cx));
    assert_eq!(dev_paths, std::slice::from_ref(&source));

    const KEY: &str = "fyx-test-status-item/status";
    let start = |cx: &mut TestAppContext| {
        let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
        registry.update(cx, |registry, cx| {
            registry.set_dev_paths(dev_paths.clone(), cx);
            registry.scan_installed(installed.clone(), fs.clone(), cx)
        });
        registry
    };
    let registry = start(cx);
    run_until(cx, "the dev extension", |cx| {
        registry.read_with(cx, |registry, _| registry.surface(KEY).is_some())
    });
    // The next start, with only the setting.
    let registry = start(cx);
    run_until(cx, "the dev extension after a restart", |cx| {
        registry.read_with(cx, |registry, _| registry.surface(KEY).is_some())
    });
    let surface = registry.read_with(cx, |registry, _| {
        registry.surface(KEY).unwrap().state.clone()
    });

    // Off the list: its surfaces stop.
    registry.update(cx, |registry, cx| registry.set_dev_paths(Vec::new(), cx));
    run_until(cx, "the removal", |cx| {
        surface.read_with(cx, |state, _| {
            matches!(state.phase(), crate::Phase::Failed { message, .. } if message.contains("uninstalled"))
        })
    });

    // A folder without [fyx_ui] is refused.
    let plain = root.path().join("plain");
    std::fs::create_dir_all(&plain).expect("plain");
    std::fs::write(
        plain.join("extension.toml"),
        "id = \"plain\"\nname = \"Plain\"\nversion = \"0.1.0\"\nschema_version = 1\n",
    )
    .expect("plain manifest");
    let error = crate::check_dev_folder(&plain, fs.as_ref())
        .await
        .expect_err("not a UI extension");
    assert!(format!("{error:#}").contains("[fyx_ui]"), "{error:#}");
}

/// Review integration-4 #2: an id from `extension.toml` is checked before a dev folder is added
/// (ids name folders and settings keys). Dev installs no longer link through `cmd`, so the folder
/// name itself needs no check.
#[gpui::test]
async fn test_dev_folder_ids_are_checked(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    let root = tempfile::tempdir().expect("temp dir");
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    for (index, id) in [
        "x&calc",
        r"x&\\host\s\p.exe",
        "x|calc",
        "x^calc",
        "x%PATH%",
        "../evil",
        r"..\evil",
        "/abs",
        r"C:\abs",
        "..",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        let source = root.path().join(format!("source-{index}"));
        std::fs::create_dir_all(&source).expect("source");
        std::fs::write(
            source.join("extension.toml"),
            format!(
                "id = {id:?}\nname = \"X\"\nversion = \"0.1.0\"\nschema_version = 1\n\n\
                 [fyx_ui]\napi_version = \"0.1.0\"\ncomponent = \"fyx_ui.wasm\"\n\
                 capabilities = [\"ui:panel\"]\n"
            ),
        )
        .expect("manifest");
        std::fs::write(source.join("fyx_ui.wasm"), b"not checked here").expect("component");
        let error = crate::check_dev_folder(&source, fs.as_ref())
            .await
            .expect_err(id);
        assert!(format!("{error:#}").contains("id"), "{id:?}: {error:#}");
    }
}

/// Plan 1.3: Restart extension gives a stopped extension a fresh instance and its surface
/// remounts; a rebuild that does not load keeps the old instance running and shows "Reload
/// failed" with Retry; the next good build loads by itself and clears it.
#[gpui::test]
async fn test_restart_and_failed_reload(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let wasm = status_item_wasm();
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("my-status-item");
    std::fs::create_dir_all(&source).expect("source");
    std::fs::copy(status_item_manifest(), source.join("extension.toml")).expect("manifest");
    std::fs::write(source.join("fyx_ui.wasm"), &wasm).expect("component");
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    registry.update(cx, |registry, cx| {
        registry.set_dev_paths(vec![source.clone()], cx);
        registry.scan_installed(root.path().join("installed"), fs.clone(), cx)
    });
    const ID: &str = "fyx-test-status-item";
    const LABEL: NodeId = 3;
    const PING: NodeId = 4;
    run_until(cx, "the dev extension", |cx| {
        registry.read_with(cx, |registry, _| {
            registry.surface_of(ID, "status").is_some()
        })
    });
    let surface = registry.read_with(cx, |registry, _| {
        registry.surface_of(ID, "status").unwrap().state.clone()
    });
    run_until(cx, "the mount", |cx| text_of(&surface, LABEL, cx).is_some());
    let ping = |cx: &mut TestAppContext, expected: &str| {
        surface.update(cx, |state, cx| state.dispatch(UiEvent::Click(PING), cx));
        run_until(cx, expected, |cx| {
            text_of(&surface, LABEL, cx).as_deref() == Some(expected)
        });
    };
    ping(cx, "Pings: 1");

    // A crash (a guest panic traps the instance) stops it for good, until a restart.
    const CRASH: NodeId = 5;
    surface.update(cx, |state, cx| state.dispatch(UiEvent::Click(CRASH), cx));
    run_until(cx, "the crash", |cx| {
        surface.read_with(cx, |state, _| {
            matches!(
                state.phase(),
                crate::Phase::Failed {
                    retryable: false,
                    ..
                }
            ) && state.can_restart()
        })
    });
    surface.update(cx, |state, cx| state.restart_extension(cx));
    run_until(cx, "the restart", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 0")
    });

    // Real time for the file watcher, test time for the debounce timer.
    let settle = |cx: &mut TestAppContext| {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
    };

    // A broken build: the old instance keeps running, the footer says why.
    ping(cx, "Pings: 1");
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(source.join("fyx_ui.wasm"), b"not a component").expect("broken build");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        settle(cx);
        if surface.read_with(cx, |state, _| state.reload_failed()) {
            break;
        }
    }
    surface.read_with(cx, |state, _| {
        assert!(state.reload_failed(), "no reload error shown");
        assert!(
            state
                .last_error()
                .is_some_and(|error| error.contains("Reload failed")),
            "{:?}",
            state.last_error()
        );
    });
    ping(cx, "Pings: 2");

    // The next good build loads by itself and clears the error.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(source.join("fyx_ui.wasm"), &wasm).expect("fixed build");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        settle(cx);
        if text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 0") {
            break;
        }
    }
    assert_eq!(text_of(&surface, LABEL, cx).as_deref(), Some("Pings: 0"));
    surface.read_with(cx, |state, _| assert!(!state.reload_failed()));
}

/// Hot reload (b): when the component of a dev extension (from `fyx.extensions.dev_paths`) is
/// rebuilt, the extension loads again by itself, after the writes pause, and its surfaces remount;
/// another file changing in the folder does nothing.
#[gpui::test]
async fn test_dev_extension_hot_reloads(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let wasm = status_item_wasm();
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("my-status-item");
    let installed = root.path().join("installed");
    std::fs::create_dir_all(&source).expect("source");
    std::fs::copy(status_item_manifest(), source.join("extension.toml")).expect("manifest");
    std::fs::write(source.join("fyx_ui.wasm"), &wasm).expect("component");
    let fs: std::sync::Arc<dyn fs::Fs> = fs::RealFs::new(None, cx.executor());

    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    registry.update(cx, |registry, cx| {
        registry.set_dev_paths(vec![source.clone()], cx);
        registry.scan_installed(installed.clone(), fs.clone(), cx)
    });
    const LABEL: NodeId = 3;
    const PING: NodeId = 4;
    run_until(cx, "the dev extension", |cx| {
        registry.read_with(cx, |registry, _| {
            registry.surface("fyx-test-status-item/status").is_some()
        })
    });
    let surface = registry.read_with(cx, |registry, _| {
        registry
            .surface("fyx-test-status-item/status")
            .map(|surface| surface.state.clone())
            .expect("loaded")
    });
    run_until(cx, "the mount", |cx| text_of(&surface, LABEL, cx).is_some());
    surface.update(cx, |state, cx| state.dispatch(UiEvent::Click(PING), cx));
    run_until(cx, "a ping", |cx| {
        text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 1")
    });

    // Real time for the file watcher, test time for the debounce timer.
    let settle = |cx: &mut TestAppContext, steps: usize| {
        for _ in 0..steps {
            std::thread::sleep(Duration::from_millis(20));
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
        }
    };

    // Another file in the folder: no reload.
    std::fs::write(source.join("notes.txt"), "unrelated").expect("notes");
    settle(cx, 20);
    assert_eq!(text_of(&surface, LABEL, cx).as_deref(), Some("Pings: 1"));

    // A rebuilt component: the surface remounts on a fresh instance by itself.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(source.join("fyx_ui.wasm"), &wasm).expect("rebuilt component");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        settle(cx, 1);
        if text_of(&surface, LABEL, cx).as_deref() == Some("Pings: 0") {
            return;
        }
    }
    panic!(
        "the dev extension did not reload; label is {:?}",
        text_of(&surface, LABEL, cx)
    );
}
