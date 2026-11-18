//! Optional security modes for agents (design: `docs/fyx/security-modes.md`).
//!
//! Every entry point returns early while `fyx_security.mode` is `off`, so Fyx then behaves exactly
//! like Zed. The mode is read from the global settings value only: project settings have no
//! `fyx_security` key, so a repository can neither raise nor lower it. A repository can only
//! tighten, through `.fyx/security.toml` (see [`RepoPolicy`]).

pub mod paths;
mod policy;

use std::path::Path;

use agent_client_protocol::{ErrorCode, v1::schema as acp_v1};
use collections::HashMap;
use fs::Fs;
use gpui::{App, AsyncApp, Entity, Global, WeakEntity};
use project::{
    Project, ProjectPath, Worktree, WorktreeId, WorktreeSettings,
    agent_server_store::AgentServerCommand,
};
use settings::{FyxSecurityMode, RegisterSetting, Settings, SettingsStore};
use util::rel_path::RelPath;

pub use policy::{
    RepoPolicy, RepoState, SECRET_ENV_PATTERNS, SECURITY_FILE, env_name_matches, protected_config,
    protected_config_rel, secret_env_names,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecurityMode {
    Off,
    /// Shield as a dry run: log what would be blocked, block nothing.
    Audit,
    Shield,
}

/// `fyx_security` in user settings (schema in `settings_content::FyxSecuritySettingsContent`,
/// default in `assets/settings/fyx.json`).
#[derive(Clone, Debug, RegisterSetting)]
pub struct SecuritySettings {
    pub mode: SecurityMode,
    pub allow_env: Vec<String>,
}

impl Settings for SecuritySettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let security = content.fyx_security.as_ref();
        Self {
            mode: mode_from_content(security.and_then(|security| security.mode)),
            allow_env: security
                .and_then(|security| security.allow_env.clone())
                .unwrap_or_default(),
        }
    }
}

fn mode_from_content(mode: Option<FyxSecurityMode>) -> SecurityMode {
    match mode {
        None | Some(FyxSecurityMode::Off) => SecurityMode::Off,
        Some(FyxSecurityMode::Audit) => SecurityMode::Audit,
        Some(FyxSecurityMode::Shield) => SecurityMode::Shield,
        Some(FyxSecurityMode::Unrecognized) => {
            log::error!("fyx_guard: unknown `fyx_security.mode`; using \"shield\"");
            SecurityMode::Shield
        }
    }
}

/// The active mode. Without a settings store (some headless and test hosts) nothing can have
/// turned a mode on, so it is off.
pub fn mode(cx: &App) -> SecurityMode {
    if cx.has_global::<SettingsStore>() {
        SecuritySettings::get_global(cx).mode
    } else {
        SecurityMode::Off
    }
}

/// Applies `mode` to a denial: shield turns it into the message for the agent, audit only logs it.
fn enforce(mode: SecurityMode, action: &str, denial: Option<String>) -> Result<(), String> {
    let Some(reason) = denial else {
        return Ok(());
    };
    match mode {
        SecurityMode::Off => Ok(()),
        SecurityMode::Audit => {
            log::warn!("fyx_guard audit: would block {action}: {reason}");
            Ok(())
        }
        SecurityMode::Shield => {
            log::warn!("fyx_guard: blocked {action}: {reason}");
            Err(format!(
                "Fyx's security shield blocked {action}: {reason}. Do not retry or work around \
                 this; ask the user if the task needs it."
            ))
        }
    }
}

/// Hook for ACP `fs/read_text_file`: Zed's `private_files` (the same check as the built-in agent's
/// `read_file` tool) plus the worktree's `.fyx/security.toml`.
pub fn check_private_path(
    project: &Project,
    path: &ProjectPath,
    cx: &App,
) -> Result<(), acp_v1::Error> {
    let mode = mode(cx);
    if mode == SecurityMode::Off {
        return Ok(());
    }
    let action = format!(
        "an agent read of {}",
        display_project_path(project, path, cx)
    );
    enforce(mode, &action, private_denial(path, cx)).map_err(|message| {
        acp_v1::Error::new(ErrorCode::INVALID_PARAMS.value(), message.clone()).data(message)
    })
}

