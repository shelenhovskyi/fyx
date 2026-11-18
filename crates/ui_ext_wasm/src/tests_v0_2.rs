//! The 0.2 host against `fixtures/probe_v0_2`, a guest that runs a script of host calls and
//! reports each result (see its crate doc). Every test loads its own instance.

use std::path::Path;

use gpui::TestAppContext;
use ui_ext::v0_2::{
    ButtonProps, DecorationLocation, DecorationPlacement, DismissReason, DockPosition, Icon,
    IconRef, InputProps, ListProps, ModalDescriptor, ModalWidth, Node, PanelDescriptor, Patch,
    Props, StatusBarSide, StatusItemDescriptor, Surface, SurfaceKind, UiEvent, Update,
};

use super::*;

const SURFACE: &str = "probe";
/// `input-changed` on this node stores the script; on any other node it runs it.
const SET_SCRIPT: ui_ext::NodeId = 100;
const RUN: ui_ext::NodeId = 7;

fn probe_v0_2_wasm() -> Vec<u8> {
    crate::test_support::fixture_wasm("fyx_ui_test_probe_v0_2")
}

async fn load_with(options: LoadOptions, cx: &mut TestAppContext) -> Result<UiExtension> {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    cx.update(|cx| {
        UiExtension::load_with_options(
            "probe-v0-2".into(),
            probe_v0_2_wasm(),
            options,
            &cx.to_async(),
        )
    })
    .await
    .map(|(extension, _pushed)| extension)
}

async fn load(cx: &mut TestAppContext) -> UiExtension {
    load_with(
        LoadOptions {
            allow_staged: Some(true),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .unwrap_or_else(|error| panic!("the 0.2 probe failed to load: {error:#}"))
}

fn panel(id: &str, icon: Icon) -> Surface {
    Surface {
        id: id.into(),
        title: id.into(),
        kind: SurfaceKind::Panel(PanelDescriptor {
            position: DockPosition::Right,
            icon: IconRef::Frozen(icon),
            default_size: 300,
        }),
    }
}

fn status(id: &str) -> Surface {
    Surface {
        id: id.into(),
        title: id.into(),
        kind: SurfaceKind::StatusItem(StatusItemDescriptor {
            side: StatusBarSide::Right,
            priority: 0,
        }),
    }
}

fn modal(id: &str) -> Surface {
    Surface {
        id: id.into(),
        title: id.into(),
        kind: SurfaceKind::Modal(ModalDescriptor {
            width: ModalWidth::Medium,
        }),
    }
}

fn push(surface_id: &str, update: Update) -> HostEffect {
    HostEffect::Push {
        surface_id: surface_id.into(),
        update,
    }
}

/// The probe's report in a reply: the text of node 1 of a `replace`.
fn report(update: &Update) -> String {
    match update {
        Update::Replace(nodes) => match &nodes[0].props {
            Props::Text(text) => text.content.clone(),
            other => panic!("not a report: {other:?}"),
        },
        other => panic!("not a report: {other:?}"),
    }
}

/// The results of the script, without the event the report starts with.
fn results(report: &str) -> Vec<String> {
    report.split(" | ").skip(1).map(String::from).collect()
}

async fn set_script(extension: &UiExtension, script: &str) {
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(SET_SCRIPT, script.into()))
        .await
        .expect("stores the script");
    assert_eq!(reply.value, Update::Unchanged);
    assert!(reply.effects.is_empty());
}

/// Runs `script` from `event` on `surface_id`; returns its results and effects.
async fn run_on(
    extension: &UiExtension,
    surface_id: &str,
    event: UiEvent,
    script: &str,
) -> (Vec<String>, Vec<HostEffect>) {
    set_script(extension, script).await;
    let reply = extension
        .handle_event_v0_2(surface_id, event.clone())
        .await
        .unwrap_or_else(|error| panic!("{event:?} running {script:?}: {error:#}"));
    (results(&report(&reply.value)), reply.effects)
}

/// Runs `script` from typing, which is not a gesture.
async fn run(extension: &UiExtension, script: &str) -> (Vec<String>, Vec<HostEffect>) {
    run_on(
        extension,
        SURFACE,
        UiEvent::InputChanged(RUN, "x".into()),
        script,
    )
    .await
}

async fn run_from_tick(extension: &UiExtension, script: &str) -> (Vec<String>, Vec<HostEffect>) {
    set_script(extension, script).await;
    let reply = extension.tick(SURFACE).await.expect("tick");
    let report = report(&reply.value);
    assert!(report.starts_with("tick probe"), "{report}");
    (results(&report), reply.effects)
}

async fn click(extension: &UiExtension, script: &str) -> (Vec<String>, Vec<HostEffect>) {
    run_on(extension, SURFACE, UiEvent::Click(RUN), script).await
}

#[gpui::test]
async fn test_v0_2_probe_loads_as_v0_2(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    assert_eq!(extension.api_version(), &Version::new(0, 2, 0));
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_2_0);
    assert_eq!(
        extension.capabilities().len(),
        5,
        "no manifest: every capability"
    );
    assert_eq!(extension.surfaces_v0_2(), &[panel(SURFACE, Icon::Box)]);
    // The 0.1 view the renderer uses until plan 2.3.
    assert_eq!(
        extension.surfaces(),
        &[ui_ext::Surface {
            id: SURFACE.into(),
            title: SURFACE.into(),
            kind: ui_ext::SurfaceKind::Panel(ui_ext::PanelDescriptor {
                position: ui_ext::DockPosition::Right,
                icon: "box".into(),
                default_size: 300,
            }),
        }]
    );
    // `add-surface` during `surfaces()` is applied after the static surfaces.
    assert_eq!(
        extension.initial_effects(),
        &[HostEffect::AddSurface(status("boot"))]
    );
}

