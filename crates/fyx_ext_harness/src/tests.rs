use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ui_ext::{InsertOp, Patch, UiEvent, Update};

use super::*;

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds a guest for `wasm32-wasip2` with `cargo build <args>` in `dir`, into the target directory
/// `target` next to the test binary's own, and returns `<target>/wasm32-wasip2/<output>`. Builds
/// that differ only in features need targets of their own, or they overwrite each other's output.
fn build_guest(
    dir: &Path,
    target: &str,
    args: &[&str],
    output: &str,
) -> std::result::Result<PathBuf, String> {
    // <target>/debug/deps/<test binary>
    let target_dir = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .ancestors()
        .nth(3)
        .ok_or("the test binary is not inside a cargo target directory")?
        .join(target);
    // Blocking is fine: it runs once per guest and test process, before any test work.
    #[allow(clippy::disallowed_methods)]
    let status = std::process::Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .arg("build")
        .args(args)
        .args(["--locked", "--target", "wasm32-wasip2", "--target-dir"])
        .arg(&target_dir)
        .current_dir(dir)
        .status()
        .map_err(|error| format!("running cargo: {error}"))?;
    if !status.success() {
        return Err(format!("cargo build {args:?} in {dir:?} failed: {status}"));
    }
    Ok(target_dir.join("wasm32-wasip2").join(output))
}

fn read_guest(
    built: &'static OnceLock<std::result::Result<PathBuf, String>>,
    build: fn() -> std::result::Result<PathBuf, String>,
) -> Vec<u8> {
    let path = built
        .get_or_init(build)
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

/// A demo's default (0.1) build, or with `v0_2` its `fyx-ui-0-2` build.
fn demo(
    crate_name: &'static str,
    v0_2: bool,
    built: &'static OnceLock<std::result::Result<PathBuf, String>>,
) -> Vec<u8> {
    let path = built
        .get_or_init(|| {
            let feature = format!("{crate_name}/fyx-ui-0-2");
            let mut args = vec!["-p", crate_name];
            let target = if v0_2 {
                args.extend(["--features", &feature]);
                "fyx-ui-fixtures-v0_2"
            } else {
                "fyx-ui-fixtures"
            };
            build_guest(
                &workspace_dir(),
                target,
                &args,
                &format!("debug/{crate_name}.wasm"),
            )
        })
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn counter_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_counter", false, &BUILT)
}

fn counter_v0_2_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_counter", true, &BUILT)
}

fn todo_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_todo", false, &BUILT)
}

/// `fixtures/v0_2`, a 0.2 guest written with the SDK; its doc comment lists the node ids.
fn fixture_v0_2_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    read_guest(&BUILT, || {
        build_guest(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
            "fyx-ui-fixtures",
            &["--release"],
            "release/fyx_harness_fixture_v0_2.wasm",
        )
    })
}

fn probe_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    read_guest(&BUILT, || {
        build_guest(
            &workspace_dir().join("crates/ui_ext_wasm/fixtures/probe"),
            "fyx-ui-fixtures",
            &["--release"],
            "release/fyx_ui_test_probe.wasm",
        )
    })
}

/// Node ids of `fixtures/probe` in `ui_ext_wasm`.
mod probe {
    use ui_ext::NodeId;

    pub const SURFACE: &str = "probe";
    pub const PUSH: NodeId = 1;
    pub const SPIN: NodeId = 2;
    pub const PUSH_THEN_PATCH: NodeId = 4;
    pub const PUSH_THEN_FAIL: NodeId = 5;
    pub const PUSH_FLOOD: NodeId = 6;
}

