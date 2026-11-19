use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ui_ext::{InsertOp, Patch, UiEvent, Update};

use super::*;

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `<target>`: the cargo target directory of the test binary.
fn test_target_dir() -> std::result::Result<PathBuf, String> {
    // <target>/debug/deps/<test binary>
    Ok(std::env::current_exe()
        .map_err(|error| error.to_string())?
        .ancestors()
        .nth(3)
        .ok_or("the test binary is not inside a cargo target directory")?
        .to_path_buf())
}

/// Builds a guest for `wasm32-wasip2` with `cargo build <args>` in `dir`, into the target directory
/// `target` next to the test binary's own, and returns `<target>/wasm32-wasip2/<output>`. Builds
/// that differ only in features need targets of their own, or they overwrite each other's output.
///
/// Under nextest, which runs every test in a process of its own, the build runs once per guest and
/// test run: the first process to take the guest's lock file builds it and stamps it with
/// `NEXTEST_RUN_ID`, and the others find the stamp. The sources do not change during a run. It
/// also turns on the harness's cache of compiled components, in `<target>/fyx-ext-harness-cache`.
fn build_guest(
    dir: &Path,
    target: &str,
    args: &[&str],
    output: &str,
) -> std::result::Result<PathBuf, String> {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};

    let test_target = test_target_dir()?;
    cache_compiled_components(test_target.join("fyx-ext-harness-cache"));
    let target_dir = test_target.join(target);
    let built = target_dir.join("wasm32-wasip2").join(output);
    let Ok(run_id) = std::env::var("NEXTEST_RUN_ID") else {
        run_cargo_build(dir, &target_dir, args)?;
        return Ok(built);
    };
    let mut hasher = DefaultHasher::new();
    (dir, args, output).hash(&mut hasher);
    let stamp = target_dir.join(format!("fyx-built-{:016x}", hasher.finish()));
    std::fs::create_dir_all(&target_dir)
        .map_err(|error| format!("creating {target_dir:?}: {error}"))?;
    let lock = std::fs::File::create(stamp.with_extension("lock"))
        .map_err(|error| format!("creating the lock for {stamp:?}: {error}"))?;
    // Held until this process has built the guest, or seen that another one did.
    lock.lock()
        .map_err(|error| format!("locking {stamp:?}: {error}"))?;
    if std::fs::read_to_string(&stamp).is_ok_and(|stamped| stamped == run_id) && built.exists() {
        return Ok(built);
    }
    run_cargo_build(dir, &target_dir, args)?;
    std::fs::write(&stamp, &run_id).map_err(|error| format!("writing {stamp:?}: {error}"))?;
    Ok(built)
}