#[gpui::test]
async fn test_v0_2_is_staged(cx: &mut TestAppContext) {
    let error = load_with(
        LoadOptions {
            allow_staged: Some(false),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .err()
    .expect("Stable and Preview stay on 0.1");
    let message = format!("{error:#}");
    assert!(
        message.contains("targets fyx:ui 0.2.0, which is staged"),
        "{message}"
    );
}

#[gpui::test]
async fn test_v0_2_capabilities_come_from_the_manifest(cx: &mut TestAppContext) {
    let error = load_with(
        LoadOptions {
            capabilities: Some(Vec::new()),
            allow_staged: Some(true),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .err()
    .expect("the static panel needs ui:panel");
    assert!(
        format!("{error:#}").contains("needs \"ui:panel\""),
        "{error:#}"
    );

    let extension = load_with(
        LoadOptions {
            capabilities: Some(vec![CAPABILITY_PANEL.into()]),
            allow_staged: Some(true),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .expect("loads with ui:panel");
    // `add-surface("boot")` was refused, so nothing is left to apply.
    assert_eq!(extension.initial_effects(), &[]);
    let (results, effects) = run(
        &extension,
        "add modal picker; add status s; add panel p; place d src/a.rs 1; unplace d",
    )
    .await;
    assert_eq!(
        results,
        [
            "not-permitted",
            "not-permitted",
            "ok",
            "not-permitted",
            "not-permitted"
        ]
    );
    assert_eq!(effects, [HostEffect::AddSurface(panel("p", Icon::Box))]);
}

#[gpui::test]
async fn test_v0_2_load_from_dir(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let fs = fs::FakeFs::new(cx.executor());
    fs.create_dir(Path::new("/ext/probe")).await.expect("dir");
    fs.insert_file(
        "/ext/probe/extension.toml",
        b"id = \"probe-v0-2\"\nname = \"Probe\"\nversion = \"0.2.0\"\nschema_version = 1\n\
          [fyx_ui]\napi_version = \"0.2.0\"\ncomponent = \"fyx_ui.wasm\"\n\
          capabilities = [\"ui:panel\", \"ui:modal\"]\n"
            .to_vec(),
    )
    .await;
    fs.insert_file("/ext/probe/fyx_ui.wasm", probe_v0_2_wasm())
        .await;
    let (extension, _pushed) = cx
        .update(|cx| {
            let cx = cx.to_async();
            async move {
                UiExtension::load_from_dir_with_options(
                    Path::new("/ext/probe"),
                    LoadOptions {
                        allow_staged: Some(true),
                        ..LoadOptions::default()
                    },
                    fs,
                    &cx,
                )
                .await
            }
        })
        .await
        .expect("loads")
        .expect("has a [fyx_ui] table");
    assert_eq!(extension.capabilities(), ["ui:panel", "ui:modal"]);
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_2_0);
}

#[gpui::test]
async fn test_v0_2_mount_and_rows_carry_the_new_props(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let nodes = extension.mount_v0_2(SURFACE).await.expect("mount").value;
    let props = |id: u32| {
        nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.props.clone())
            .unwrap_or_else(|| panic!("no node {id}"))
    };
    assert_eq!(
        props(2),
        Props::Button(ButtonProps {
            label: "Accept".into(),
            icon: Some(IconRef::Frozen(Icon::CheckDouble)),
            style: ui_ext::ButtonStyle::Filled,
            disabled: false,
            tooltip: Some("Accept every hunk".into()),
        })
    );
    assert_eq!(
        props(3),
        Props::Input(InputProps {
            value: "hunter2".into(),
            placeholder: "Token".into(),
            secret: true,
        })
    );
    assert_eq!(
        props(4),
        Props::List(ListProps {
            row_count: 10,
            row_height: 22,
            selected: Some(2),
        })
    );
    assert!(matches!(
        props(5),
        Props::Icon(icon) if icon.name == IconRef::Frozen(Icon::Spinner)
    ));

    // The 0.1 view: the same tree, without what 0.1 cannot express.
    let tree = extension.mount(SURFACE).await.expect("mount").value;
    assert_eq!(tree.children(Some(1)), &[2, 3, 4, 5]);
    match tree.get(2).map(ui_ext::TreeNode::props) {
        Some(ui_ext::Props::Button(button)) => {
            assert_eq!(button.icon.as_deref(), Some("check_double"))
        }
        other => panic!("{other:?}"),
    }

    let rows = extension
        .list_rows_v0_2(SURFACE, 4, 30, 5)
        .await
        .expect("rows")
        .value;
    assert_eq!(
        rows.iter()
            .map(|row| (row.index, row.indent))
            .collect::<Vec<_>>(),
        [(30, 30), (31, 31), (32, 32), (33, 33), (34, 34)]
    );
    // The host passes the indent on; 32 levels is what is drawn.
    assert_eq!(rows[4].indent_levels(), 32);
}

#[gpui::test]
async fn test_v0_2_push_update_returns_a_result(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let (results, effects) = run(
        &extension,
        "push probe remove 1; push nope unchanged; push probe patches 2049; \
         push probe patches 2048; push probe replace 16385; push probe replace 16384; \
         push probe invalidate 4",
    )
    .await;
    assert_eq!(
        results,
        [
            "ok",
            "unknown-surface",
            "limit-exceeded",
            "ok",
            "limit-exceeded",
            "ok",
            "ok"
        ]
    );
    assert_eq!(effects.len(), 4);
    assert_eq!(
        effects[0],
        push(SURFACE, Update::Patches(vec![Patch::Remove(1)]))
    );
    assert!(
        matches!(&effects[2], HostEffect::Push { update: Update::Replace(nodes), .. } if nodes.len() == 16384)
    );
    assert_eq!(
        effects[3],
        push(
            SURFACE,
            Update::Patches(vec![Patch::InvalidateRows(
                ui_ext::v0_2::InvalidateRowsOp {
                    target: 4,
                    rows: Some(ui_ext::v0_2::RowRange { start: 2, count: 3 }),
                }
            )])
        )
    );

    // The 65th push of a call is refused, not a trap: the extension stays enabled.
    let (results, effects) = run(&extension, "repeat 70 push probe unchanged").await;
    assert_eq!(results, ["ok*64,limit-exceeded*6"]);
    assert_eq!(effects.len(), MAX_PUSHES_PER_CALL);
    let (results, _) = run(&extension, "push probe unchanged").await;
    assert_eq!(results, ["ok"]);
}

#[gpui::test]
async fn test_v0_2_effects_are_checked_in_call_order(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let (results, effects) = run(
        &extension,
        "push probe unchanged; remove probe; push probe unchanged; add panel extra; \
         push extra unchanged; remove probe",
    )
    .await;
    assert_eq!(
        results,
        ["ok", "ok", "unknown-surface", "ok", "ok", "unknown-surface"]
    );
    assert_eq!(
        effects,
        [
            push(SURFACE, Update::Unchanged),
            HostEffect::RemoveSurface {
                surface_id: SURFACE.into()
            },
            HostEffect::AddSurface(panel("extra", Icon::Box)),
            push("extra", Update::Unchanged),
        ]
    );

    // A removed id can be added again, as a new surface.
    let (results, _) = run(&extension, "add panel probe; push probe unchanged").await;
    assert_eq!(results, ["ok", "ok"]);

    // A call that fails after its effects carries them in the error.
    set_script(&extension, "push probe unchanged; fail").await;
    let error = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(RUN, "x".into()))
        .await
        .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<EffectReplyError>()
            .map(|error| &error.effects),
        Some(&vec![push(SURFACE, Update::Unchanged)])
    );
    // The 0.1 view of the same error carries the push as a `ReplyError`.
    let error = extension
        .handle_event(SURFACE, ui_ext::UiEvent::InputChanged(RUN, "x".into()))
        .await
        .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<ReplyError>()
            .map(|error| &error.pushed),
        Some(&vec![PushedUpdate {
            surface_id: SURFACE.into(),
            update: ui_ext::Update::Unchanged,
        }])
    );
}

#[gpui::test]
async fn test_v0_2_add_surface_checks(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let (results, effects) = run(
        &extension,
        "add panel Bad; add panel probe; add panel boot; add modal picker",
    )
    .await;
    assert_eq!(
        results,
        [
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "ok"
        ]
    );
    assert_eq!(effects, [HostEffect::AddSurface(modal("picker"))]);

    // `probe`, `boot` and `picker` are live: 29 more make 32.
    let (results, effects) = run(&extension, "repeat 30 add status s#").await;
    assert_eq!(results, ["ok*29,limit-exceeded*1"]);
    assert_eq!(effects.len(), 29);
}

#[gpui::test]
async fn test_v0_2_request_tick(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let (results, effects) = run(
        &extension,
        "tick nope 100; tick probe 86400001; tick probe 86400000; tick probe 10; \
         tick probe 500; cancel probe; cancel nope; add modal picker; tick picker 100",
    )
    .await;
    assert_eq!(
        results,
        [
            "unknown-surface",
            "invalid-argument",
            "ok",
            "ok",
            "ok",
            "done",
            "done",
            "ok",
            "invalid-argument",
        ]
    );
    let tick = |delay: Duration| HostEffect::RequestTick {
        surface_id: SURFACE.into(),
        delay,
    };
    assert_eq!(
        effects,
        [
            tick(MAX_TICK_DELAY),
            tick(MIN_TICK_DELAY),
            tick(Duration::from_millis(500)),
            HostEffect::CancelTick {
                surface_id: SURFACE.into()
            },
            HostEffect::AddSurface(modal("picker")),
        ]
    );

    // The tick export runs and its effects come back with it; a tick may re-request.
    let (results, effects) = run_from_tick(&extension, "tick probe 1000").await;
    assert_eq!(results, ["ok"]);
    assert_eq!(effects, [tick(Duration::from_secs(1))]);
}

/// A tick that blocks is cut off by the budget and disables the extension, like any call.
#[gpui::test]
async fn test_v0_2_blocking_tick_disables_extension(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    set_script(&extension, "sleep 1000").await;
    let error = extension.tick(SURFACE).await.unwrap_err();
    assert!(format!("{error:#}").contains("time budget"), "{error:#}");
    let error = extension.tick(SURFACE).await.unwrap_err();
    assert!(format!("{error:#}").contains("disabled"), "{error:#}");
}

#[gpui::test]
async fn test_v0_2_show_surface_needs_a_click_or_enter(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let show = HostEffect::ShowSurface {
        surface_id: "picker".into(),
    };
    let hide = HostEffect::HideSurface {
        surface_id: "picker".into(),
    };
    run(&extension, "add modal picker").await;

    // No gesture: typing, keyboard selection, a dismissal, a tick, list-rows.
    assert_eq!(run(&extension, "show picker").await.0, ["not-permitted"]);
    let (results, effects) = run_on(
        &extension,
        SURFACE,
        UiEvent::RowSelected(4, 1),
        "show picker",
    )
    .await;
    assert_eq!(results, ["not-permitted"]);
    assert!(effects.is_empty());
    // The reply to `dismissed` is forced to `unchanged`, so only the effects tell.
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::Dismissed(DismissReason::Escape))
        .await
        .expect("dismissed");
    assert_eq!(reply.value, Update::Unchanged);
    assert!(reply.effects.is_empty());
    assert_eq!(
        run_from_tick(&extension, "show picker").await.0,
        ["not-permitted"]
    );
    set_script(&extension, "show picker").await;
    let rows = extension
        .list_rows_v0_2(SURFACE, 4, 0, 1)
        .await
        .expect("rows");
    match &rows.value[0].nodes[0].props {
        Props::Text(text) => assert!(text.content.ends_with("not-permitted"), "{}", text.content),
        other => panic!("{other:?}"),
    }

    // A click: shown; showing it again while it is shown does nothing.
    let (results, effects) = click(&extension, "show picker; show picker").await;
    assert_eq!(results, ["ok", "ok"]);
    assert_eq!(effects, std::slice::from_ref(&show));
    // Hidden by the extension: no gesture needed, and hiding twice does nothing.
    let (results, effects) = run(&extension, "hide picker; hide picker").await;
    assert_eq!(results, ["ok", "ok"]);
    assert_eq!(effects, std::slice::from_ref(&hide));
    // The click's activation was used up.
    assert_eq!(
        run_from_tick(&extension, "show picker").await.0,
        ["not-permitted"]
    );

    // A click that does not show anything leaves an activation for a tick within 5 s, once.
    click(&extension, "").await;
    let (results, effects) = run_from_tick(&extension, "show picker").await;
    assert_eq!(
        (results, effects),
        (vec!["ok".to_string()], vec![show.clone()])
    );
    run(&extension, "hide picker").await;
    assert_eq!(
        run_from_tick(&extension, "show picker").await.0,
        ["not-permitted"]
    );

    // `row-click` and `input-submitted` are gestures too.
    for event in [
        UiEvent::RowClick(4, 2),
        UiEvent::InputSubmitted(RUN, "query".into()),
    ] {
        let (results, effects) = run_on(&extension, SURFACE, event, "show picker").await;
        assert_eq!(
            (results, effects),
            (vec!["ok".to_string()], vec![show.clone()])
        );
        run(&extension, "hide picker").await;
    }

    // The activation expires.
    extension
        .with_state(|state| state.api.set_activation_window(Duration::ZERO))
        .await;
    click(&extension, "").await;
    assert_eq!(
        run_from_tick(&extension, "show picker").await.0,
        ["not-permitted"]
    );
}