#[test]
fn counter_follows_its_contract() {
    let mut harness = Harness::load(&counter_wasm()).expect("load");
    assert_eq!(harness.api_version(), &Version::new(0, 1, 0));
    assert_eq!(
        harness
            .surfaces()
            .iter()
            .map(|surface| surface.id.as_str())
            .collect::<Vec<_>>(),
        ["counter"]
    );
    assert!(
        harness.event("counter", UiEvent::Click(5)).is_err(),
        "not mounted yet"
    );
    assert!(harness.mount("nope").is_err());

    let ids = harness
        .mount("counter")
        .expect("mount")
        .to_nodes()
        .iter()
        .map(|node| node.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(harness.text("counter", 2), Some("Count: 0"));

    let outcome = harness.click("counter", "Increment").expect("click");
    assert!(outcome.pushed.is_empty());
    assert!(
        outcome.touched.contains(&2) && outcome.touched.contains(&7),
        "{outcome:?}"
    );
    assert_eq!(harness.text("counter", 2), Some("Count: 1"));
    assert_eq!(harness.find_text("counter", "Count: 1"), Some(2));

    let rows = harness.rows("counter", 7, 0, 10).expect("rows");
    assert_eq!(rows.len(), 1);
    let (index, row) = &rows[0];
    assert_eq!(*index, 0);
    assert!(
        row.to_nodes().iter().any(|node| matches!(
            &node.props,
            Props::Text(text) if text.content == "Row 1 of 1"
        )),
        "{row:?}"
    );

    let remounted = harness.mount("counter").expect("remount");
    assert_eq!(remounted.roots(), [8]);
    assert!(harness.disabled().is_none());
}

#[test]
fn typed_text_is_written_before_the_event() {
    let mut harness = Harness::load(&todo_wasm()).expect("load");
    harness.mount("todos").expect("mount");
    let input = harness
        .find("todos", |props| matches!(props, Props::Input(_)))
        .expect("the filter input");
    harness
        .event("todos", UiEvent::InputChanged(input, "main".into()))
        .expect("typing");
    match harness
        .tree("todos")
        .and_then(|tree| tree.get(input))
        .map(TreeNode::props)
    {
        Some(Props::Input(props)) => assert_eq!(props.value, "main"),
        other => panic!("the input changed kind: {other:?}"),
    }
    assert!(
        harness
            .find("todos", |props| matches!(
                props,
                Props::Text(text) if text.content.starts_with("2 of")
            ))
            .is_some(),
        "the filter applied"
    );
}

#[test]
fn pushes_come_before_the_reply_and_failures_keep_them() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    assert!(harness.surfaces().is_empty());

    let reply = harness
        .handle_event_raw(probe::SURFACE, UiEvent::Click(probe::PUSH_THEN_PATCH))
        .expect("call");
    assert_eq!(reply.pushed.len(), 1);
    assert!(matches!(
        &reply.pushed[0].update,
        Update::Patches(patches) if matches!(&patches[..], [Patch::Insert(InsertOp { node, .. })] if node.id == 10)
    ));
    assert!(matches!(
        &reply.value,
        Ok(Update::Patches(patches)) if matches!(&patches[..], [Patch::Insert(InsertOp { node, .. })] if node.id == 11)
    ));

    let reply = harness
        .handle_event_raw(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .expect("call");
    assert_eq!(reply.value, Ok(Update::Unchanged));
    assert!(
        harness
            .logs()
            .iter()
            .any(|(level, message)| *level == ui_ext::LogLevel::Info
                && message == "pushing an update")
    );

    let reply = harness
        .handle_event_raw(probe::SURFACE, UiEvent::Click(probe::PUSH_THEN_FAIL))
        .expect("call");
    assert_eq!(reply.value, Err("failed after pushing".to_string()));
    assert_eq!(reply.pushed.len(), 1);
    assert_eq!(harness.pushed().len(), 3);
    assert!(
        harness.disabled().is_none(),
        "a guest error does not disable"
    );
}

#[test]
fn a_push_flood_traps_and_disables() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    let error = harness
        .handle_event_raw(probe::SURFACE, UiEvent::Click(probe::PUSH_FLOOD))
        .expect_err("the 65th push traps");
    assert!(
        format!("{error:#}").contains("pushed more than 64"),
        "{error:#}"
    );
    assert!(harness.disabled().is_some());
    let error = harness
        .handle_event_raw(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .expect_err("disabled");
    assert!(error.to_string().contains("disabled"), "{error}");
}

#[test]
fn a_runaway_call_is_stopped_by_the_budget() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    let started = Instant::now();
    harness
        .handle_event_raw(probe::SURFACE, UiEvent::Click(probe::SPIN))
        .expect_err("over budget");
    assert!(started.elapsed() < INIT_BUDGET, "{:?}", started.elapsed());
    assert!(harness.disabled().is_some());
}