fn run_cargo_build(
    dir: &Path,
    target_dir: &Path,
    args: &[&str],
) -> std::result::Result<(), String> {
    // Blocking is fine: it runs once per guest and test process, before any test work.
    #[allow(clippy::disallowed_methods)]
    let status = std::process::Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .arg("build")
        .args(args)
        .args(["--locked", "--target", "wasm32-wasip2", "--target-dir"])
        .arg(target_dir)
        .current_dir(dir)
        .status()
        .map_err(|error| format!("running cargo: {error}"))?;
    if !status.success() {
        return Err(format!("cargo build {args:?} in {dir:?} failed: {status}"));
    }
    Ok(())
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

/// The demos the tests load as 0.1 and as 0.2. Each set is built with one `cargo build`, which
/// gives the same components as building them one by one: their `fyx-ui-0-2` features change
/// only the demo itself, not the crates they share.
const DEMOS_V0_1: [&str; 3] = ["fyx_demo_counter", "fyx_demo_todo", "fyx_demo_decorations"];
const DEMOS_V0_2: [&str; 3] = [
    "fyx_demo_counter",
    "fyx_demo_gallery",
    "fyx_demo_decorations",
];

/// A demo's default (0.1) build, or with `v0_2` its `fyx-ui-0-2` build.
fn demo(crate_name: &'static str, v0_2: bool) -> Vec<u8> {
    fn build(demos: [&str; 3], v0_2: bool) -> std::result::Result<PathBuf, String> {
        let features = demos.map(|demo| format!("{demo}/fyx-ui-0-2")).join(",");
        let mut args = Vec::new();
        if v0_2 {
            args.extend(["--features", &features]);
        }
        for demo in &demos {
            args.extend(["-p", demo]);
        }
        let target = if v0_2 {
            "fyx-ui-fixtures-v0_2"
        } else {
            "fyx-ui-fixtures"
        };
        let wasm = build_guest(
            &workspace_dir(),
            target,
            &args,
            &format!("debug/{}.wasm", demos[0]),
        )?;
        Ok(wasm.parent().ok_or("no output directory")?.to_path_buf())
    }
    static BUILT_V0_1: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    static BUILT_V0_2: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    let (demos, built) = if v0_2 {
        (DEMOS_V0_2, &BUILT_V0_2)
    } else {
        (DEMOS_V0_1, &BUILT_V0_1)
    };
    assert!(
        demos.contains(&crate_name),
        "{crate_name} is not in the demo list for this build"
    );
    let dir = built
        .get_or_init(|| build(demos, v0_2))
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    let path = dir.join(format!("{crate_name}.wasm"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn counter_wasm() -> Vec<u8> {
    demo("fyx_demo_counter", false)
}

fn counter_v0_2_wasm() -> Vec<u8> {
    demo("fyx_demo_counter", true)
}

fn todo_wasm() -> Vec<u8> {
    demo("fyx_demo_todo", false)
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
    pub const ALLOCATE: NodeId = 7;
    pub const PUSH_BULK: NodeId = 8;
    pub const PUSH_TEXT: NodeId = 10;
    pub const LOG_LONG: NodeId = 11;
}

#[test]
fn counter_follows_its_contract() {
    let mut harness = Harness::load(&counter_wasm()).expect("load");
    assert_eq!(harness.api_version(), &Version::new(0, 1, 0));
    assert_eq!(
        harness
            .surfaces_v0_1()
            .iter()
            .map(|surface| surface.id.as_str())
            .collect::<Vec<_>>(),
        ["counter"]
    );
    assert!(
        harness.event_v0_1("counter", UiEvent::Click(5)).is_err(),
        "not mounted yet"
    );
    assert!(harness.mount("nope").is_err());

    let ids = harness
        .mount("counter")
        .expect("mount")
        .iter()
        .map(|node| node.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(harness.text("counter", 2), Some("Count: 0"));

    let outcome = harness.click_v0_1("counter", "Increment").expect("click");
    assert!(outcome.pushed.is_empty());
    assert!(
        outcome.touched.contains(&2) && outcome.touched.contains(&7),
        "{outcome:?}"
    );
    assert_eq!(harness.text("counter", 2), Some("Count: 1"));
    assert_eq!(harness.find_text("counter", "Count: 1"), Some(2));

    let rows = harness.rows_v0_1("counter", 7, 0, 10).expect("rows");
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

    let remounted = harness.mount_v0_1("counter").expect("remount");
    assert_eq!(remounted.roots(), [8]);
    assert!(harness.disabled().is_none());
}

#[test]
fn typed_text_is_written_before_the_event() {
    let mut harness = Harness::load(&todo_wasm()).expect("load");
    harness.mount("todos").expect("mount");
    let input = harness
        .find_v0_1("todos", |props| matches!(props, Props::Input(_)))
        .expect("the filter input");
    harness
        .event_v0_1("todos", UiEvent::InputChanged(input, "main".into()))
        .expect("typing");
    match harness
        .tree_v0_1("todos")
        .and_then(|tree| tree.get(input))
        .map(TreeNode::props)
    {
        Some(Props::Input(props)) => assert_eq!(props.value, "main"),
        other => panic!("the input changed kind: {other:?}"),
    }
    assert!(
        harness
            .find_v0_1("todos", |props| matches!(
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
    assert!(harness.surfaces_v0_1().is_empty());

    let reply = harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH_THEN_PATCH))
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
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH))
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
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH_THEN_FAIL))
        .expect("call");
    assert_eq!(reply.value, Err("failed after pushing".to_string()));
    assert_eq!(reply.pushed.len(), 1);
    assert_eq!(harness.pushed_v0_1().len(), 3);
    assert!(
        harness.disabled().is_none(),
        "a guest error does not disable"
    );
}

#[test]
fn a_push_flood_traps_and_disables() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    let error = harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH_FLOOD))
        .expect_err("the 65th push traps");
    assert!(
        format!("{error:#}").contains("pushed more than 64"),
        "{error:#}"
    );
    assert!(harness.disabled().is_some());
    let error = harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH))
        .expect_err("disabled");
    assert!(error.to_string().contains("disabled"), "{error}");
}