#[gpui::test]
async fn test_v0_2_dismissed_and_closed_modals(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    run(&extension, "add modal picker").await;
    let (results, _) = click(&extension, "show picker; tick picker 100").await;
    assert_eq!(results, ["ok", "ok"], "a shown modal may tick");

    // The user closed it: the reply must be `unchanged`, but its effects apply.
    let (results, effects) = {
        set_script(&extension, "push probe unchanged; tick picker 100").await;
        let reply = extension
            .handle_event_v0_2("picker", UiEvent::Dismissed(DismissReason::FocusLost))
            .await
            .expect("dismissed");
        assert_eq!(reply.value, Update::Unchanged, "the probe answered replace");
        (Vec::<String>::new(), reply.effects)
    };
    assert!(results.is_empty());
    assert_eq!(
        effects,
        [push(SURFACE, Update::Unchanged)],
        "no tick for a closed modal"
    );

    // Shown again after a click; the renderer drops the show (no window) and says so.
    let show = HostEffect::ShowSurface {
        surface_id: "picker".into(),
    };
    assert_eq!(
        click(&extension, "show picker").await.1,
        std::slice::from_ref(&show)
    );
    extension.modal_closed("picker");
    assert_eq!(click(&extension, "show picker").await.1, [show]);

    // Panels show and hide without a gesture; status items and decorations cannot.
    let (results, effects) = run(
        &extension,
        "show probe; hide probe; show boot; hide boot; place d src/a.rs 1; show d; hide d; \
         show nope",
    )
    .await;
    assert_eq!(
        results,
        [
            "ok",
            "ok",
            "invalid-argument",
            "invalid-argument",
            "ok",
            "invalid-argument",
            "invalid-argument",
            "unknown-surface",
        ]
    );
    assert_eq!(
        effects[..2],
        [
            HostEffect::ShowSurface {
                surface_id: SURFACE.into()
            },
            HostEffect::HideSurface {
                surface_id: SURFACE.into()
            },
        ]
    );
}