#[test]
fn reads_the_version_section() {
    assert_eq!(
        api_version_section(&counter_wasm()).expect("parse"),
        Some(Version::new(0, 1, 0))
    );
    assert!(Harness::load(b"\0asm\x01\0\0\0").is_err());
}

/// Node ids of `fixtures/v0_2`.
mod fixture {
    use ui_ext::NodeId;

    pub const MAIN: &str = "main";
    pub const PICKER: &str = "picker";
    pub const TICKS: NodeId = 2;
    pub const STATUS: NodeId = 3;
    pub const INPUT: NodeId = 6;
    pub const LIST: NodeId = 11;
    pub const PICKER_INPUT: NodeId = 2;
    pub const PICKER_LIST: NodeId = 3;
    pub const PICKER_SELECTED: NodeId = 4;
    pub const PICKER_CLOSE: NodeId = 5;
}

fn loaded_fixture() -> Harness {
    let mut harness = Harness::load(&fixture_v0_2_wasm()).expect("load");
    harness.mount(fixture::MAIN).expect("mount main");
    harness
}

fn ids<'a>(surfaces: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    surfaces.into_iter().collect()
}

#[test]
fn v0_2_loads_with_static_and_added_surfaces() {
    let mut harness = Harness::load(&fixture_v0_2_wasm()).expect("load");
    assert_eq!(harness.api_version(), &Version::new(0, 2, 0));
    assert_eq!(harness.ui_api_version(), ApiVersion::V0_2_0);
    assert_eq!(
        ids(harness.surfaces_v0_2().iter().map(|s| s.id.as_str())),
        ["main", "picker", "boot"]
    );
    assert_eq!(
        ids(harness.surfaces().iter().map(|s| s.id.as_str())),
        ["main", "boot"],
        "the 0.1 view leaves the modal out"
    );
    assert!(matches!(
        harness.effects(),
        [HostEffect::AddSurface(surface)] if surface.id == "boot"
    ));
    assert!(matches!(
        &harness.surfaces_v0_2()[0].kind,
        model::SurfaceKind::Panel(panel) if panel.icon == model::IconRef::Frozen(model::Icon::Sparkle)
    ));

    let error = harness.mount(fixture::PICKER).expect_err("not shown");
    assert!(error.to_string().contains("not shown"), "{error}");
    assert_eq!(
        harness.mount("boot").expect("mount boot").to_nodes().len(),
        1
    );

    let error = Harness::load_with_capabilities(&fixture_v0_2_wasm(), &["ui:panel"])
        .err()
        .expect("a modal needs ui:modal");
    assert!(format!("{error:#}").contains("ui:modal"), "{error:#}");
}