/// security-2 T9: the host's memory cap applies here too, so a guest that grows past it fails in
/// tests as it does in Fyx.
#[test]
fn growing_past_the_memory_cap_traps_and_disables() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::ALLOCATE))
        .expect_err("300 MiB is over the 256 MiB cap");
    assert!(harness.disabled().is_some());
}

/// The host's per-call caps on pushed items and on text, which trap.
#[test]
fn per_call_item_and_text_caps_trap_as_in_the_host() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    let error = harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH_BULK))
        .expect_err("40 pushes of 200 patches are over 4096 items");
    assert!(
        format!("{error:#}").contains("patches and nodes in one call"),
        "{error:#}"
    );

    let mut harness = Harness::load(&probe_wasm()).expect("load");
    let error = harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::PUSH_TEXT))
        .expect_err("20 MiB of text is over 16 MiB");
    assert!(format!("{error:#}").contains("bytes of text"), "{error:#}");

    // A 1 MiB log line is under the text cap, and kept cut to 16 KiB.
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::LOG_LONG))
        .expect("under the cap");
    let (_, message) = harness.logs().last().expect("logged");
    assert!(
        message.len() < MAX_LOG_MESSAGE_LEN + 64,
        "{}",
        message.len()
    );
    assert!(message.ends_with("bytes, cut)"), "{message:.40}");
}

