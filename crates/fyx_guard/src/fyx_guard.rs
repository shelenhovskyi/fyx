//! Optional security modes for agents (design: `docs/fyx/security-modes.md`).
//!
//! Every entry point returns early while `fyx_security.mode` is `off`, so Fyx then behaves exactly
//! like Zed. The mode is read from the global settings value only: project settings have no
//! `fyx_security` key, so a repository can neither raise nor lower it. A repository can only
//! tighten, through `.fyx/security.toml` (see [`RepoPolicy`]). While Fyx runs, a settings change
//! that lowers protection waits for the user's confirmation (see [`init`]).

pub mod paths;
mod policy;

use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
};

use agent_client_protocol::{ErrorCode, v1::schema as acp_v1};
use collections::HashMap;
use fs::Fs;
use gpui::{App, AsyncApp, Entity, Global, PromptButton, PromptLevel, WeakEntity};
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

/// Ordered by strength: `Off < Audit < Shield`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SecurityMode {
    Off,
    /// Shield as a dry run: log what would be blocked, block nothing.
    Audit,
    Shield,
}

/// `fyx_security` in user settings (schema in `settings_content::FyxSecuritySettingsContent`,
/// default in `assets/settings/fyx.json`).
#[derive(Clone, Debug, PartialEq, RegisterSetting)]
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

/// The settings in force. After [`init`] they are the guard's, which hold back a lowering until the
/// user confirms it. Without `init` (tests, hosts other than Fyx's main) nothing lowers a mode at run
/// time, so the setting applies directly; without a settings store nothing can have turned one on.
fn active(cx: &App) -> Option<&SecuritySettings> {
    if let Some(effective) = cx
        .try_global::<Guard>()
        .and_then(|guard| guard.effective.as_ref())
    {
        return Some(effective);
    }
    cx.has_global::<SettingsStore>()
        .then(|| SecuritySettings::get_global(cx))
}

/// The active mode.
pub fn mode(cx: &App) -> SecurityMode {
    active(cx).map_or(SecurityMode::Off, |settings| settings.mode)
}