#[test]
fn ticks_are_requested_clamped_and_run_on_demand() {
    let mut harness = loaded_fixture();
    assert!(harness.tick(fixture::MAIN).is_err(), "nothing requested");

    let outcome = harness.click_v0_2(fixture::MAIN, "Tick").expect("click");
    assert_eq!(
        outcome.effects,
        [HostEffect::RequestTick {
            surface_id: fixture::MAIN.into(),
            delay: MIN_TICK_DELAY,
        }],
        "10 ms is clamped up"
    );
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok")
    );
    assert_eq!(harness.pending_tick(fixture::MAIN), Some(MIN_TICK_DELAY));

    harness.tick(fixture::MAIN).expect("tick");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::TICKS),
        Some("ticks: 1")
    );
    assert_eq!(harness.pending_tick(fixture::MAIN), None, "one-shot");

    harness
        .click_v0_2(fixture::MAIN, "Failing ticks")
        .expect("click");
    for _ in 0..MAX_CONSECUTIVE_TICK_ERRORS {
        let error = harness.tick(fixture::MAIN).expect_err("fails");
        assert!(format!("{error:#}").contains("tick failed"), "{error:#}");
        assert!(
            harness.pending_tick(fixture::MAIN).is_some(),
            "re-requested"
        );
    }
    let error = harness.tick(fixture::MAIN).expect_err("stopped");
    assert!(error.to_string().contains("stopped"), "{error}");
    harness
        .event_v0_2(fixture::MAIN, model::UiEvent::Click(fixture::TICKS))
        .expect("an event resumes ticks");
    let error = harness.tick(fixture::MAIN).expect_err("still failing");
    assert!(format!("{error:#}").contains("tick failed"), "{error:#}");
    assert!(harness.disabled().is_none());
}

#[test]
fn a_modal_needs_a_gesture_mounts_when_shown_and_closes() {
    let mut harness = loaded_fixture();
    harness
        .event_v0_2(
            fixture::MAIN,
            model::UiEvent::InputChanged(fixture::INPUT, "open".into()),
        )
        .expect("typing");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: not-permitted"),
        "typing is no gesture"
    );
    assert!(!harness.is_shown(fixture::PICKER));

    harness
        .click_v0_2(fixture::MAIN, "Open picker")
        .expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok")
    );
    assert!(harness.is_shown(fixture::PICKER));
    assert!(matches!(
        harness.props_v0_2(fixture::PICKER, fixture::PICKER_INPUT),
        Some(model::Props::Input(input)) if input.secret
    ));
    assert!(matches!(
        harness.props_v0_2(fixture::PICKER, fixture::PICKER_CLOSE),
        Some(model::Props::Button(button)) if button.tooltip.as_deref() == Some("Close the picker")
    ));

    harness
        .event_v0_2(
            fixture::PICKER,
            model::UiEvent::RowSelected(fixture::PICKER_LIST, 2),
        )
        .expect("select");
    assert!(matches!(
        harness.props_v0_2(fixture::PICKER, fixture::PICKER_LIST),
        Some(model::Props::List(list)) if list.selected == Some(2)
    ));
    assert_eq!(
        harness.text(fixture::PICKER, fixture::PICKER_SELECTED),
        Some("selected: 2")
    );

    let outcome = harness
        .event_v0_2(
            fixture::PICKER,
            model::UiEvent::InputSubmitted(fixture::PICKER_INPUT, "x".into()),
        )
        .expect("submit");
    assert!(matches!(
        &outcome.effects[..],
        [HostEffect::HideSurface { .. }, HostEffect::Push { surface_id, .. }] if surface_id == fixture::MAIN
    ));
    assert!(!harness.is_shown(fixture::PICKER));
    assert!(harness.tree(fixture::PICKER).is_none());
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: picked 2")
    );

    harness
        .click_v0_2(fixture::MAIN, "Open picker")
        .expect("click");
    assert_eq!(
        harness.text(fixture::PICKER, fixture::PICKER_SELECTED),
        Some("selected: none"),
        "a fresh mount each time it shows"
    );
    let outcome = harness
        .dismiss(fixture::PICKER, model::DismissReason::Escape)
        .expect("dismiss");
    assert!(
        matches!(outcome.update, model::Update::Patches(_)),
        "returned, not applied"
    );
    assert!(!harness.is_shown(fixture::PICKER));
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: dismissed escape")
    );
    assert!(
        harness
            .dismiss(fixture::PICKER, model::DismissReason::Escape)
            .is_err()
    );

    harness
        .click_v0_2(fixture::MAIN, "Open later")
        .expect("click");
    harness.tick(fixture::MAIN).expect("tick");
    assert!(
        harness.is_shown(fixture::PICKER),
        "within the activation window"
    );
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok")
    );
}

