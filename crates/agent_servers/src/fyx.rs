//! fyx: account profiles for ACP agents (`fyx.accounts`, design §C.6). Fyx's code in this crate
//! lives in this file, so rebases never conflict on it; `acp.rs` and `acp/v1.rs` call it from
//! `fyx:` lines.
//!
//! An agent of a tool with accounts (`fyx.accounts.tools.<tool>.agents`) gets the project's account
//! for that tool: only the tool's config-dir variable, e.g. `CLAUDE_CONFIG_DIR`, in its environment.
//! Its sign-in terminal gets the same variable, so signing in lands in the same dir. Fyx never reads
//! the dir and never changes an agent's account on its own: the account is fixed when the agent
//! connects.

use std::path::PathBuf;

use collections::HashMap;
use gpui::{App, Entity, EntityId, Global, WeakEntity};
use project::{AgentId, AgentServerStore, Project};
use settings::{FyxAccount, FyxAccountsSettingsContent, SettingsStore};

/// The account each connected agent got, by agent server store (one per project) and agent.
#[derive(Default)]
struct ConnectedAccounts(HashMap<(EntityId, AgentId), FyxAccount>);

impl Global for ConnectedAccounts {}

fn accounts(cx: &App) -> Option<FyxAccountsSettingsContent> {
    cx.try_global::<SettingsStore>()?
        .merged_settings()
        .fyx
        .as_ref()?
        .accounts
        .clone()
}

/// The project's account for the agent's tool.
fn project_account(agent_id: &AgentId, project: &Entity<Project>, cx: &App) -> Option<FyxAccount> {
    let project = project.read(cx);
    if !project.is_local() {
        return None;
    }
    let roots = project
        .visible_worktrees(cx)
        .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
        .collect::<Vec<PathBuf>>();
    let accounts = accounts(cx)?;
    let tool = accounts.tool_for_agent(agent_id.0.as_ref())?;
    accounts.project_account(tool, roots.iter().map(PathBuf::as_path))
}

/// The account an agent shows in the agent panel (design §C.6: always visible).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAccount {
    pub account: FyxAccount,
    /// The tool's menu name, e.g. "Claude Code".
    pub tool_label: String,
    /// The agent connected with it; otherwise it is the one it will get.
    pub connected: bool,
}

/// For the agent panel: the account `agent_id` connected with in `project`, or else the one it
/// gets when it connects. `None` for a tool without accounts and for remote projects.
pub fn agent_account(
    agent_id: &AgentId,
    project: &Entity<Project>,
    cx: &App,
) -> Option<AgentAccount> {
    let store = project.read(cx).agent_server_store().entity_id();
    let connected = cx
        .try_global::<ConnectedAccounts>()
        .and_then(|connected| connected.0.get(&(store, agent_id.clone())))
        .cloned();
    let (account, connected) = match connected {
        Some(account) => (account, true),
        None => (project_account(agent_id, project, cx)?, false),
    };
    let tool_label = accounts(cx)
        .map(|accounts| accounts.tool_label(&account.tool))
        .unwrap_or_else(|| account.tool.clone());
    Some(AgentAccount {
        account,
        tool_label,
        connected,
    })
}

/// At connect: puts the project's account into the agent's environment and remembers it for the
/// agent's sign-in terminal.
pub(crate) fn add_account_env(
    agent_id: &AgentId,
    project: &Entity<Project>,
    store: &WeakEntity<AgentServerStore>,
    env: &mut Option<HashMap<String, String>>,
    cx: &mut App,
) {
    let key = (store.entity_id(), agent_id.clone());
    match project_account(agent_id, project, cx) {
        Some(account) => {
            let (name, value) = account.env_entry();
            log::info!("agent {} uses the account dir in {name}", agent_id.0);
            env.get_or_insert_default().insert(name, value);
            cx.default_global::<ConnectedAccounts>()
                .0
                .insert(key, account);
        }
        None => {
            cx.default_global::<ConnectedAccounts>().0.remove(&key);
        }
    }
}

/// For the agent's sign-in terminal: the account the agent connected with.
pub(crate) fn add_login_env(
    agent_id: &AgentId,
    store: &WeakEntity<AgentServerStore>,
    env: &mut impl Extend<(String, String)>,
    cx: &App,
) {
    let entry = cx
        .try_global::<ConnectedAccounts>()
        .and_then(|connected| connected.0.get(&(store.entity_id(), agent_id.clone())));
    if let Some(account) = entry {
        env.extend([account.env_entry()]);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gpui::UpdateGlobal as _;

    use super::*;

    #[gpui::test]
    async fn an_agent_and_its_sign_in_get_the_projects_account(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
        });
        let fs = fs::FakeFs::new(cx.executor());
        fs.insert_tree("/", serde_json::json!({ "work": {}, "home": {} }))
            .await;
        let work = project::Project::test(fs.clone(), [Path::new("/work")], cx).await;
        let home = project::Project::test(fs, [Path::new("/home")], cx).await;
        let root = |project: &Entity<Project>, cx: &mut gpui::TestAppContext| {
            project.read_with(cx, |project, cx| {
                project
                    .visible_worktrees(cx)
                    .next()
                    .unwrap()
                    .read(cx)
                    .abs_path()
                    .to_string_lossy()
                    .replace('\\', "\\\\")
            })
        };
        let work_root = root(&work, cx);
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store
                    .set_user_settings(
                        &format!(
                            r#"{{ "fyx": {{ "accounts": {{
                                "tools": {{ "claude": {{ "env": "CLAUDE_CONFIG_DIR", "agents": ["claude-acp"] }} }},
                                "profiles": [ {{ "name": "Work", "tool": "claude", "dir": "/w" }} ],
                                "projects": {{ "{work_root}": {{ "claude": "Work" }} }}
                            }} }} }}"#
                        ),
                        cx,
                    )
                    .result()
                    .unwrap();
            });
        });

        let claude = AgentId::new("claude-acp");
        let connect =
            |project: &Entity<Project>, agent: &AgentId, cx: &mut gpui::TestAppContext| {
                cx.update(|cx| {
                    let store = project.read(cx).agent_server_store().downgrade();
                    let mut env = None;
                    add_account_env(agent, project, &store, &mut env, cx);
                    let mut login = HashMap::default();
                    add_login_env(agent, &store, &mut login, cx);
                    (env.unwrap_or_default(), login)
                })
            };
        let shown = |project: &Entity<Project>, cx: &mut gpui::TestAppContext| {
            cx.update(|cx| agent_account(&claude, project, cx))
                .map(|account| (account.account.name, account.connected))
        };
        assert_eq!(
            shown(&work, cx),
            Some(("Work".to_owned(), false)),
            "before connecting"
        );
        let (env, login) = connect(&work, &claude, cx);
        assert_eq!(shown(&work, cx), Some(("Work".to_owned(), true)));
        assert_eq!(shown(&home, cx), None);
        let dir = Path::new("/w").to_string_lossy().into_owned();
        assert_eq!(env.get("CLAUDE_CONFIG_DIR"), Some(&dir));
        assert_eq!(
            login.get("CLAUDE_CONFIG_DIR"),
            Some(&dir),
            "sign in where the agent reads"
        );
        assert_eq!(env.len(), 1, "only the config-dir variable");

        let (env, login) = connect(&home, &claude, cx);
        assert!(
            env.is_empty() && login.is_empty(),
            "another project, no choice"
        );
        let (env, _) = connect(&work, &AgentId::new("codex-acp"), cx);
        assert!(env.is_empty(), "another tool");
    }
}
