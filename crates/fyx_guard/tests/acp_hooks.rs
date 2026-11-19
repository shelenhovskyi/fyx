//! fyx_guard's hooks in `acp_thread`'s `read_text_file` and `write_text_file`, kept here rather than
//! in `acp_thread`'s own test module so the upstream file carries only the hook lines.

use std::{path::Path, rc::Rc};

use acp_thread::{AcpThread, AgentConnection as _, StubAgentConnection};
use gpui::{Entity, TestAppContext, UpdateGlobal as _};
use project::{FakeFs, Project};
use serde_json::json;
use settings::{FyxSecurityMode, SettingsStore};
use util::{path, path_list::PathList};

fn init_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut settings_store = SettingsStore::test(cx);
        settings_store.register_setting::<feature_flags::FeatureFlagsSettings>();
        cx.set_global(settings_store);
        fyx_guard::init_without_persistence(cx);
    });
}

fn set_mode(mode: FyxSecurityMode, cx: &mut TestAppContext) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.fyx_security.get_or_insert_default().mode = Some(mode);
            });
        });
    });
    cx.run_until_parked();
}

async fn thread(cx: &mut TestAppContext) -> Entity<AcpThread> {
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/tmp"),
        json!({ ".env": "API_KEY=1", "main.rs": "", ".git": { "config": "" } }),
    )
    .await;
    let project = Project::test(fs, [path!("/tmp").as_ref()], cx).await;
    let connection = Rc::new(StubAgentConnection::new());
    cx.update(|cx| connection.new_session(project, PathList::new(&[Path::new(path!("/tmp"))]), cx))
        .await
        .unwrap()
}

async fn read(
    thread: &Entity<AcpThread>,
    path: &str,
    cx: &mut TestAppContext,
) -> Result<String, String> {
    thread
        .update(cx, |thread, cx| {
            thread.read_text_file(path.into(), None, None, false, cx)
        })
        .await
        .map_err(|error| error.message)
}

async fn write(
    thread: &Entity<AcpThread>,
    path: &str,
    cx: &mut TestAppContext,
) -> Result<(), String> {
    thread
        .update(cx, |thread, cx| {
            thread.write_text_file(path.into(), "{}".into(), cx)
        })
        .await
        .map_err(|error| format!("{error:#}"))
}

#[gpui::test]
async fn off_is_a_no_op_on_acp_file_access(cx: &mut TestAppContext) {
    init_test(cx);
    let thread = thread(cx).await;
    cx.run_until_parked();

    assert_eq!(
        read(&thread, path!("/tmp/.env"), cx).await.unwrap(),
        "API_KEY=1"
    );
    write(&thread, path!("/tmp/.claude/settings.json"), cx)
        .await
        .unwrap();
    write(&thread, path!("/tmp/.git/config"), cx).await.unwrap();
}

#[gpui::test]
async fn shield_on_acp_file_access(cx: &mut TestAppContext) {
    init_test(cx);
    let thread = thread(cx).await;
    set_mode(FyxSecurityMode::Shield, cx);

    let error = read(&thread, path!("/tmp/.env"), cx).await.unwrap_err();
    assert!(error.contains("private_files"), "{error}");
    let error = write(&thread, path!("/tmp/.claude/settings.json"), cx)
        .await
        .unwrap_err();
    assert!(error.contains("read-only"), "{error}");
    let error = write(&thread, path!("/tmp/.git/config"), cx)
        .await
        .unwrap_err();
    assert!(error.contains(".git/"), "{error}");
    read(&thread, path!("/tmp/main.rs"), cx).await.unwrap();
    write(&thread, path!("/tmp/main.rs"), cx).await.unwrap();
}
