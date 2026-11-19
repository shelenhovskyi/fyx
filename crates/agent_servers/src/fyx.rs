//! fyx: account profiles for ACP agents (`fyx.accounts`, design §C.6). Fyx's code in this crate
//! lives in this file, so rebases never conflict on it; `acp.rs` and `acp/v1.rs` call it from
//! `fyx:` lines.
//!
//! An agent of a tool with accounts (`fyx.accounts.tools.<tool>.agents`) gets the project's account
//! for that tool: only the tool's config-dir variable, e.g. `CLAUDE_CONFIG_DIR`, in its environment.
//! Its sign-in terminal gets the same variable, so signing in lands in the same dir. Fyx never reads
//! the dir and never changes an agent's account on its own: the account is fixed when the agent
//! connects. The agent panel's "+" menu ("Claude Code — Work") picks the project's account and, for
//! an agent connected with another one, a new connection for the new thread (§C.6 rule 3).

use std::path::{Path, PathBuf};

use collections::HashMap;
use gpui::{App, Entity, EntityId, Global, WeakEntity};
use project::{AgentId, AgentServerStore, Project};
use settings::{FyxAccount, FyxAccountsSettingsContent, SettingsStore};

/// The account each connected agent got, by agent server store (one per project) and agent;
/// `None` for an agent that connected without one.
#[derive(Default)]
struct ConnectedAccounts(HashMap<(EntityId, AgentId), Option<FyxAccount>>);

impl Global for ConnectedAccounts {}

/// The account the agent's next connection gets, chosen in the "+" menu: the choice also goes to
/// user settings, which are written back later than the new thread connects.
#[derive(Default)]
struct NextAccounts(HashMap<(EntityId, AgentId), FyxAccount>);

impl Global for NextAccounts {}

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
        .cloned()
        .flatten();
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

/// For the agent panel's "+" menu ("Claude Code — Work"): every account of the agent's tool, in
/// settings order, and whether the project uses it now. Empty for a tool without accounts and for
/// a remote project.
pub fn account_choices(
    agent_id: &AgentId,
    project: &Entity<Project>,
    cx: &App,
) -> Vec<(FyxAccount, bool)> {
    if !project.read(cx).is_local() {
        return Vec::new();
    }
    let Some(accounts) = accounts(cx) else {
        return Vec::new();
    };
    let Some(tool) = accounts.tool_for_agent(agent_id.0.as_ref()) else {
        return Vec::new();
    };
    let current = project_account(agent_id, project, cx);
    accounts
        .accounts()
        .into_iter()
        .filter(|account| account.tool == tool)
        .map(|account| {
            let chosen = current.as_ref() == Some(&account);
            (account, chosen)
        })
        .collect()
}

/// A "+" menu account entry was chosen: from now on the project uses `account` for the agent's
/// tool ("In this project, use Work", kept in user settings by project folder, never in the
/// repository), and the agent's next connection gets it. Returns whether the agent connected
/// with another account (or none): then the panel needs a new connection and a new thread, and
/// threads already open keep theirs (§C.6 rule 3).
pub fn choose_account(
    agent_id: &AgentId,
    project: &Entity<Project>,
    account: &FyxAccount,
    cx: &mut App,
) -> bool {
    let project_ref = project.read(cx);
    if !project_ref.is_local() {
        return false;
    }
    let Some(folder) = project_ref
        .visible_worktrees(cx)
        .next()
        .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
    else {
        return false;
    };
    let fs = project_ref.fs().clone();
    let key = (
        project_ref.agent_server_store().entity_id(),
        agent_id.clone(),
    );
    let connected = cx
        .try_global::<ConnectedAccounts>()
        .and_then(|connected| connected.0.get(&key))
        .cloned();
    cx.default_global::<NextAccounts>()
        .0
        .insert(key, account.clone());
    let (tool, name) = (account.tool.clone(), account.name.clone());
    settings::update_settings_file(fs, cx, move |content, _| {
        set_project_account(content, &folder, &tool, &name)
    });
    connected.is_some_and(|connected| connected.as_ref() != Some(account))
}

/// Writes "in this folder, `tool` uses `account`" into user settings.
fn set_project_account(
    content: &mut settings::SettingsContent,
    folder: &Path,
    tool: &str,
    account: &str,
) {
    content
        .fyx
        .get_or_insert_default()
        .accounts
        .get_or_insert_default()
        .projects
        .get_or_insert_default()
        .entry(folder.to_string_lossy().into_owned())
        .or_default()
        .insert(tool.to_owned(), account.to_owned());
}