fn private_denial(path: &ProjectPath, cx: &App) -> Option<String> {
    if WorktreeSettings::get_global(cx).is_path_private(&path.path) {
        return Some("it matches the global `private_files` setting".into());
    }
    if WorktreeSettings::get(Some(path.into()), cx).is_path_private(&path.path) {
        return Some("it matches the worktree `private_files` setting".into());
    }
    repo_denial(path.worktree_id, cx, |state| state.read_denial(&path.path))
}

/// Hook for ACP `fs/write_text_file`: agent configs and the worktree's `read_only` patterns. The
/// error carries the message as an ACP error, so the agent sees it rather than "Internal error".
pub fn check_config_write(project: &Project, abs_path: &Path, cx: &App) -> anyhow::Result<()> {
    write_check(project, abs_path, cx).map_err(|message| {
        anyhow::Error::new(
            agent_client_protocol::Error::new(ErrorCode::INVALID_PARAMS, message.clone())
                .data(message),
        )
    })
}

/// [`check_config_write`] for the built-in agent's edit and write tools, which report errors to the
/// model as strings.
pub fn check_agent_edit(
    project: &Entity<Project>,
    abs_path: &Path,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    cx.update(|cx| write_check(project.read(cx), abs_path, cx))
}

fn write_check(project: &Project, abs_path: &Path, cx: &App) -> Result<(), String> {
    let mode = mode(cx);
    if mode == SecurityMode::Off {
        return Ok(());
    }
    let project_path = project.find_project_path(abs_path, cx);
    let read_only_config =
        |rule: String| format!("agent configs ({rule}) are read-only for agents");
    let denial = protected_config(abs_path)
        .map(read_only_config)
        .or_else(|| {
            let project_path = project_path.as_ref()?;
            protected_config_rel(&project_path.path).map(read_only_config)
        })
        .or_else(|| {
            let project_path = project_path.as_ref()?;
            repo_denial(project_path.worktree_id, cx, |state| {
                state.write_denial(&project_path.path)
            })
        });
    let action = format!("an agent write to {}", abs_path.display());
    enforce(mode, &action, denial)
}

/// Hook for the environment an agent terminal inherits, applied before the variables the agent
/// sets for its command. Secret-looking variables not in `allow_env` are set to an empty value
/// rather than removed: the terminal re-resolves the directory environment and inherits Fyx's own
/// process environment, and an explicit empty value is what overrides both.
pub fn scrub_env(env: &mut HashMap<String, String>, cx: &App) {
    let mode = mode(cx);
    if mode == SecurityMode::Off {
        return;
    }
    let inherited = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .collect::<Vec<_>>();
    scrub(
        env,
        &inherited,
        mode,
        &SecuritySettings::get_global(cx).allow_env,
    );
}

fn scrub(
    env: &mut HashMap<String, String>,
    inherited: &[String],
    mode: SecurityMode,
    allow: &[String],
) -> Vec<String> {
    let mut names = env.keys().chain(inherited).cloned().collect::<Vec<_>>();
    names.sort();
    names.dedup();
    let secret = secret_env_names(&names, allow);
    if secret.is_empty() {
        return secret;
    }
    match mode {
        SecurityMode::Off => {}
        SecurityMode::Audit => log::warn!(
            "fyx_guard audit: would blank in an agent terminal: {}",
            secret.join(", ")
        ),
        SecurityMode::Shield => {
            for name in &secret {
                env.insert(name.clone(), String::new());
            }
            log::info!(
                "fyx_guard: blanked in an agent terminal: {}",
                secret.join(", ")
            );
        }
    }
    secret
}

/// Step 2 (`tasks/W9-security.md`): will run an external ACP agent's process tree in an OS
/// sandbox that denies reads outside the project (Seatbelt, bubblewrap, MXC on Windows), and fail
/// closed if the sandbox cannot start. Not wired yet; returns the command unchanged.
pub fn wrap_agent(command: AgentServerCommand, _cx: &App) -> anyhow::Result<AgentServerCommand> {
    Ok(command)
}

/// Step 2: the same isolation for a stdio MCP server. Not wired yet; returns the command
/// unchanged. Its hook in `context_server` cannot call this crate directly (dependency cycle
/// through `project`); see the design doc, hook H9.
pub fn wrap_mcp(
    command: std::process::Command,
    _cx: &App,
) -> anyhow::Result<std::process::Command> {
    Ok(command)
}