#[gpui::test]
async fn test_v0_2_decorations(cx: &mut TestAppContext) {
    let extension = load(cx).await;

    let (results, effects) = run(
        &extension,
        "place probe src/a.rs 1; place Bad src/a.rs 1; place d1 ../x.rs 1; place d1 /abs.rs 1; \
         place d1 src\\a.rs 1; place d1 src/a.rs 1 16385; place d1 src/a.rs 3; \
         place d1 src/b.rs 4; push d1 unchanged; tick d1 100; show d1; remove d1; add panel d1; \
         unplace d1; unplace d1; push d1 unchanged",
    )
    .await;
    assert_eq!(
        results,
        [
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "limit-exceeded",
            "ok",
            "ok",
            "ok",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "invalid-argument",
            "ok",
            "unknown-surface",
            "unknown-surface",
        ]
    );
    let place = |path: &str, line: u32| HostEffect::PlaceDecoration {
        id: "d1".into(),
        location: DecorationLocation {
            path: path.into(),
            line,
            placement: DecorationPlacement::Below,
            height_lines: Some(2),
        },
        nodes: vec![Node {
            id: 0,
            parent: None,
            props: Props::Text(ui_ext::TextProps {
                content: "note".into(),
                color: None,
                bold: false,
                monospace: false,
            }),
        }],
    };
    assert_eq!(
        effects,
        [
            place("src/a.rs", 3),
            place("src/b.rs", 4),
            push("d1", Update::Unchanged),
            HostEffect::RemoveDecoration { id: "d1".into() },
        ]
    );

    // At most 512 live decorations.
    let (results, _) = run(&extension, "repeat 300 place a# src/a.rs 1").await;
    assert_eq!(results, ["ok*300"]);
    let (results, _) = run(&extension, "repeat 300 place b# src/a.rs 1").await;
    assert_eq!(results, ["ok*212,limit-exceeded*88"]);
}

