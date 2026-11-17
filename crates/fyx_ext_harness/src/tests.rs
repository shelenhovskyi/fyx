use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ui_ext::{InsertOp, Patch, UiEvent, Update};

use super::*;

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Builds a guest for `wasm32-wasip2` with `cargo build <args>` in `dir`, into a target directory
/// next to the test binary's own, and returns `<target>/wasm32-wasip2/<output>`.
fn build_guest(dir: &Path, args: &[&str], output: &str) -> std::result::Result<PathBuf, String> {
    // <target>/debug/deps/<test binary>
    let target_dir = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .ancestors()
        .nth(3)
        .ok_or("the test binary is not inside a cargo target directory")?
        .join("fyx-ui-fixtures");
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

fn demo(
    crate_name: &'static str,
    built: &'static OnceLock<std::result::Result<PathBuf, String>>,
) -> Vec<u8> {
    let path = built
        .get_or_init(|| {
            build_guest(
                &workspace_dir(),
                &["-p", crate_name],
                &format!("debug/{crate_name}.wasm"),
            )
        })
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn counter_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_counter", &BUILT)
}

fn todo_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_todo", &BUILT)
}

fn probe_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    read_guest(&BUILT, || {
        build_guest(
            &workspace_dir().join("crates/ui_ext_wasm/fixtures/probe"),
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