fn display_project_path(project: &Project, path: &ProjectPath, cx: &App) -> String {
    match project.absolute_path(path, cx) {
        Some(abs_path) => abs_path.display().to_string(),
        None => path.path.as_unix_str().to_string(),
    }
}

/// Repository policies by worktree, kept current while a mode is on.
#[derive(Default)]
struct Guard {
    projects: Vec<WeakEntity<Project>>,
    worktrees: HashMap<WorktreeId, (u64, RepoState)>,
    next_generation: u64,
    mode: Option<SecurityMode>,
}

impl Global for Guard {}

fn repo_denial(
    worktree_id: WorktreeId,
    cx: &App,
    denial: impl FnOnce(&RepoState) -> Option<String>,
) -> Option<String> {
    let (_, state) = cx.try_global::<Guard>()?.worktrees.get(&worktree_id)?;
    denial(state)
}

pub fn init(cx: &mut App) {
    cx.set_global(Guard::default());
    on_settings_changed(cx);
    cx.observe_global::<SettingsStore>(on_settings_changed)
        .detach();
    cx.observe_new(|_: &mut Project, _, cx| {
        // The project is still being built here; it can be read once the update returns.
        let project = cx.entity();
        cx.defer(move |cx| track_project(project, cx));
    })
    .detach();
}

fn track_project(project: Entity<Project>, cx: &mut App) {
    let guard = cx.global_mut::<Guard>();
    guard.projects.retain(|project| project.upgrade().is_some());
    guard.projects.push(project.downgrade());
    cx.subscribe(&project, on_project_event).detach();
    load_project_policies(&project, cx);
}

fn on_settings_changed(cx: &mut App) {
    let mode = mode(cx);
    let previous = cx.global::<Guard>().mode;
    if previous == Some(mode) {
        return;
    }
    cx.global_mut::<Guard>().mode = Some(mode);
    log::info!("fyx_guard: security mode {mode:?}");
    if mode == SecurityMode::Off {
        cx.global_mut::<Guard>().worktrees.clear();
    } else if previous.is_none_or(|previous| previous == SecurityMode::Off) {
        let projects = cx.global::<Guard>().projects.clone();
        for project in projects.iter().filter_map(WeakEntity::upgrade) {
            load_project_policies(&project, cx);
        }
    }
}

fn on_project_event(project: Entity<Project>, event: &project::Event, cx: &mut App) {
    match event {
        project::Event::WorktreeAdded(worktree_id) => {
            if let Some(worktree) = project.read(cx).worktree_for_id(*worktree_id, cx) {
                load_policy(&project, &worktree, cx);
            }
        }
        project::Event::WorktreeRemoved(worktree_id) => {
            cx.global_mut::<Guard>().worktrees.remove(worktree_id);
        }
        project::Event::WorktreeUpdatedEntries(worktree_id, changes) => {
            // Removing or renaming `.fyx/` reports only the directory.
            let touches_policy = |path: &RelPath| {
                !path.is_empty()
                    && RelPath::from_unix_str(SECURITY_FILE)
                        .is_ok_and(|security_file| security_file.starts_with(path))
            };
            if changes.iter().any(|(path, _, _)| touches_policy(path))
                && let Some(worktree) = project.read(cx).worktree_for_id(*worktree_id, cx)
            {
                load_policy(&project, &worktree, cx);
            }
        }
        _ => {}
    }
}

fn load_project_policies(project: &Entity<Project>, cx: &mut App) {
    let worktrees = project.read(cx).worktrees(cx).collect::<Vec<_>>();
    for worktree in worktrees {
        load_policy(project, &worktree, cx);
    }
}