#[gpui::test]
async fn test_v0_2_decoration_effects_per_call(cx: &mut TestAppContext) {
    let extension = load(cx).await;
    let (results, effects) = run(
        &extension,
        "repeat 300 place a# src/a.rs 1; repeat 212 unplace a#; place z src/a.rs 1; unplace a299",
    )
    .await;
    assert_eq!(
        results,
        ["ok*300", "ok*212", "limit-exceeded", "limit-exceeded"]
    );
    assert_eq!(effects.len(), ui_ext::MAX_DECORATION_EFFECTS_PER_CALL);
    // The refused placement left no live decoration behind; the refused removal kept a299.
    let (results, _) = run(&extension, "unplace z; unplace a299").await;
    assert_eq!(results, ["unknown-surface", "ok"]);
}

#[gpui::test]
async fn test_v0_2_version_falls_back_to_imports(cx: &mut TestAppContext) {
    let mut wasm = probe_v0_2_wasm();
    let name = API_VERSION_SECTION.as_bytes();
    let offset = wasm
        .windows(name.len())
        .position(|window| window == name)
        .expect("the probe has a version section");
    wasm[offset + name.len() - 1] = b'N';
    assert_eq!(parse_api_version("probe", &wasm).expect("parses"), None);

    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let (extension, _) = cx
        .update(|cx| {
            UiExtension::load_with_options(
                "probe-v0-2".into(),
                wasm,
                LoadOptions {
                    allow_staged: Some(true),
                    ..LoadOptions::default()
                },
                &cx.to_async(),
            )
        })
        .await
        .expect("loads");
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_2_0);
}

