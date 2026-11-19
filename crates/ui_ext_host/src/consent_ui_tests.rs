//! The 0.4 consent UI (plan 5.7) on W2c's broker: the install dialog, the page's items and
//! journal, and the held update's card.

use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, VisualTestContext};
use ui_ext_wasm::{UiManifest, permissions::PermissionState};

use crate::{
    consent::{ConsentRecord, ConsentStore, Grant},
    consent_dialog::{ConsentDialog, ConsentDialogText, ask_consent},
    consent_model::{ConsentItem, ItemLineExt as _, consent_items},
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

/// What the dialog relies on: one line per program and the stronger "all-new" one, and
/// checkboxes on T2 lines only.
#[test]
fn test_consent_items_checkboxes() {
    let lines = consent_items(&manifest(MANIFEST));
    let checkboxes: Vec<String> = lines
        .iter()
        .filter(|line| line.has_checkbox())
        .map(|line| line.item.key())
        .collect();
    for key in [
        "terminal:create#codex",
        "terminal:create#git",
        "terminal:env#all-new",
        "network:fetch",
    ] {
        assert!(
            checkboxes.contains(&key.to_string()),
            "{key}: {checkboxes:?}"
        );
    }
    for key in ["ui:panel", "context:editor", "terminal:read"] {
        assert!(
            lines.iter().any(|line| line.item.key() == key) && !checkboxes.contains(&key.into()),
            "{key} is listed without a checkbox"
        );
    }
}

/// A registry update's dialog shows only what it adds or widens, and notes what it drops.
#[test]
fn test_registry_update_lines() {
    let before = consent_items(&manifest(MANIFEST));
    let newer = MANIFEST
        .replace(r#"args = ["status"]"#, r#"args = ["status", "push"]"#)
        .replace(r#""ui:approvals", "#, "")
        + "[[terminal_programs]]
name = \"claude\"
reason = \"Runs Claude\"
";
    let (shown, removed) =
        crate::store_actions::update_lines(&before, consent_items(&manifest(&newer)));
    let keys: Vec<String> = shown.iter().map(|line| line.item.key()).collect();
    assert_eq!(keys, ["terminal:create#claude", "terminal:create#git"]);
    let removed = removed.expect("a removal note");
    assert!(removed.starts_with("No longer asks for: "), "{removed}");
    let (shown, removed) = crate::store_actions::update_lines(&before, before.clone());
    assert!(shown.is_empty() && removed.is_none());
}

/// The file install's notes: unsigned, the package, and the version it replaces.
#[test]
fn test_file_install_notes() {
    let info = crate::install_store::PackageInfo {
        extension_id: "demo".into(),
        version: semver::Version::new(1, 2, 0),
        manifest: manifest(MANIFEST),
        zed_extension: false,
    };
    let package = std::path::Path::new("demo.fyxext");
    let fresh = crate::store_actions::file_install_notes(&info, package, None);
    assert_eq!(
        fresh,
        ["Unsigned, not from the registry. Package: demo.fyxext."]
    );
    let over = crate::store_actions::file_install_notes(
        &info,
        package,
        Some(&semver::Version::new(1, 1, 0)),
    );
    assert_eq!(
        over.last().map(String::as_str),
        Some("Installed now: 1.1.0.")
    );
}

/// W2c's broker answers per item: an unticked program is denied while its ticked sibling runs,
/// and the page's switch changes one item.
#[test]
fn test_broker_answers_per_item() {
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
        lines,
        &[
            program("codex"),
            ConsentItem::Capability("network:fetch".into()),
        ],
    );
    let state = |store: &ConsentStore, item: &ConsentItem| store.item_state("demo", item);
    assert_eq!(state(&store, &program("codex")), PermissionState::Granted);
    assert_eq!(state(&store, &program("git")), PermissionState::Denied);
    assert_eq!(
        state(&store, &ConsentItem::Capability("network:fetch".into())),
        PermissionState::Granted
    );
    assert_eq!(
        state(&store, &ConsentItem::EnvAllNew),
        PermissionState::Denied
    );

    store
        .set_item("demo", &program("git"), true)
        .expect("a T2 item");
    assert_eq!(state(&store, &program("git")), PermissionState::Granted);
    assert_eq!(state(&store, &program("codex")), PermissionState::Granted);
    store
        .set_item(
            "demo",
            &ConsentItem::Capability("network:fetch".into()),
            false,
        )
        .expect("a T2 item");
    assert_eq!(
        state(&store, &ConsentItem::Capability("network:fetch".into())),
        PermissionState::Denied
    );
    assert!(
        store
            .set_item("demo", &ConsentItem::Capability("ui:panel".into()), false)
            .is_err(),
        "T0 has no switch"
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

/// The page's "Turn On": the same line, already ticked, behind the same guard.
#[gpui::test]
async fn test_confirm_item_is_preticked_and_guarded(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let fs = fs::FakeFs::new(cx.executor());
    let project = project::Project::test(fs, [], cx).await;
    let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());
    crate::decoration_tests::redraw(cx);
    let line = consent_items(&manifest(MANIFEST))
        .into_iter()
        .find(|line| line.item == program("codex"))
        .unwrap();
    let mut answer = workspace.update_in(cx, |workspace, window, cx| {
        crate::consent_dialog::confirm_item(workspace, "Demo", line, window, cx)
    });
    crate::decoration_tests::redraw(cx);
    let view = dialog(&workspace, cx);
    view.update(cx, |dialog, cx| dialog.install(cx));
    cx.run_until_parked();
    assert!(answer.try_recv().unwrap().is_none(), "inside the guard");
    cx.executor()
        .advance_clock(CLICK_GUARD + Duration::from_millis(1));
    view.update(cx, |dialog, cx| dialog.install(cx));
    cx.run_until_parked();
    assert_eq!(answer.try_recv().unwrap(), Some(vec![program("codex")]));
}

/// §C.3: the page's "Always allow" switch shows the owner's warning, with no groups, behind the
/// same guard; Cancel answers nothing.
#[gpui::test]
async fn test_confirm_always_allow_warns_and_is_guarded(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let fs = fs::FakeFs::new(cx.executor());
    let project = project::Project::test(fs, [], cx).await;
    let (multi_workspace, cx) = cx.add_window_view(|window, cx| {
        workspace::MultiWorkspace::test_new(project.clone(), window, cx)
    });
    let workspace = multi_workspace.read_with(cx, |multi, _| multi.workspace().clone());
    crate::decoration_tests::redraw(cx);
    let mut answer = workspace.update_in(cx, |workspace, window, cx| {
        crate::consent_dialog::confirm_always_allow(
            workspace,
            "Demo",
            ui_ext_wasm::permissions::T3Capability::Read,
            window,
            cx,
        )
    });
    crate::decoration_tests::redraw(cx);
    let view = dialog(&workspace, cx);
    view.update(cx, |dialog, cx| dialog.install(cx));
    cx.run_until_parked();
    assert!(answer.try_recv().unwrap().is_none(), "inside the guard");
    cx.executor()
        .advance_clock(CLICK_GUARD + Duration::from_millis(1));
    view.update(cx, |dialog, cx| dialog.install(cx));
    cx.run_until_parked();
    assert_eq!(answer.try_recv().unwrap(), Some(vec![]));

    let mut cancelled = workspace.update_in(cx, |workspace, window, cx| {
        crate::consent_dialog::confirm_always_allow(
            workspace,
            "Demo",
            ui_ext_wasm::permissions::T3Capability::Write,
            window,
            cx,
        )
    });
    crate::decoration_tests::redraw(cx);
    cx.dispatch_action(menu::Cancel);
    cx.run_until_parked();
    assert!(cancelled.try_recv().is_err(), "dropped without an answer");
}

/// §C.4: the page's wording (a read never shows content), and the journal goes on uninstall.
#[gpui::test]
async fn test_journal_lines_and_uninstall(cx: &mut TestAppContext) {
    use crate::consent_model::{JournalEntry, JournalKind, JournalPage, journal_line};
    assert_eq!(
        journal_line(&JournalKind::ProgramStarted {
            capability: "terminal:create".into(),
            program: "codex".into(),
            args: vec!["-c".into(), "notify=x".into()],
            cwd: Some("D:/demo".into()),
        }),
        "Ran codex -c notify=x in D:/demo"
    );
    assert_eq!(
        journal_line(&JournalKind::TerminalRead {
            terminal: "pwsh".into(),
            scope: "the last 50 lines".into(),
            bytes: 1200,
        }),
        "Read the last 50 lines of the terminal \"pwsh\" (1200 bytes)"
    );
    crate::tests::init_test(cx);
    let fs: std::sync::Arc<dyn fs::Fs> = fs::FakeFs::new(cx.executor());
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let page = JournalPage {
        entries: vec![JournalEntry {
            at_ms: ui_ext_wasm::journal::now_ms(),
            kind: JournalKind::ListenerOpened {
                listener: "ide".into(),
                port: 4100,
            },
        }],
        incomplete: false,
    };
    registry.update(cx, |registry, cx| {
        registry.set_journal("demo", page.clone());
        registry.set_consent_lines("demo", consent_items(&manifest(MANIFEST)));
        assert_eq!(registry.journal("demo"), page);
        assert!(!registry.consent_lines("demo").is_empty());
        registry.forget_consent("demo", fs, cx);
        assert_eq!(registry.journal("demo"), JournalPage::default());
    });
}

/// Plan 5.7's check: a new program in an update shows in the diff, the card says so, and the
/// dialog for it lists only what is new.
#[gpui::test]
async fn test_held_update_shows_a_new_program(cx: &mut TestAppContext) {
    let old = manifest(
        r#"
api_version = "0.3.0"
component = "x.wasm"
capabilities = ["ui:panel", "process:spawn", "network:fetch"]
[network]
domains = ["api.example.com"]
[[processes]]
name = "codex"
"#,
    );
    let mut new = old.clone();
    new.processes.push(ui_ext::process::ProcessSpec {
        name: "git".into(),
        args: vec!["status".into()],
        path: None,
        reason: None,
    });
    let new_capabilities: Vec<String> = new
        .consent_items()
        .into_iter()
        .filter(|item| !old.consent_items().contains(item))
        .collect();
    assert!(!new_capabilities.is_empty());
    crate::tests::init_test(cx);
    let registry = cx.new(|_| crate::UiExtensionRegistry::trusting());
    let lines = consent_items(&new);
    let diff = registry.update(cx, |registry, _| {
        let consent = registry.consent_mut();
        consent.declare("demo", new.consent_items());
        consent.install(
            "demo",
            ConsentRecord {
                capabilities: old.consent_items(),
                grant: Grant::Component {
                    sha256: "abc".into(),
                },
            },
            consent_items(&old),
            &[ConsentItem::Program {
                capability: "process:spawn".into(),
                name: "codex".into(),
            }],
        );
        consent.diff("demo", &lines)
    });
    let keys: Vec<String> = crate::consent_model::new_lines(&diff)
        .iter()
        .map(|line| line.item.key())
        .collect();
    assert_eq!(keys, ["process:spawn#git"], "only the new program");
    assert!(diff.needs_approval());
    let pending = crate::registry::PendingConsent {
        extension_id: "demo".into(),
        dir: "D:/demo".into(),
        installed_dir: "D:/installed".into(),
        capabilities: new.capabilities.clone(),
        is_dev: false,
        new_capabilities,
        zed_extension: false,
        record: record(),
        lines,
        diff,
    };
    let text = crate::registry::consent_card_text(&pending);
    assert!(
        text.contains("New in this version: Runs git (with status)."),
        "{text}"
    );
}