/// Loads a worktree's `.fyx/security.toml`. Until it has loaded, and if it cannot be read or
/// parsed, the worktree is treated as fully restricted (fail closed).
fn load_policy(project: &Entity<Project>, worktree: &Entity<Worktree>, cx: &mut App) {
    if mode(cx) == SecurityMode::Off {
        return;
    }
    let worktree_id = worktree.read(cx).id();
    let Ok(security_file) = RelPath::from_unix_str(SECURITY_FILE) else {
        return;
    };
    let generation = {
        let guard = cx.global_mut::<Guard>();
        guard.next_generation += 1;
        guard.next_generation
    };
    if !worktree.read(cx).is_local() {
        // Remote worktrees cannot load files here yet: a policy the repository has is unusable.
        let state = if worktree.read(cx).entry_for_path(security_file).is_some() {
            RepoState::Failed("it cannot be read from a remote worktree yet".into())
        } else {
            RepoState::Loaded(None)
        };
        cx.global_mut::<Guard>()
            .worktrees
            .insert(worktree_id, (generation, state));
        return;
    }
    cx.global_mut::<Guard>()
        .worktrees
        .insert(worktree_id, (generation, RepoState::Loading));
    let path = worktree
        .read(cx)
        .abs_path()
        .join(security_file.as_std_path());
    let fs = project.read(cx).fs().clone();
    cx.spawn(async move |cx| {
        let state = read_policy(fs.as_ref(), &path).await;
        if let RepoState::Failed(error) = &state {
            log::error!("fyx_guard: {} is unusable: {error}", path.display());
        }
        cx.update(|cx| {
            // A newer load (the file changed again) or a removed worktree wins.
            if let Some(entry) = cx.global_mut::<Guard>().worktrees.get_mut(&worktree_id)
                && entry.0 == generation
            {
                entry.1 = state;
            }
        });
    })
    .detach();
}