#[test]
fn a_runaway_call_is_stopped_by_the_budget() {
    let mut harness = Harness::load(&probe_wasm()).expect("load");
    let started = Instant::now();
    harness
        .handle_event_raw_v0_1(probe::SURFACE, UiEvent::Click(probe::SPIN))
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
fn notifications_and_progress_run_with_their_gestures() {
    use ui_ext::v0_2::notifications::{NotificationCloseReason, ProgressValue};
    let mut harness = loaded_fixture();
    let status = |harness: &Harness| {
        harness
            .text(fixture::MAIN, fixture::STATUS)
            .map(str::to_string)
    };

    let outcome = harness.click(fixture::MAIN, "Notify").expect("notify");
    assert!(
        matches!(&outcome.effects[..], [HostEffect::Notify { id: 1, .. }]),
        "{outcome:?}"
    );
    assert_eq!(status(&harness).as_deref(), Some("status: notified 1"));
    assert_eq!(harness.notifications()[&1].title, "Saved");

    harness.notification_action(1, "undo").expect("undo");
    assert_eq!(status(&harness).as_deref(), Some("status: action 1 undo"));
    assert!(harness.notifications().is_empty(), "acting closes it");
    assert!(harness.notification_action(1, "undo").is_err());

    harness.click(fixture::MAIN, "Notify").expect("notify");
    assert!(harness.notification_action(2, "nope").is_err());
    harness
        .close_notification(2, NotificationCloseReason::Expired)
        .expect("expire");
    assert_eq!(
        status(&harness).as_deref(),
        Some("status: closed 2 expired")
    );

    // Three open at most; the fourth is logged instead.
    for _ in 0..4 {
        harness.click(fixture::MAIN, "Notify").expect("notify");
    }
    assert_eq!(status(&harness).as_deref(), Some("status: limit-exceeded"));
    assert_eq!(harness.notifications().len(), 3);
    assert!(
        harness
            .logs()
            .iter()
            .any(|(_, line)| line.contains("notification not shown"))
    );

    harness.click(fixture::MAIN, "Start job").expect("start");
    assert_eq!(
        status(&harness).as_deref(),
        Some("status: started 1, update ok")
    );
    let progress = &harness.progresses()[&1];
    assert_eq!(progress.value, ProgressValue::Percent(10));
    assert_eq!(progress.message.as_deref(), Some("1 of 10"));
    let outcome = harness.cancel_progress(1).expect("cancel");
    assert!(
        matches!(&outcome.effects[..], [HostEffect::ProgressEnd { id: 1 }]),
        "{outcome:?}"
    );
    assert_eq!(status(&harness).as_deref(), Some("status: cancelled 1"));
    assert!(harness.progresses().is_empty());
    assert!(harness.cancel_progress(1).is_err());
}

#[test]
fn v0_2_loads_with_static_and_added_surfaces() {
    let mut harness = Harness::load(&fixture_v0_2_wasm()).expect("load");
    assert_eq!(harness.api_version(), &Version::new(0, 2, 0));
    assert_eq!(harness.ui_api_version(), ApiVersion::V0_2_0);
    assert_eq!(
        ids(harness.surfaces().iter().map(|s| s.id.as_str())),
        ["main", "picker", "boot"]
    );
    assert_eq!(
        ids(harness.surfaces_v0_1().iter().map(|s| s.id.as_str())),
        ["main", "boot"],
        "the 0.1 view leaves the modal out"
    );
    assert!(matches!(
        harness.effects(),
        [HostEffect::AddSurface(surface)] if surface.id == "boot"
    ));
    assert!(matches!(
        &harness.surfaces()[0].kind,
        model::SurfaceKind::Panel(panel) if panel.icon == model::IconRef::Frozen(model::Icon::Sparkle)
    ));

    let error = harness.mount(fixture::PICKER).expect_err("not shown");
    assert!(error.to_string().contains("not shown"), "{error}");
    assert_eq!(harness.mount("boot").expect("mount boot").len(), 1);

    let error = Harness::load_with_capabilities(&fixture_v0_2_wasm(), &["ui:panel"])
        .err()
        .expect("a modal needs ui:modal");
    assert!(format!("{error:#}").contains("ui:modal"), "{error:#}");
}

#[test]
fn ticks_are_requested_clamped_and_run_on_demand() {
    let mut harness = loaded_fixture();
    assert!(harness.tick(fixture::MAIN).is_err(), "nothing requested");

    let outcome = harness.click(fixture::MAIN, "Tick").expect("click");
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
        .click(fixture::MAIN, "Failing ticks")
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
        .event(fixture::MAIN, model::UiEvent::Click(fixture::TICKS))
        .expect("an event resumes ticks");
    let error = harness.tick(fixture::MAIN).expect_err("still failing");
    assert!(format!("{error:#}").contains("tick failed"), "{error:#}");
    assert!(harness.disabled().is_none());
}

#[test]
fn a_modal_needs_a_gesture_mounts_when_shown_and_closes() {
    let mut harness = loaded_fixture();
    harness
        .event(
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

    harness.click(fixture::MAIN, "Open picker").expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok")
    );
    assert!(harness.is_shown(fixture::PICKER));
    assert!(matches!(
        harness.props(fixture::PICKER, fixture::PICKER_INPUT),
        Some(model::Props::Input(input)) if input.secret
    ));
    assert!(matches!(
        harness.props(fixture::PICKER, fixture::PICKER_CLOSE),
        Some(model::Props::Button(button)) if button.tooltip.as_deref() == Some("Close the picker")
    ));

    harness
        .event(
            fixture::PICKER,
            model::UiEvent::RowSelected(fixture::PICKER_LIST, 2),
        )
        .expect("select");
    assert!(matches!(
        harness.props(fixture::PICKER, fixture::PICKER_LIST),
        Some(model::Props::List(list)) if list.selected == Some(2)
    ));
    assert_eq!(
        harness.text(fixture::PICKER, fixture::PICKER_SELECTED),
        Some("selected: 2")
    );

    let outcome = harness
        .event(
            fixture::PICKER,
            model::UiEvent::InputSubmitted(fixture::PICKER_INPUT, "x".into()),
        )
        .expect("submit");
    assert!(matches!(
        &outcome.effects[..],
        [HostEffect::HideSurface { .. }, HostEffect::Push { surface_id, .. }] if surface_id == fixture::MAIN
    ));
    assert!(!harness.is_shown(fixture::PICKER));
    assert!(harness.tree_v0_1(fixture::PICKER).is_none());
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: picked 2")
    );

    harness.click(fixture::MAIN, "Open picker").expect("click");
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

    harness.click(fixture::MAIN, "Open later").expect("click");
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
    harness.click(fixture::MAIN, "Add clock").expect("click");
    assert!(harness.surfaces().iter().any(|s| s.id == "clock"));
    harness.mount("clock").expect("mount clock");
    assert_eq!(harness.text("clock", 1), Some("clock"));

    harness.click(fixture::MAIN, "Remove clock").expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: ok, push unknown-surface"),
        "checked against the live set as of that point in the call"
    );
    assert!(!harness.surfaces().iter().any(|s| s.id == "clock"));
    assert!(harness.tree_v0_1("clock").is_none());

    harness.click(fixture::MAIN, "Place note").expect("click");
    assert_eq!(harness.decoration("note").map(|l| l.line), Some(3));
    harness
        .click("note", "Ack")
        .expect("an event on a decoration");
    assert_eq!(harness.text("note", 2), Some("acked"));
    harness.click(fixture::MAIN, "Remove note").expect("click");
    assert_eq!(harness.decorations().count(), 0);
    assert!(harness.tree_v0_1("note").is_none());

    harness
        .click(fixture::MAIN, "Place bad note")
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
    harness.click(fixture::MAIN, "Place note").expect("click");
    assert_eq!(
        harness.text(fixture::MAIN, fixture::STATUS),
        Some("status: not-permitted")
    );
}