/// A 0.1 component in the 0.2 API: never ticked, and never sent 0.2-only events.
#[gpui::test]
async fn test_v0_1_component_in_the_v0_2_api(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let (extension, _) = cx
        .update(|cx| {
            UiExtension::load(
                "counter".into(),
                crate::test_support::demo_wasm("fyx_demo_counter"),
                &cx.to_async(),
            )
        })
        .await
        .expect("loads");
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_1_0);
    assert_eq!(
        extension.surfaces_v0_2()[0].kind,
        SurfaceKind::Panel(PanelDescriptor {
            position: DockPosition::Right,
            icon: IconRef::Upstream("hash".into()),
            default_size: 320,
        })
    );
    assert_eq!(
        extension.tick("counter").await.expect("tick").value,
        Update::Unchanged
    );
    let reply = extension
        .handle_event_v0_2("counter", UiEvent::Dismissed(DismissReason::Escape))
        .await
        .expect("dropped, not sent");
    assert_eq!(reply.value, Update::Unchanged);
    // Mount and increment, through the 0.2 API.
    let nodes = extension.mount_v0_2("counter").await.expect("mount").value;
    assert_eq!(nodes.len(), 7);
    let reply = extension
        .handle_event_v0_2("counter", UiEvent::Click(5))
        .await
        .expect("click");
    assert!(
        matches!(reply.value, Update::Patches(_)),
        "{:?}",
        reply.value
    );
}