/// Applies `mode` to a denial: shield turns it into the message for the agent, audit only logs it.
fn enforce(
    mode: SecurityMode,
    action: impl FnOnce() -> String,
    denial: Option<String>,
) -> Result<(), String> {
    let Some(reason) = denial else {
        return Ok(());
    };
    match mode {
        SecurityMode::Off => Ok(()),
        SecurityMode::Audit => {
            log::warn!("fyx_guard audit: would block {}: {reason}", action());
            Ok(())
        }
        SecurityMode::Shield => {
            let action = action();
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
    let action = || {
        format!(
            "an agent read of {}",
            display_project_path(project, path, cx)
        )
    };
    enforce(mode, action, private_denial(path, cx)).map_err(|message| {
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

/// Hook for the built-in agent's `create_directory` tool. Paths are the tool's input: absolute, or
/// starting with a worktree's root name.
pub fn check_agent_create(
    project: &Entity<Project>,
    path: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    check_path_tool(project, "create of", path, None, cx)
}

/// Hook for the built-in agent's `delete_path` tool: the path and everything under it.
pub fn check_agent_delete(
    project: &Entity<Project>,
    path: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    check_path_tool(project, "delete of", path, Some(path), cx)
}

/// Hook for the built-in agent's `move_path` tool: the tree it removes at the source and the tree it
/// writes at the destination.
pub fn check_agent_move(
    project: &Entity<Project>,
    source: &str,
    destination: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    check_path_tool(project, "move of", source, Some(source), cx)?;
    check_path_tool(project, "move to", destination, Some(source), cx)
}

/// Hook for the built-in agent's `copy_path` tool: the tree it writes at the destination. Reading the
/// source is not a write, so a copy out of an agent config is allowed.
pub fn check_agent_copy(
    project: &Entity<Project>,
    source: &str,
    destination: &str,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    check_path_tool(project, "copy to", destination, Some(source), cx)
}

/// Checks `target` and, when the tool carries the tree at `tree_from` there, every path of that tree
/// as it lands under `target`: moving `pkg/` with a `pkg/.claude/` inside writes a `.claude/`.
/// The tree is the one the worktree has scanned; excluded directories such as `.git/` are not in it.
fn check_path_tool(
    project: &Entity<Project>,
    verb: &str,
    target: &str,
    tree_from: Option<&str>,
    cx: &mut AsyncApp,
) -> Result<(), String> {
    cx.update(|cx| {
        let mode = mode(cx);
        if mode == SecurityMode::Off {
            return Ok(());
        }
        let project = project.read(cx);
        let target = Path::new(target);
        let mut paths = vec![target.to_path_buf()];
        if let Some(tree_from) = tree_from {
            paths.extend(
                tree_suffixes(project, tree_from, cx)
                    .into_iter()
                    .map(|suffix| target.join(suffix)),
            );
        }
        let Some((path, reason)) = paths
            .iter()
            .find_map(|path| Some((path, write_denial(project, path, cx)?)))
        else {
            return Ok(());
        };
        enforce(
            mode,
            || format!("an agent {verb} {}", path.display()),
            Some(reason),
        )
    })
}

/// The paths below `path` in its worktree, relative to `path`.
fn tree_suffixes(project: &Project, path: &str, cx: &App) -> Vec<PathBuf> {
    let Some(project_path) = project.find_project_path(path, cx) else {
        return Vec::new();
    };
    let Some(worktree) = project.worktree_for_id(project_path.worktree_id, cx) else {
        return Vec::new();
    };
    worktree
        .read(cx)
        .traverse_from_path(true, true, true, &project_path.path)
        .take_while(|entry| entry.path.starts_with(&project_path.path))
        .filter_map(|entry| {
            let suffix = entry.path.strip_prefix(&project_path.path).ok()?;
            (!suffix.is_empty()).then(|| suffix.as_std_path().to_path_buf())
        })
        .collect()
}

fn write_check(project: &Project, path: &Path, cx: &App) -> Result<(), String> {
    let mode = mode(cx);
    if mode == SecurityMode::Off {
        return Ok(());
    }
    let action = || format!("an agent write to {}", path.display());
    enforce(mode, action, write_denial(project, path, cx))
}

fn write_denial(project: &Project, path: &Path, cx: &App) -> Option<String> {
    let project_path = project.find_project_path(path, cx);
    let read_only_config =
        |rule: String| format!("agent configs ({rule}) are read-only for agents");
    protected_config(path)
        .or_else(|| user_config(path))
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
        })
}

/// Fyx's own config directory: the user settings file sets the mode, and `tasks.json` and
/// `keymap.json` there run commands, like `.zed/` does for a project.
fn user_config(path: &Path) -> Option<String> {
    let path = match path.strip_prefix("~") {
        Ok(rest) => util::paths::home_dir().join(rest),
        Err(_) => path.to_path_buf(),
    };
    let config_dir = paths::config_dir();
    policy::is_within(&path, config_dir).then(|| format!("{}/", config_dir.display()))
}

/// Hook for the environment an agent terminal inherits, applied before the variables the agent
/// sets for its command. Secret-looking variables not in `allow_env` are set to an empty value
/// rather than removed: the terminal re-resolves the directory environment and inherits Fyx's own
/// process environment, and an explicit empty value is what overrides both.
pub fn scrub_env(env: &mut HashMap<String, String>, cx: &App) {
    let Some(settings) = active(cx) else {
        return;
    };
    if settings.mode == SecurityMode::Off {
        return;
    }
    let inherited = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .collect::<Vec<_>>();
    scrub(env, &inherited, settings.mode, &settings.allow_env);
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

/// Step 2 (`tasks/W9-security.md`), not wired and not isolation yet: returns the command
/// unchanged. It will run an external ACP agent's process tree in an OS sandbox that denies reads
/// outside the project (Seatbelt, bubblewrap, MXC on Windows), and fail closed if the sandbox
/// cannot start.
#[doc(hidden)]
pub fn wrap_agent(command: AgentServerCommand, _cx: &App) -> anyhow::Result<AgentServerCommand> {
    Ok(command)
}

/// Step 2, not wired and not isolation yet: the same for a stdio MCP server. Its hook in
/// `context_server` cannot call this crate directly (dependency cycle through `project`); see the
/// design doc, hook H9.
#[doc(hidden)]
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

/// Repository policies by worktree, kept current while a mode is on, and the settings in force.
#[derive(Default)]
struct Guard {
    projects: Vec<WeakEntity<Project>>,
    worktrees: HashMap<WorktreeId, (u64, RepoState)>,
    next_generation: u64,
    /// `None` until the settings are first read at [`init`].
    effective: Option<SecuritySettings>,
    /// A lowering from the settings file that the user has not answered yet.
    pending: Option<SecuritySettings>,
    /// A lowering the user declined. It is not asked about again until the file asks for
    /// something else; it applies at the next start.
    declined: Option<SecuritySettings>,
    prompt_open: bool,
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

/// Reads the settings once and then follows them. Raising protection applies at once. Lowering it
/// (a lower mode, or `allow_env` entries the current mode does not have) keeps the stricter
/// settings and asks the user in a window prompt, because an agent's terminal can write the user
/// settings file: the lower value applies only once the user confirms, or at the next start.
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

/// Whether `requested` protects less than `current`. A higher mode protects more whatever its
/// `allow_env`, since a lower mode blanks nothing.
fn lowers(current: &SecuritySettings, requested: &SecuritySettings) -> bool {
    match requested.mode.cmp(&current.mode) {
        Ordering::Less => true,
        Ordering::Greater => false,
        Ordering::Equal => {
            current.mode != SecurityMode::Off
                && requested
                    .allow_env
                    .iter()
                    .any(|entry| !current.allow_env.contains(entry))
        }
    }
}

fn describe(settings: &SecuritySettings) -> String {
    let mode = match settings.mode {
        SecurityMode::Off => "off",
        SecurityMode::Audit => "audit",
        SecurityMode::Shield => "shield",
    };
    if settings.allow_env.is_empty() || settings.mode == SecurityMode::Off {
        format!("\"{mode}\"")
    } else {
        format!("\"{mode}\" with allow_env {:?}", settings.allow_env)
    }
}

fn on_settings_changed(cx: &mut App) {
    let requested = SecuritySettings::get_global(cx).clone();
    let guard = cx.global_mut::<Guard>();
    let Some(current) = guard.effective.clone() else {
        log::info!("fyx_guard: security mode {}", describe(&requested));
        apply(requested, cx);
        return;
    };
    if current == requested {
        guard.pending = None;
        guard.declined = None;
        return;
    }
    if lowers(&current, &requested) {
        if guard.pending.as_ref() == Some(&requested) || guard.declined.as_ref() == Some(&requested)
        {
            return;
        }
        log::warn!(
            "fyx_guard: the settings file asks to lower security from {} to {}; keeping {} until \
             the user confirms",
            describe(&current),
            describe(&requested),
            describe(&current),
        );
        guard.pending = Some(requested);
        guard.declined = None;
        cx.defer(ask_to_lower);
        return;
    }
    if current.mode != SecurityMode::Off || requested.mode != SecurityMode::Off {
        log::warn!(
            "fyx_guard: security changed from {} to {}",
            describe(&current),
            describe(&requested)
        );
    }
    guard.pending = None;
    guard.declined = None;
    apply(requested, cx);
}

fn apply(settings: SecuritySettings, cx: &mut App) {
    let guard = cx.global_mut::<Guard>();
    let previous = guard.effective.as_ref().map(|settings| settings.mode);
    let mode = settings.mode;
    guard.effective = Some(settings);
    if mode == SecurityMode::Off {
        guard.worktrees.clear();
    } else if previous.is_none_or(|previous| previous == SecurityMode::Off) {
        let projects = guard.projects.clone();
        for project in projects.iter().filter_map(WeakEntity::upgrade) {
            load_project_policies(&project, cx);
        }
    }
}

fn ask_to_lower(cx: &mut App) {
    let guard = cx.global::<Guard>();
    if guard.prompt_open {
        // The open prompt asks again when it is answered.
        return;
    }
    let (Some(current), Some(requested)) = (guard.effective.clone(), guard.pending.clone()) else {
        return;
    };
    let Some(window) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
    else {
        log::warn!(
            "fyx_guard: no window to confirm lowering security in; keeping {} until the next start",
            describe(&current)
        );
        return;
    };
    let message = format!(
        "Lower Fyx's security mode from {} to {}?",
        describe(&current),
        describe(&requested)
    );
    let detail = "`fyx_security` changed in your settings file. An agent's terminal can write that \
                  file, so a change that lowers protection waits for you. If you keep the current \
                  mode, the file's value applies the next time Fyx starts.";
    let answer = window.update(cx, |_, window, cx| {
        window.prompt(
            PromptLevel::Warning,
            &message,
            Some(detail),
            &[
                PromptButton::ok("Lower Protection"),
                PromptButton::cancel("Keep Current Mode"),
            ],
            cx,
        )
    });
    let answer = match answer {
        Ok(answer) => answer,
        Err(error) => {
            log::error!(
                "fyx_guard: cannot ask to lower security ({error:#}); keeping {} until the next \
                 start",
                describe(&current)
            );
            return;
        }
    };
    cx.global_mut::<Guard>().prompt_open = true;
    cx.spawn(async move |cx| {
        let lower = answer.await.ok() == Some(0);
        cx.update(|cx| on_lowering_answered(requested, lower, cx));
    })
    .detach();
}

fn on_lowering_answered(asked: SecuritySettings, lower: bool, cx: &mut App) {
    let guard = cx.global_mut::<Guard>();
    guard.prompt_open = false;
    if guard.pending.as_ref() != Some(&asked) {
        // The file changed while the prompt was open; ask about what it asks for now.
        if guard.pending.is_some() {
            ask_to_lower(cx);
        }
        return;
    }
    guard.pending = None;
    if lower {
        log::warn!(
            "fyx_guard: the user confirmed lowering security to {}",
            describe(&asked)
        );
        apply(asked, cx);
    } else {
        log::warn!(
            "fyx_guard: the user kept the current security mode; {} applies at the next start",
            describe(&asked)
        );
        guard.declined = Some(asked);
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
        let action = || "a read".to_string();
        assert!(enforce(SecurityMode::Off, action, denial()).is_ok());
        assert!(enforce(SecurityMode::Audit, action, denial()).is_ok());
        let error = enforce(SecurityMode::Shield, action, denial()).unwrap_err();
        assert!(error.contains("private_files"), "{error}");
        assert!(enforce(SecurityMode::Shield, action, None).is_ok());
        assert!(
            enforce(
                SecurityMode::Shield,
                || panic!("the action is described only for a denial"),
                None
            )
            .is_ok()
        );
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

    fn update_security(
        cx: &mut TestAppContext,
        update: impl FnOnce(&mut settings::FyxSecuritySettingsContent),
    ) {
        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    update(settings.fyx_security.get_or_insert_default());
                });
            });
        });
        cx.run_until_parked();
    }

    fn set_mode(mode: FyxSecurityMode, cx: &mut TestAppContext) {
        update_security(cx, |security| security.mode = Some(mode));
    }

    fn set_allow_env(allow_env: &[&str], cx: &mut TestAppContext) {
        let allow_env = allow_env.iter().map(|name| name.to_string()).collect();
        update_security(cx, |security| security.allow_env = Some(allow_env));
    }

    fn active_settings(cx: &mut TestAppContext) -> SecuritySettings {
        cx.update(|cx| active(cx).cloned().unwrap())
    }

    struct TestView;

    impl gpui::Render for TestView {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::Empty
        }
    }

    #[gpui::test]
    async fn lowering_waits_for_the_users_confirmation(cx: &mut TestAppContext) {
        init_test(cx);
        cx.add_window(|_, _| TestView);
        set_mode(FyxSecurityMode::Shield, cx);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        assert!(!cx.has_pending_prompt(), "raising is immediate");

        // An agent's terminal rewrites the settings file: the shield stays until the user answers.
        set_mode(FyxSecurityMode::Off, cx);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        let (message, _) = cx.pending_prompt().unwrap();
        assert!(message.contains("\"shield\" to \"off\""), "{message}");
        cx.simulate_prompt_answer("Keep Current Mode");
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);

        // A declined value is not asked about again on unrelated settings changes.
        update_security(cx, |_| {});
        assert!(!cx.has_pending_prompt());

        // Another lower value asks again; confirming applies it.
        set_mode(FyxSecurityMode::Audit, cx);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        cx.simulate_prompt_answer("Lower Protection");
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Audit);

        // Growing `allow_env` under the same mode lowers protection too; shrinking it does not.
        set_mode(FyxSecurityMode::Shield, cx);
        assert!(!cx.has_pending_prompt());
        set_allow_env(&["GITHUB_TOKEN"], cx);
        assert!(active_settings(cx).allow_env.is_empty());
        cx.simulate_prompt_answer("Lower Protection");
        cx.run_until_parked();
        assert_eq!(active_settings(cx).allow_env, ["GITHUB_TOKEN"]);
        set_allow_env(&[], cx);
        assert!(!cx.has_pending_prompt());
        assert!(active_settings(cx).allow_env.is_empty());

        // Reverting the file while the prompt is open withdraws the question.
        set_mode(FyxSecurityMode::Off, cx);
        set_mode(FyxSecurityMode::Shield, cx);
        cx.simulate_prompt_answer("Lower Protection");
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        assert!(!cx.has_pending_prompt());
    }

    #[gpui::test]
    async fn lowering_without_a_window_waits_for_the_next_start(cx: &mut TestAppContext) {
        init_test(cx);
        set_mode(FyxSecurityMode::Shield, cx);
        set_mode(FyxSecurityMode::Off, cx);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);

        // The next start reads the file as it is.
        cx.update(init);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Off);
    }

    #[gpui::test]
    async fn off_never_asks(cx: &mut TestAppContext) {
        init_test(cx);
        cx.add_window(|_, _| TestView);
        set_allow_env(&["*"], cx);
        set_mode(FyxSecurityMode::Off, cx);
        assert!(!cx.has_pending_prompt());
        assert_eq!(active_settings(cx).allow_env, ["*"]);
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Off);
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
        assert!(write(&paths::settings_file().to_string_lossy(), cx));
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
        assert!(!write(path!("/p/.git/config"), cx));
        assert!(!write(&paths::settings_file().to_string_lossy(), cx));
        assert!(!write(
            &paths::config_dir().join("tasks.json").to_string_lossy(),
            cx
        ));
        assert!(!write("~/.claude/settings.json", cx));
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
