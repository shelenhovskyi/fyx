//! fyx_guard's hooks in the built-in agent's path tools: create directory, delete, move and copy.

use std::sync::Arc;

use action_log::ActionLog;
use agent::{
    AgentTool, CopyPathTool, CopyPathToolInput, CreateDirectoryTool, CreateDirectoryToolInput,
    DeletePathTool, DeletePathToolInput, MovePathTool, MovePathToolInput, ToolCallEventStream,
    ToolInput,
};
use agent_settings::AgentSettings;
use fs::Fs as _;
use gpui::{AppContext as _, Entity, TestAppContext, UpdateGlobal as _};
use project::{FakeFs, Project};
use serde_json::json;
use settings::{FyxSecurityMode, Settings as _, SettingsStore};
use util::path;

fn set_mode(mode: FyxSecurityMode, cx: &mut TestAppContext) {
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, cx| {
            store.update_user_settings(cx, |settings| {
                settings.fyx_security.get_or_insert_default().mode = Some(mode);
            });
        });
        // No permission prompts, so a tool that is not blocked runs to the end.
        let mut settings = AgentSettings::get_global(cx).clone();
        settings.tool_permissions.default = settings::ToolPermissionMode::Allow;
        AgentSettings::override_global(settings, cx);
    });
    cx.run_until_parked();
}

struct Tools {
    fs: Arc<FakeFs>,
    project: Entity<Project>,
    action_log: Entity<ActionLog>,
}