/// At connect: puts the account into the agent's environment (the one just chosen in the "+"
/// menu, else the project's) and remembers it for the agent's sign-in terminal.
pub(crate) fn add_account_env(
    agent_id: &AgentId,
    project: &Entity<Project>,
    store: &WeakEntity<AgentServerStore>,
    env: &mut Option<HashMap<String, String>>,
    cx: &mut App,
) {
    let key = (store.entity_id(), agent_id.clone());
    let account = cx
        .default_global::<NextAccounts>()
        .0
        .remove(&key)
        .or_else(|| project_account(agent_id, project, cx));
    if let Some(account) = &account {
        let (name, value) = account.env_entry();
        log::info!("agent {} uses the account dir in {name}", agent_id.0);
        env.get_or_insert_default().insert(name, value);
    }
    cx.default_global::<ConnectedAccounts>()
        .0
        .insert(key, account);
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
        .and_then(|connected| connected.0.get(&(store.entity_id(), agent_id.clone())))
        .and_then(Option::as_ref);
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

    /// The "+" menu's "Claude Code — Work" (§C.6): the tool's accounts with the project's ticked;
    /// choosing one goes to user settings by project folder and to the next connection at once,
    /// and asks for a new connection only when the agent connected with another account.
    #[gpui::test]
    async fn choosing_an_account_in_the_menu(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
        });
        let fs = fs::FakeFs::new(cx.executor());
        fs.insert_tree("/", serde_json::json!({ "work": {} })).await;
        let project = project::Project::test(fs, [Path::new("/work")], cx).await;
        let root = project.read_with(cx, |project, cx| {
            project
                .visible_worktrees(cx)
                .next()
                .unwrap()
                .read(cx)
                .abs_path()
                .to_string_lossy()
                .replace('\\', "\\\\")
        });
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store
                    .set_user_settings(
                        &format!(
                            r#"{{ "fyx": {{ "accounts": {{
                                "tools": {{ "claude": {{ "env": "CLAUDE_CONFIG_DIR", "agents": ["claude-acp"] }} }},
                                "profiles": [
                                    {{ "name": "Personal", "tool": "claude", "dir": "/p" }},
                                    {{ "name": "Work", "tool": "claude", "dir": "/w" }},
                                    {{ "name": "Other", "tool": "codex", "dir": "/c" }}
                                ],
                                "projects": {{ "{root}": {{ "claude": "Work" }} }}
                            }} }} }}"#
                        ),
                        cx,
                    )
                    .result()
                    .unwrap();
            });
        });
        let claude = AgentId::new("claude-acp");
        let choices = |cx: &mut gpui::TestAppContext| {
            cx.update(|cx| account_choices(&claude, &project, cx))
                .into_iter()
                .map(|(account, chosen)| (account.name, chosen))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            choices(cx),
            [("Personal".to_owned(), false), ("Work".to_owned(), true)],
            "only the tool's accounts, the project's ticked"
        );
        assert!(
            cx.update(|cx| account_choices(&AgentId::new("gemini"), &project, cx))
                .is_empty()
        );
        let account = |name: &str, cx: &mut gpui::TestAppContext| {
            cx.update(|cx| accounts(cx).unwrap().account("claude", name).unwrap())
        };
        let connect = |cx: &mut gpui::TestAppContext| {
            cx.update(|cx| {
                let store = project.read(cx).agent_server_store().downgrade();
                let mut env = None;
                add_account_env(&claude, &project, &store, &mut env, cx);
                env.unwrap_or_default().remove("CLAUDE_CONFIG_DIR")
            })
        };
        let dir = |dir: &str| Some(Path::new(dir).to_string_lossy().into_owned());

        // Not connected yet: no new connection needed; the next one gets the choice at once.
        let work = account("Work", cx);
        let personal = account("Personal", cx);
        assert!(!cx.update(|cx| choose_account(&claude, &project, &personal, cx)));
        assert_eq!(connect(cx), dir("/p"));
        // Connected with Personal, Work is chosen: a new connection, which gets Work.
        assert!(cx.update(|cx| choose_account(&claude, &project, &work, cx)));
        assert_eq!(connect(cx), dir("/w"));
        // The same account again: the connection stays.
        assert!(!cx.update(|cx| choose_account(&claude, &project, &work, cx)));
        // Another account while connected with Work: a new connection, which gets Personal.
        assert!(cx.update(|cx| choose_account(&claude, &project, &personal, cx)));
        assert_eq!(connect(cx), dir("/p"));

        // The choice was written for the project's folder ("In this project, use Personal"). The
        // test's settings file holds only what was written.
        cx.run_until_parked();
        let written = cx.update(|cx| {
            let accounts = accounts(cx).unwrap();
            accounts
                .project_for(Path::new("/work/src"))
                .map(|(_, choices)| choices.get("claude").cloned())
        });
        assert_eq!(written, Some(Some("Personal".to_owned())));
    }
}