async fn read_policy(fs: &dyn Fs, path: &Path) -> RepoState {
    match fs.metadata(path).await {
        Ok(None) => return RepoState::Loaded(None),
        Ok(Some(_)) => {}
        Err(error) => return RepoState::Failed(format!("{error:#}")),
    }
    match fs.load(path).await {
        Ok(text) => match RepoPolicy::parse(&text) {
            Ok(policy) => RepoState::Loaded(Some(policy)),
            Err(error) => RepoState::Failed(format!("{error:#}")),
        },
        Err(error) => RepoState::Failed(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs::FakeFs;
    use gpui::{BorrowAppContext as _, TestAppContext};
    use project::Project;
    use serde_json::json;
    use settings::SettingsStore;
    use util::{path, rel_path::rel_path};

    #[test]
    fn unknown_modes_fail_closed() {
        assert_eq!(mode_from_content(None), SecurityMode::Off);
        assert_eq!(
            mode_from_content(Some(FyxSecurityMode::Off)),
            SecurityMode::Off
        );
        assert_eq!(
            mode_from_content(Some(FyxSecurityMode::Audit)),
            SecurityMode::Audit
        );
        assert_eq!(
            mode_from_content(Some(FyxSecurityMode::Shield)),
            SecurityMode::Shield
        );
        assert_eq!(
            mode_from_content(Some(FyxSecurityMode::Unrecognized)),
            SecurityMode::Shield
        );
        let parsed: settings::FyxSecuritySettingsContent =
            serde_json::from_value(json!({ "mode": "sheild" })).unwrap();
        assert_eq!(parsed.mode, Some(FyxSecurityMode::Unrecognized));
    }

    #[test]
    fn enforce_blocks_only_in_shield() {
        let denial = || Some("it matches `private_files`".to_string());
        assert!(enforce(SecurityMode::Off, "a read", denial()).is_ok());
        assert!(enforce(SecurityMode::Audit, "a read", denial()).is_ok());
        let error = enforce(SecurityMode::Shield, "a read", denial()).unwrap_err();
        assert!(error.contains("private_files"), "{error}");
        assert!(enforce(SecurityMode::Shield, "a read", None).is_ok());
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn scrub_blanks_in_shield_and_only_logs_in_audit() {
        let original = env(&[
            ("PATH", "/usr/bin"),
            ("GITHUB_TOKEN", "ghp_x"),
            ("OPENAI_API_KEY", "sk-x"),
            ("SSH_AUTH_SOCK", "/tmp/agent"),
        ]);
        let inherited = ["AWS_SECRET_ACCESS_KEY".to_string(), "HOME".to_string()];

        let mut audited = original.clone();
        let secret = scrub(&mut audited, &inherited, SecurityMode::Audit, &[]);
        assert_eq!(audited, original);
        assert_eq!(
            secret,
            [
                "AWS_SECRET_ACCESS_KEY",
                "GITHUB_TOKEN",
                "OPENAI_API_KEY",
                "SSH_AUTH_SOCK"
            ]
        );

        let mut shielded = original;
        scrub(
            &mut shielded,
            &inherited,
            SecurityMode::Shield,
            &["GITHUB_TOKEN".to_string()],
        );
        assert_eq!(shielded["PATH"], "/usr/bin");
        assert_eq!(shielded["GITHUB_TOKEN"], "ghp_x");
        assert_eq!(shielded["OPENAI_API_KEY"], "");
        assert_eq!(shielded["SSH_AUTH_SOCK"], "");
        // Only inherited by the PTY from Fyx's own environment: overridden with an empty value.
        assert_eq!(shielded["AWS_SECRET_ACCESS_KEY"], "");
        assert!(!shielded.contains_key("HOME"));
    }

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            init(cx);
        });
    }

    fn set_mode(mode: FyxSecurityMode, cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings.fyx_security.get_or_insert_default().mode = Some(mode);
                });
            });
        });
    }

    #[gpui::test]
    async fn settings_off_by_default_and_raised_only_by_user_settings(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/p"),
            json!({ ".zed": { "settings.json": r#"{ "fyx_security": { "mode": "shield" } }"# } }),
        )
        .await;
        let project = Project::test(fs, [path!("/p").as_ref()], cx).await;
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Off);

        set_mode(FyxSecurityMode::Audit, cx);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Audit);
        set_mode(FyxSecurityMode::Shield, cx);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        drop(project);
    }

    #[gpui::test]
    async fn repo_policy_only_tightens(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/p"),
            json!({
                ".env": "SECRET=1",
                ".fyx": { "security.toml": "private_files = [\"keys/**\"]\nread_only = [\"Makefile\"]" },
                "keys": { "prod.json": "{}" },
                "src": { "main.rs": "" },
                "Makefile": "",
            }),
        )
        .await;
        let project = Project::test(fs.clone(), [path!("/p").as_ref()], cx).await;
        let worktree_id = project.read_with(cx, |project, cx| {
            project.worktrees(cx).next().unwrap().read(cx).id()
        });
        let project_path = |path: &str| ProjectPath {
            worktree_id,
            path: rel_path(path).into(),
        };
        let read = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_private_path(project, &project_path(path), cx).is_ok()
            })
        };
        let write = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_config_write(project, Path::new(path), cx).is_ok()
            })
        };

        // Off: nothing is checked and the repository file is not even read.
        cx.run_until_parked();
        assert!(read(".env", cx));
        assert!(read("keys/prod.json", cx));
        assert!(write(path!("/p/.claude/settings.json"), cx));
        assert!(cx.update(|cx| cx.global::<Guard>().worktrees.is_empty()));

        // Audit: logged only.
        set_mode(FyxSecurityMode::Audit, cx);
        cx.run_until_parked();
        assert!(read(".env", cx));
        assert!(write(path!("/p/.claude/settings.json"), cx));

        // Shield: Zed's `private_files` plus the repository's additions.
        set_mode(FyxSecurityMode::Shield, cx);
        cx.run_until_parked();
        assert!(!read(".env", cx));
        assert!(!read("keys/prod.json", cx));
        assert!(read("src/main.rs", cx));
        assert!(!write(path!("/p/.claude/settings.json"), cx));
        assert!(!write(path!("/p/.git/hooks/pre-commit"), cx));
        assert!(!write(path!("/p/Makefile"), cx));
        assert!(write(path!("/p/src/main.rs"), cx));

        // A file that does not parse fails closed for the whole worktree.
        fs.save(
            path!("/p/.fyx/security.toml").as_ref(),
            &"mode = \"off\"".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        assert!(!read("src/main.rs", cx));
        assert!(!write(path!("/p/src/main.rs"), cx));

        // Removing it leaves only the user's protections; `.env` is still private.
        fs.remove_file(path!("/p/.fyx/security.toml").as_ref(), Default::default())
            .await
            .unwrap();
        cx.run_until_parked();
        assert!(read("src/main.rs", cx));
        assert!(read("keys/prod.json", cx));
        assert!(!read(".env", cx));

        // Deleting the whole `.fyx/` directory reloads the policy too.
        fs.save(
            path!("/p/.fyx/security.toml").as_ref(),
            &"not toml".into(),
            Default::default(),
        )
        .await
        .unwrap();
        cx.run_until_parked();
        assert!(!read("src/main.rs", cx));
        fs.remove_dir(
            path!("/p/.fyx").as_ref(),
            fs::RemoveOptions {
                recursive: true,
                ignore_if_not_exists: false,
            },
        )
        .await
        .unwrap();
        cx.run_until_parked();
        assert!(read("src/main.rs", cx));
    }
}