#[test]
fn test_frozen_icons_match_the_wit() {
    let mapped: Vec<Icon> = fyx_ui_v0_2::WIT_ICONS
        .iter()
        .map(|&icon| icon.into())
        .collect();
    assert_eq!(mapped, Icon::ALL);
    assert_eq!(mapped.len(), 101);
}

/// The broker is asked at every gated call, so a revocation applies to the next one.
#[gpui::test]
async fn test_v0_2_broker_is_asked_at_every_call(cx: &mut TestAppContext) {
    struct Broker(std::sync::Mutex<Vec<String>>);
    impl permissions::CapabilityCheck for Broker {
        fn check(
            &self,
            extension_id: &str,
            capability: &str,
        ) -> std::result::Result<(), permissions::CapabilityDenied> {
            if self
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|revoked| revoked == capability)
            {
                Err(permissions::CapabilityDenied {
                    extension_id: extension_id.into(),
                    capability: capability.into(),
                    reason: "revoked".into(),
                })
            } else {
                Ok(())
            }
        }
    }
    let broker = Arc::new(Broker(Default::default()));
    let extension = load_with(
        LoadOptions {
            capability_check: Some(broker.clone()),
            allow_staged: Some(true),
            ..LoadOptions::default()
        },
        cx,
    )
    .await
    .expect("loads");

    let script = "place d src/a.rs 1; add modal picker; show picker";
    let (results, _) = click(&extension, script).await;
    assert_eq!(results, ["ok", "ok", "ok"]);
    run(&extension, "hide picker; unplace d; remove picker").await;

    broker
        .0
        .lock()
        .unwrap()
        .extend(["ui:editor-decorations".into(), "ui:modal".into()]);
    let (results, effects) = click(&extension, script).await;
    assert_eq!(
        results,
        ["not-permitted", "not-permitted", "unknown-surface"]
    );
    assert!(effects.is_empty());
    let checks = extension.diagnostics();
    assert!(format!("{checks:?}").contains("revoked"), "{checks:?}");
}

/// async-1 #5: [`with_budget`] counts only time the call waits on the host. A call that is ready
/// but not polled (the tokio workers are busy with other work) spends none of its budget.
#[test]
fn test_budget_ignores_time_spent_waiting_for_a_worker() {
    use std::task::{Context, Poll};

    let budget = Duration::from_millis(100);
    let polls = std::sync::atomic::AtomicUsize::new(0);
    let mut call = Box::pin(with_budget(
        budget,
        futures::future::poll_fn(|cx| {
            if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                // Ready to run on, as a guest that yields at an epoch is.
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }),
    ));
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    assert!(call.as_mut().poll(&mut cx).is_pending());
    // Not polled for three times the budget, as on a starved worker.
    std::thread::sleep(budget * 3);
    assert!(matches!(call.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));

    // A call that waits on the host and is not woken is cut off once the budget is spent.
    let mut blocked = Box::pin(with_budget(
        budget,
        futures::future::pending::<Result<()>>(),
    ));
    let started = Instant::now();
    let error = loop {
        if let Poll::Ready(result) = blocked.as_mut().poll(&mut cx) {
            break result.unwrap_err();
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(started.elapsed() >= budget, "{:?}", started.elapsed());
    assert!(
        format!("{error:#}").contains("blocked in a host call"),
        "{error:#}"
    );
}