#[test]
fn invalidate_rows_and_indent() {
    let mut harness = loaded_fixture();
    let outcome = harness.click(fixture::MAIN, "Refresh rows").expect("click");
    assert_eq!(
        outcome.invalidated,
        [model::InvalidateRowsOp {
            target: fixture::LIST,
            rows: Some(model::RowRange { start: 1, count: 1 }),
        }]
    );
    assert!(outcome.touched.contains(&fixture::LIST));
    let outcome = harness
        .click_v0_1(fixture::MAIN, "Refresh rows")
        .expect("click");
    assert_eq!(
        outcome.update,
        Update::Patches(Vec::new()),
        "the 0.1 view drops invalidate-rows"
    );

    let rows = harness
        .rows(fixture::MAIN, fixture::LIST, 0, 10)
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
    let outcome = harness.click(fixture::MAIN, "Flood").expect("click");
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
    harness.click_v0_1("counter", "Increment").expect("click");
    assert_eq!(harness.text("counter", 2), Some("Count: 1"));
    assert!(matches!(
        harness.props("counter", 5),
        Some(model::Props::Button(button)) if button.icon == Some(model::IconRef::Frozen(model::Icon::Plus))
    ));
}

mod demos_v0_2;
mod demos_v0_3;
mod demos_v0_4;