#[test]
fn runtime_surfaces_and_decorations() {
    let mut harness = loaded_fixture();
    harness
        .click_v0_2(fixture::MAIN, "Add clock")
        .expect("click");
    assert!(harness.surfaces_v0_2().iter().any(|s| s.id == "clock"));
    harness.mount("clock").expect("mount clock");
    assert_eq!(harness.text("clock", 1), Some("clock"));

    harness
        .click_v0_2(fixture::MAIN, "Remove clock")
        .expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok, push unknown-surface"),
        "checked against the live set as of that point in the call"
    );
    assert!(!harness.surfaces_v0_2().iter().any(|s| s.id == "clock"));
    assert!(harness.tree("clock").is_none());

    harness
        .click_v0_2(fixture::MAIN, "Place note")
        .expect("click");
    assert_eq!(harness.decoration("note").map(|l| l.line), Some(3));
    harness
        .click_v0_2("note", "Ack")
        .expect("an event on a decoration");
    assert_eq!(harness.text("note", 2), Some("acked"));
    harness
        .click_v0_2(fixture::MAIN, "Remove note")
        .expect("click");
    assert_eq!(harness.decorations().count(), 0);
    assert!(harness.tree("note").is_none());

    harness
        .click_v0_2(fixture::MAIN, "Place bad note")
        .expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok")
    );
    assert_eq!(harness.decorations().count(), 0, "its remount failed too");
    assert!(
        harness
            .logs()
            .iter()
            .any(|(level, message)| *level == ui_ext::LogLevel::Warn
                && message.contains("removed decoration \"bad\"")),
        "{:?}",
        harness.logs()
    );

    let mut harness = Harness::load_with_capabilities(
        &fixture_v0_2_wasm(),
        &["ui:panel", "ui:modal", "ui:status-bar"],
    )
    .expect("load");
    harness.mount(fixture::MAIN).expect("mount");
    harness
        .click_v0_2(fixture::MAIN, "Place note")
        .expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: not-permitted")
    );
}

#[test]
fn invalidate_rows_and_indent() {
    let mut harness = loaded_fixture();
    let outcome = harness
        .click_v0_2(fixture::MAIN, "Refresh rows")
        .expect("click");
    assert_eq!(
        outcome.invalidated,
        [model::InvalidateRowsOp {
            target: fixture::LIST,
            rows: Some(model::RowRange { start: 1, count: 1 }),
        }]
    );
    assert!(outcome.touched.contains(&fixture::LIST));
    let outcome = harness.click(fixture::MAIN, "Refresh rows").expect("click");
    assert_eq!(
        outcome.update,
        Update::Patches(Vec::new()),
        "the 0.1 view drops invalidate-rows"
    );

    let rows = harness
        .rows_v0_2(fixture::MAIN, fixture::LIST, 0, 10)
        .expect("rows");
    assert_eq!(
        rows.iter()
            .map(|row| (row.index, row.indent))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 1), (2, 2)]
    );
}

#[test]
fn pushes_past_the_cap_return_limit_exceeded() {
    let mut harness = loaded_fixture();
    let outcome = harness.click_v0_2(fixture::MAIN, "Flood").expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok*64 limit-exceeded*6")
    );
    assert_eq!(outcome.effects.len(), MAX_PUSHES_PER_CALL);
    assert!(harness.disabled().is_none(), "no trap in 0.2");
}

#[test]
fn the_counter_demo_runs_as_0_2() {
    let mut harness = Harness::load(&counter_v0_2_wasm()).expect("load");
    assert_eq!(harness.ui_api_version(), ApiVersion::V0_2_0);
    harness.mount("counter").expect("mount");
    harness.click("counter", "Increment").expect("click");
    assert_eq!(harness.text("counter", 2), Some("Count: 1"));
    assert!(matches!(
        harness.props_v0_2("counter", 5),
        Some(model::Props::Button(button)) if button.icon == Some(model::IconRef::Frozen(model::Icon::Plus))
    ));
}
