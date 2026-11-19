//! The 0.4 consent UI (plan 5.7): the model stand-in, the stand-in broker and the install dialog.

use std::time::Duration;

use gpui::{TestAppContext, VisualTestContext};
use ui_ext_wasm::{UiManifest, permissions::PermissionState};

use crate::{
    consent::{ConsentRecord, ConsentStore, Grant},
    consent_dialog::{ConsentDialog, ConsentDialogText, ask_consent},
    consent_model::{
        ConsentBroker as _, ConsentItem, ConsentTier, ItemLine, consent_diff, consent_items,
    },
    notifications::CLICK_GUARD,
};

const MANIFEST: &str = r#"
api_version = "0.4.0"
component = "x.wasm"
capabilities = ["ui:panel", "ui:approvals", "terminal:read", "context:editor", "terminal:create",
                "terminal:env", "network:fetch"]
[network]
domains = ["api.example.com"]
[[terminal_programs]]
name = "codex"
args = ["-c", "notify=*"]
reason = "Runs Codex for you"
[[terminal_programs]]
name = "git"
args = ["status"]
reason = "Shows the branch"
[terminal_env]
vars = { CODEX_TUI_NOTIFY = "1", CODEX_PLUGINS = "${package}/plugins" }
scope = "all-new"
reason = "So Codex reports its status"
"#;

fn manifest(text: &str) -> UiManifest {
    toml::from_str(text).expect("manifest")
}

fn program(name: &str) -> ConsentItem {
    ConsentItem::Program {
        capability: "terminal:create".into(),
        name: name.into(),
    }
}

fn record() -> ConsentRecord {
    ConsentRecord {
        capabilities: Vec::new(),
        grant: Grant::Component {
            sha256: "abc".into(),
        },
    }
}

/// §C.2: the groups in order; one line per program, one for the env and the stronger
/// "all-new" one; the author's reasons; the env and combination warnings.
#[test]
fn test_consent_items_groups_and_lines() {
    let lines = consent_items(&manifest(MANIFEST));
    let tiers: Vec<ConsentTier> = lines.iter().map(|line| line.tier).collect();
    let mut sorted = tiers.clone();
    sorted.sort();
    assert_eq!(tiers, sorted, "groups 1-4 in order");
    let keys: Vec<String> = lines.iter().map(|line| line.item.key()).collect();
    assert_eq!(
        keys,
        [
            "ui:panel",
            "ui:approvals",
            "context:editor",
            "terminal:create#codex",
            "terminal:create#git",
            "terminal:env",
            "terminal:env#all-new",
            "network:fetch",
            "terminal:read",
        ]
    );
    let line = |key: &str| lines.iter().find(|line| line.item.key() == key).unwrap();
    assert_eq!(line("terminal:create#codex").details, ["-c", "notify=*"]);
    assert_eq!(
        line("terminal:create#codex").reason.as_deref(),
        Some("Runs Codex for you")
    );
    assert_eq!(line("network:fetch").details, ["api.example.com"]);
    assert!(
        line("terminal:env")
            .details
            .contains(&"CODEX_TUI_NOTIFY=1".to_string())
    );
    assert!(
        line("terminal:env")
            .warnings
            .iter()
            .any(|warning| warning.contains("CODEX_PLUGINS")),
        "a value inside the package says so"
    );
    assert!(
        line("context:editor")
            .warnings
            .iter()
            .any(|warning| warning.contains("send what it reads")),
        "a read with a way out (§C.1)"
    );
    let checkboxes: Vec<String> = lines
        .iter()
        .filter(|line| line.has_checkbox())
        .map(|line| line.item.key())
        .collect();
    assert_eq!(
        checkboxes,
        [
            "terminal:create#codex",
            "terminal:create#git",
            "terminal:env",
            "terminal:env#all-new",
            "network:fetch",
        ],
        "T2 only; T3 is listed without one"
    );
}