async fn tools(mode: FyxSecurityMode, cx: &mut TestAppContext) -> Tools {
    cx.update(|cx| {
        let settings_store = SettingsStore::test(cx);
        cx.set_global(settings_store);
        fyx_guard::init_without_persistence(cx);
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/root"),
        json!({
            ".claude": { "commands": { "review.md": "review" } },
            ".fyx": { "security.toml": "read_only = [\"infra/**\"]" },
            "infra": { "main.tf": "" },
            "notes": { "settings.json": "{}", "mcp.json": "{}" },
            "pkg": { ".claude": { "settings.json": "{}" }, "lib.rs": "" },
            "src": { "a.rs": "", "b.rs": "" },
        }),
    )
    .await;
    let project = Project::test(fs.clone(), [path!("/root").as_ref()], cx).await;
    let action_log = cx.new(|_| ActionLog::new(project.clone()));
    set_mode(mode, cx);
    Tools {
        fs,
        project,
        action_log,
    }
}

impl Tools {
    async fn run<T: AgentTool<Output = String>>(
        &self,
        tool: T,
        input: T::Input,
        cx: &mut TestAppContext,
    ) -> Result<String, String> {
        let (event_stream, _events) = ToolCallEventStream::test();
        let task = cx.update(|cx| Arc::new(tool).run(ToolInput::resolved(input), event_stream, cx));
        let result = task.await;
        cx.run_until_parked();
        result
    }

    async fn move_path(
        &self,
        from: &str,
        to: &str,
        cx: &mut TestAppContext,
    ) -> Result<String, String> {
        let input = MovePathToolInput {
            source_path: from.into(),
            destination_path: to.into(),
        };
        self.run(MovePathTool::new(self.project.clone()), input, cx)
            .await
    }

    async fn copy_path(
        &self,
        from: &str,
        to: &str,
        cx: &mut TestAppContext,
    ) -> Result<String, String> {
        let input = CopyPathToolInput {
            source_path: from.into(),
            destination_path: to.into(),
        };
        self.run(CopyPathTool::new(self.project.clone()), input, cx)
            .await
    }

    async fn delete_path(&self, path: &str, cx: &mut TestAppContext) -> Result<String, String> {
        let tool = DeletePathTool::new(self.project.clone(), self.action_log.clone());
        self.run(tool, DeletePathToolInput { path: path.into() }, cx)
            .await
    }

    async fn create_directory(
        &self,
        path: &str,
        cx: &mut TestAppContext,
    ) -> Result<String, String> {
        let tool = CreateDirectoryTool::new(self.project.clone());
        self.run(
            tool,
            CreateDirectoryToolInput {
                path: path.into(),
                reason: None,
            },
            cx,
        )
        .await
    }

    async fn exists(&self, path: &str) -> bool {
        self.fs.is_file(path.as_ref()).await || self.fs.is_dir(path.as_ref()).await
    }
}

fn assert_blocked(result: Result<String, String>, rule: &str) {
    let error = result.expect_err("the shield should block this");
    assert!(error.contains("security shield"), "{error}");
    assert!(error.contains(rule), "expected {rule:?} in: {error}");
}

#[gpui::test]
async fn off_is_a_no_op_for_path_tools(cx: &mut TestAppContext) {
    let tools = tools(FyxSecurityMode::Off, cx).await;

    tools
        .move_path("root/notes/settings.json", "root/.claude/settings.json", cx)
        .await
        .unwrap();
    tools
        .copy_path("root/notes/mcp.json", "root/.mcp.json", cx)
        .await
        .unwrap();
    tools.create_directory("root/.git/hooks", cx).await.unwrap();
    tools.move_path("root/pkg", "root/pkg2", cx).await.unwrap();
    tools
        .delete_path("root/.fyx/security.toml", cx)
        .await
        .unwrap();
    assert!(tools.exists(path!("/root/.claude/settings.json")).await);
    assert!(tools.exists(path!("/root/.mcp.json")).await);
    assert!(tools.exists(path!("/root/.git/hooks")).await);
    assert!(
        tools
            .exists(path!("/root/pkg2/.claude/settings.json"))
            .await
    );
    assert!(!tools.exists(path!("/root/.fyx/security.toml")).await);
}

#[gpui::test]
async fn shield_blocks_path_tools_on_agent_configs(cx: &mut TestAppContext) {
    let tools = tools(FyxSecurityMode::Shield, cx).await;

    // The scenario of the review: write a harmless file, then move it into an agent config.
    assert_blocked(
        tools
            .move_path("root/notes/settings.json", "root/.claude/settings.json", cx)
            .await,
        ".claude/",
    );
    assert!(tools.exists(path!("/root/notes/settings.json")).await);
    assert!(!tools.exists(path!("/root/.claude/settings.json")).await);

    assert_blocked(
        tools
            .copy_path("root/notes/mcp.json", "root/.mcp.json", cx)
            .await,
        ".mcp.json",
    );
    assert_blocked(
        tools
            .copy_path("root/notes/settings.json", "root/.fyx/x.toml", cx)
            .await,
        ".fyx/",
    );
    assert_blocked(
        tools.delete_path("root/.fyx/security.toml", cx).await,
        ".fyx/",
    );
    assert!(tools.exists(path!("/root/.fyx/security.toml")).await);
    assert_blocked(tools.create_directory("root/.git/hooks", cx).await, ".git/");
    assert_blocked(
        tools.create_directory("root/.claude/hooks", cx).await,
        ".claude/",
    );

    // Trees: what lands below the destination, and what goes away below the source.
    assert_blocked(
        tools.copy_path("root/pkg", "root/pkg2", cx).await,
        ".claude/",
    );
    assert_blocked(
        tools.move_path("root/pkg", "root/pkg3", cx).await,
        ".claude/",
    );
    assert_blocked(tools.delete_path("root/pkg", cx).await, ".claude/");
    assert!(tools.exists(path!("/root/pkg/.claude/settings.json")).await);
    assert!(!tools.exists(path!("/root/pkg2")).await);

    // The repository's `read_only` patterns.
    assert_blocked(
        tools
            .move_path("root/src/a.rs", "root/infra/a.rs", cx)
            .await,
        "read_only",
    );

    // Everything else still works, including copying out of an agent config (a read).
    tools
        .copy_path(
            "root/.claude/commands/review.md",
            "root/notes/review.md",
            cx,
        )
        .await
        .unwrap();
    tools.create_directory("root/src/new", cx).await.unwrap();
    tools
        .move_path("root/src/a.rs", "root/src/c.rs", cx)
        .await
        .unwrap();
    tools.copy_path("root/src", "root/src2", cx).await.unwrap();
    tools.delete_path("root/src/b.rs", cx).await.unwrap();
    assert!(tools.exists(path!("/root/notes/review.md")).await);
    assert!(tools.exists(path!("/root/src2/c.rs")).await);
}

#[gpui::test]
async fn audit_only_logs_path_tools(cx: &mut TestAppContext) {
    let tools = tools(FyxSecurityMode::Audit, cx).await;
    tools
        .move_path("root/notes/settings.json", "root/.claude/settings.json", cx)
        .await
        .unwrap();
    tools.delete_path("root/pkg", cx).await.unwrap();
    assert!(tools.exists(path!("/root/.claude/settings.json")).await);
}
