//! The checks the extension host applies to `agent:observe` and `workspace:edit` (integration-5 S1,
//! S2): they hold in every security mode, the default `off` included.

use std::path::Path;

use gpui::{Entity, TestAppContext, UpdateGlobal as _};
use project::{FakeFs, Project, ProjectPath};
use serde_json::json;
use settings::{FyxSecurityMode, SettingsStore};
use util::{path, rel_path::RelPath};

async fn project(mode: FyxSecurityMode, cx: &mut TestAppContext) -> Entity<Project> {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        fyx_guard::init_without_persistence(cx);
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/root"),
        json!({
            ".env": "TOKEN=x",
            ".fyx": { "security.toml": "private_files = [\"vault/**\"]\nread_only = [\"infra/**\"]" },
            ".mcp.json": "{}",
            "infra": { "main.tf": "" },
            "pkg": { ".claude": { "settings.json": "{}" }, ".zed": { "settings.json": "{}" } },
            "src": { "main.rs": "" },
            "vault": { "notes.txt": "" },
        }),
    )
    .await;
    let project = Project::test(fs, [path!("/root").as_ref()], cx).await;
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.fyx_security.get_or_insert_default().mode = Some(mode);
            });
        });
    });
    cx.run_until_parked();
    project
}

fn project_path(project: &Entity<Project>, path: &str, cx: &mut TestAppContext) -> ProjectPath {
    cx.update(|cx| {
        let worktree_id = project.read(cx).worktrees(cx).next().unwrap().read(cx).id();
        ProjectPath {
            worktree_id,
            path: RelPath::from_unix_str(path).unwrap().into(),
        }
    })
}

#[gpui::test]
async fn private_files_stay_private_with_the_shield_off(cx: &mut TestAppContext) {
    let project = project(FyxSecurityMode::Off, cx).await;
    for (path, private) in [
        (".env", true),
        ("vault/notes.txt", true),
        ("src/main.rs", false),
    ] {
        let path = project_path(&project, path, cx);
        let denial = cx.update(|cx| fyx_guard::extension_read_denial(project.read(cx), &path, cx));
        assert_eq!(denial.is_some(), private, "{path:?}: {denial:?}");
    }
}

#[gpui::test]
async fn agent_configs_are_never_writable_by_extensions(cx: &mut TestAppContext) {
    let project = project(FyxSecurityMode::Off, cx).await;
    for (path, refused) in [
        (path!("/root/.mcp.json"), true),
        (path!("/root/pkg/.claude/settings.json"), true),
        (path!("/root/pkg/.zed/settings.json"), true),
        (path!("/root/infra/main.tf"), true),
        (path!("/root/src/main.rs"), false),
    ] {
        let denial = cx
            .update(|cx| fyx_guard::extension_write_denial(project.read(cx), Path::new(path), cx));
        assert_eq!(denial.is_some(), refused, "{path}: {denial:?}");
    }
}