/// §C.5: a new program and a new argument pattern hold the update; a removal and an item the
/// user denied do not.
#[test]
fn test_consent_diff() {
    let granted = consent_items(&manifest(MANIFEST));
    let newer = MANIFEST
        .replace(r#"args = ["status"]"#, r#"args = ["status", "push"]"#)
        .replace(
            r#"domains = ["api.example.com"]"#,
            r#"domains = ["api.example.com", "evil.example.com"]"#,
        )
        .replace(r#""ui:approvals", "#, "")
        + "[[terminal_programs]]\nname = \"claude\"\nreason = \"Runs Claude\"\n";
    let diff = consent_diff(
        &granted,
        &[ConsentItem::Capability("network:fetch".into())],
        &consent_items(&manifest(&newer)),
    );
    let keys =
        |lines: &[ItemLine]| -> Vec<String> { lines.iter().map(|line| line.item.key()).collect() };
    assert_eq!(keys(&diff.added), ["terminal:create#claude"]);
    assert_eq!(
        keys(&diff.widened),
        ["terminal:create#git"],
        "the denied network:fetch stays denied"
    );
    assert_eq!(keys(&diff.removed), ["ui:approvals"]);
    assert!(diff.needs_approval());
    let same = consent_diff(&granted, &[], &granted);
    assert!(!same.needs_approval());
}

/// The stand-in broker fails closed: a capability is allowed only when every item of it is
/// ticked; the page's switch turns it off at once.
#[test]
fn test_stand_in_broker() {
    let lines = consent_items(&manifest(MANIFEST));
    let mut store = ConsentStore::default();
    store.declare(
        "demo",
        vec![
            "terminal:create".into(),
            "terminal:env".into(),
            "network:fetch".into(),
        ],
    );
    store.install(
        "demo",
        record(),
        &lines,
        &[
            program("codex"),
            ConsentItem::Capability("network:fetch".into()),
        ],
    );
    let state = |store: &ConsentStore, item: &ConsentItem| store.item_state("demo", item);
    assert_eq!(
        state(&store, &program("codex")),
        PermissionState::Denied,
        "git is unticked"
    );
    assert_eq!(
        state(&store, &ConsentItem::Capability("network:fetch".into())),
        PermissionState::Granted
    );
    assert_eq!(
        state(&store, &ConsentItem::EnvAllNew),
        PermissionState::Denied
    );

    store.set_item("demo", &lines, &program("git"), true);
    assert_eq!(
        state(&store, &program("codex")),
        PermissionState::Denied,
        "the stand-in needs every item's answer"
    );
    store.set_item(
        "demo",
        &lines,
        &ConsentItem::Capability("network:fetch".into()),
        false,
    );
    assert_eq!(
        state(&store, &ConsentItem::Capability("network:fetch".into())),
        PermissionState::Denied
    );
    assert_eq!(
        state(&store, &ConsentItem::Capability("ui:panel".into())),
        PermissionState::Granted,
        "T0"
    );
}

async fn open_dialog(
    cx: &mut TestAppContext,
) -> (
    gpui::Entity<workspace::Workspace>,
    futures::channel::oneshot::Receiver<Vec<ConsentItem>>,
    &mut VisualTestContext,
) {
    crate::tests::init_test(cx);
    let fs = fs::FakeFs::new(cx.executor());
    let project = project::Project::test(fs, [], cx).await;
    let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());
    crate::decoration_tests::redraw(cx);
    let lines = consent_items(&manifest(MANIFEST));
    let answer = workspace.update_in(cx, |workspace, window, cx| {
        ask_consent(
            workspace,
            ConsentDialogText {
                title: "Install Demo?".into(),
                notes: Vec::new(),
                confirm: "Install".into(),
            },
            lines,
            window,
            cx,
        )
    });
    crate::decoration_tests::redraw(cx);
    (workspace, answer, cx)
}

fn dialog(
    workspace: &gpui::Entity<workspace::Workspace>,
    cx: &mut VisualTestContext,
) -> gpui::Entity<ConsentDialog> {
    workspace
        .read_with(cx, |workspace, cx| {
            workspace.active_modal::<ConsentDialog>(cx)
        })
        .expect("the dialog is open")
}

/// §C.2: checkboxes and Install ignore clicks for 1 s; only T2 lines toggle; Enter installs
/// nothing; Install answers what is ticked.
#[gpui::test]
async fn test_consent_dialog_guard_and_ticks(cx: &mut TestAppContext) {
    let (workspace, mut answer, cx) = open_dialog(cx).await;
    let view = dialog(&workspace, cx);
    view.update(cx, |dialog, cx| {
        dialog.toggle(&program("codex"), cx);
        dialog.install(cx);
    });
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |dialog, _| dialog.ticked()), []);
    assert!(answer.try_recv().unwrap().is_none(), "inside the guard");

    cx.executor()
        .advance_clock(CLICK_GUARD + Duration::from_millis(1));
    view.update(cx, |dialog, cx| {
        dialog.toggle(&program("codex"), cx);
        dialog.toggle(&ConsentItem::EnvAllNew, cx);
        dialog.toggle(&ConsentItem::EnvAllNew, cx);
        dialog.toggle(&ConsentItem::Capability("context:editor".into()), cx);
        dialog.toggle(&ConsentItem::Capability("terminal:read".into()), cx);
    });
    cx.dispatch_action(menu::Confirm);
    cx.run_until_parked();
    assert!(answer.try_recv().unwrap().is_none(), "no Enter default");
    view.update(cx, |dialog, cx| dialog.install(cx));
    cx.run_until_parked();
    assert_eq!(answer.try_recv().unwrap(), Some(vec![program("codex")]));
}

/// Escape answers nothing.
#[gpui::test]
async fn test_consent_dialog_escape(cx: &mut TestAppContext) {
    let (workspace, mut answer, cx) = open_dialog(cx).await;
    let _ = dialog(&workspace, cx);
    cx.dispatch_action(menu::Cancel);
    cx.run_until_parked();
    assert!(answer.try_recv().is_err(), "dropped without an answer");
}
