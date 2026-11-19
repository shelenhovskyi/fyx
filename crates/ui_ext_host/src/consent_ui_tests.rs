//! The 0.4 consent UI (plan 5.7): the model stand-in, the stand-in broker and the install dialog.

use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, VisualTestContext};
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

/// §C.4: the page's wording (a read never shows content), and the journal goes on uninstall.
#[gpui::test]
async fn test_journal_lines_and_uninstall(cx: &mut TestAppContext) {
    use crate::consent_model::{JournalEntry, JournalKind, JournalPage, journal_line};
    assert_eq!(
        journal_line(&JournalKind::ProgramStarted {
            name: "codex".into(),
            argv: vec!["-c".into(), "notify=x".into()],
            cwd: "D:/demo".into(),
            exit: Some(0),
        }),
        "Ran codex -c notify=x in D:/demo, exited 0"
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
            at: std::time::SystemTime::now(),
            kind: JournalKind::ListenerOpened { port: 4100 },
        }],
        incomplete: false,
    };
    registry.update(cx, |registry, cx| {
        registry.set_journal("demo", page.clone());
        registry.set_consent_lines("demo", consent_items(&manifest(MANIFEST)));
        assert_eq!(registry.journal("demo"), page);
        assert_eq!(registry.consent_lines("demo").len(), 9);
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
        consent.grant(
            "demo",
            ConsentRecord {
                capabilities: old.consent_items(),
                grant: Grant::Component {
                    sha256: "abc".into(),
                },
            },
        );
        consent.set_t2("demo", "process:spawn", true);
        registry.held_update_diff("demo", &lines, &new_capabilities)
    });
    let keys: Vec<String> = diff
        .added
        .iter()
        .chain(&diff.widened)
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
        text.contains("New in this version: Runs git (status)."),
        "{text}"
    );
}
