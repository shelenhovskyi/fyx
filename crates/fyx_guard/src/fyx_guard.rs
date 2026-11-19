//! Optional security modes for agents (design: `docs/fyx/security-modes.md`).
//!
//! Every entry point returns early while `fyx_security.mode` is `off`, so Fyx then behaves exactly
//! like Zed. The mode is read from the global settings value only: project settings have no
//! `fyx_security` key, so a repository can neither raise nor lower it. A repository can only
//! tighten, through `.fyx/security.toml` (see [`RepoPolicy`]). A settings change that lowers
//! protection waits for the user's confirmation, while Fyx runs and across restarts (see
//! [`init`]). The rules themselves are in `fyx_guard_core`, without GPUI; this crate applies
//! them to projects, settings and agents, and re-exports them.

pub mod paths;

use std::{
    ops::Deref,
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

use fyx_guard_core::{AccountDir, CONFIRMED_FILE, enforce, scrub};
pub use fyx_guard_core::{
    RepoPolicy, RepoState, SECRET_ENV_PATTERNS, SECURITY_FILE, SecurityConfig, SecurityMode,
    env_name_matches, is_credential_url, protected_config, protected_config_rel, sanitize_for_log,
    secret_env_names, secret_env_vars,
};

/// `fyx_security` in user settings (schema in `settings_content::FyxSecuritySettingsContent`,
/// default in `assets/settings/fyx.json`).
#[derive(Clone, Debug, PartialEq, RegisterSetting)]
pub struct SecuritySettings(pub SecurityConfig);

impl Deref for SecuritySettings {
    type Target = SecurityConfig;

    fn deref(&self) -> &SecurityConfig {
        &self.0
    }
}

impl Settings for SecuritySettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let security = content.fyx_security.as_ref();
        Self(SecurityConfig {
            mode: mode_from_content(security.and_then(|security| security.mode)),
            allow_env: security
                .and_then(|security| security.allow_env.clone())
                .unwrap_or_default(),
        })
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
    enforce(mode, action, private_denial(path, project, cx)).map_err(|message| {
        acp_v1::Error::new(ErrorCode::INVALID_PARAMS.value(), message.clone()).data(message)
    })
}

/// Hook for the built-in agent's `read_file` tool, beside its own `private_files` checks: the
/// worktree's `.fyx/security.toml` (w9-step1 F7), and every spelling of the path, so a link or a
/// Windows alias does not get around Zed's `private_files` either. The tool reports the error to
/// the model.
pub fn check_agent_read(project: &Project, path: &ProjectPath, cx: &App) -> Result<(), String> {
    let mode = mode(cx);
    if mode == SecurityMode::Off {
        return Ok(());
    }
    let action = || format!("an agent read of {}", path.path.as_unix_str());
    enforce(mode, action, private_denial(path, project, cx))
}

/// For the extension host (`agent:observe`, `workspace:edit` reads): why `path` is private, in
/// every security mode, shield or not (integration-5 S1). Zed's `private_files` (global and
/// worktree) and the worktree's `.fyx/security.toml`, matched against every spelling of the path.
/// An extension never gets the text of such a file.
pub fn extension_read_denial(project: &Project, path: &ProjectPath, cx: &App) -> Option<String> {
    private_denial(path, project, cx)
}

/// For the extension host (`workspace:edit`): why an extension may not write `abs_path`, in
/// every security mode (integration-5 S2). Agent configs at any depth, the worktree's `read_only`
/// patterns, and links out of the project, as [`check_config_write`] checks under shield.
pub fn extension_write_denial(project: &Project, abs_path: &Path, cx: &App) -> Option<String> {
    write_denial(project, abs_path, cx)
}

/// Hook for the built-in agent's `grep` tool: whether to skip a file the shield makes private.
/// Audit mode logs and skips nothing.
pub fn agent_read_blocked(project: &Project, path: &ProjectPath, cx: &App) -> bool {
    check_agent_read(project, path, cx).is_err()
}

/// Every spelling of the file `path` names, as project paths: the typed one, Windows' rewrites
/// of it, the real file behind links, and on case-insensitive file systems the lower-case form
/// (triage-3 top-7 #1, security-2 S1 and T4, bugs-1 #14 and #16). A protection applies when any
/// of them matches.
fn project_path_spellings(project: &Project, path: &ProjectPath, cx: &App) -> Vec<ProjectPath> {
    let mut spellings = vec![path.clone()];
    if let Some(abs_path) = project.absolute_path(path, cx) {
        for candidate in resolve_path(project, &abs_path).candidates() {
            if let Some(found) = project.find_project_path(candidate, cx)
                && !spellings.contains(&found)
            {
                spellings.push(found);
            }
        }
    }
    if cfg!(any(windows, target_os = "macos")) {
        let lower: Vec<ProjectPath> = spellings
            .iter()
            .filter_map(|spelling| {
                let lower = spelling.path.as_unix_str().to_lowercase();
                let lower = RelPath::from_unix_str(&lower).ok()?;
                Some(ProjectPath {
                    worktree_id: spelling.worktree_id,
                    path: lower.into(),
                })
            })
            .collect();
        for lower in lower {
            if !spellings.contains(&lower) {
                spellings.push(lower);
            }
        }
    }
    spellings
}

/// [`paths::resolve_blocking`] on a real file system: hooks are synchronous, and resolving costs
/// one `canonicalize` of the deepest existing ancestor. A fake file system (tests) gets the
/// spellings without the real file.
fn resolve_path(project: &Project, abs_path: &Path) -> paths::ResolvedPath {
    if project.fs().is_fake() {
        paths::unresolved(abs_path)
    } else {
        paths::resolve_blocking(abs_path)
    }
}

fn private_denial(path: &ProjectPath, project: &Project, cx: &App) -> Option<String> {
    project_path_spellings(project, path, cx)
        .iter()
        .find_map(|spelling| {
            private_denial_of(spelling, cx)
                .or_else(|| account_dir(&project.absolute_path(spelling, cx)?, cx))
        })
}

fn private_denial_of(path: &ProjectPath, cx: &App) -> Option<String> {
    if WorktreeSettings::get_global(cx).is_path_private(&path.path) {
        return Some(fyx_guard_core::GLOBAL_PRIVATE_DENIAL.into());
    }
    if WorktreeSettings::get(Some(path.into()), cx).is_path_private(&path.path) {
        return Some(fyx_guard_core::WORKTREE_PRIVATE_DENIAL.into());
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

/// Inside a worktree the protected names are matched against the worktree-relative path only, so
/// a project that itself lives under a `.claude/` or `.codex/` folder (an agent's worktree) is
/// not read-only as a whole (bugs-1 #15). Outside every worktree the absolute path is matched.
fn write_denial(project: &Project, path: &Path, cx: &App) -> Option<String> {
    let resolved = resolve_path(project, path);
    if resolved.has_windows_alias() {
        return Some(fyx_guard_core::WINDOWS_ALIAS_DENIAL.into());
    }
    if let Some(canonical) = &resolved.canonical
        && project.find_project_path(&resolved.lexical, cx).is_some()
        && !inside_worktrees(project, canonical, cx)
    {
        return Some(fyx_guard_core::LINK_OUT_DENIAL.into());
    }
    let mut spellings: Vec<PathBuf> = resolved
        .candidates()
        .into_iter()
        .map(Path::to_path_buf)
        .collect();
    if cfg!(any(windows, target_os = "macos")) {
        let lower: Vec<PathBuf> = spellings
            .iter()
            .filter_map(|path| path.to_str().map(|text| PathBuf::from(text.to_lowercase())))
            .collect();
        spellings.extend(lower);
    }
    spellings
        .iter()
        .find_map(|spelling| write_denial_of(project, spelling, cx))
}

/// Whether `path` (a real, canonical path) is inside one of the project's worktrees, whose roots
/// are resolved too.
fn inside_worktrees(project: &Project, path: &Path, cx: &App) -> bool {
    project.find_project_path(path, cx).is_some()
        || project.worktrees(cx).any(|worktree| {
            let root = worktree.read(cx).abs_path();
            resolve_path(project, &root)
                .canonical
                .is_some_and(|root| path.starts_with(root))
        })
}

fn write_denial_of(project: &Project, path: &Path, cx: &App) -> Option<String> {
    let project_path = project.find_project_path(path, cx);
    let protected = match &project_path {
        Some(project_path) => protected_config_rel(&project_path.path),
        None => protected_config(path),
    };
    protected
        .or_else(|| user_config(path))
        .map(|rule| fyx_guard_core::read_only_config_denial(&rule))
        .or_else(|| account_dir(path, cx))
        .or_else(|| {
            let project_path = project_path.as_ref()?;
            repo_denial(project_path.worktree_id, cx, |state| {
                state.write_denial(&project_path.path)
            })
        })
}

/// Fyx's own config directory and confirmed file ([`fyx_guard_core::user_config_rule`]).
fn user_config(path: &Path) -> Option<String> {
    fyx_guard_core::user_config_rule(path, ::paths::config_dir(), ::paths::data_dir())
}

/// An account dir of `fyx.accounts` ([`fyx_guard_core::account_dir_denial`]).
fn account_dir(path: &Path, cx: &App) -> Option<String> {
    let accounts = cx
        .try_global::<SettingsStore>()?
        .merged_settings()
        .fyx
        .as_ref()?
        .accounts
        .as_ref()?
        .accounts();
    fyx_guard_core::account_dir_denial(
        path,
        accounts.iter().map(|account| AccountDir {
            tool: &account.tool,
            name: &account.name,
            dir: &account.dir,
        }),
    )
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
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok())))
        .collect::<Vec<_>>();
    scrub(env, &inherited, settings.mode, &settings.allow_env);
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
    /// something else or Fyx restarts; it applies only once the user confirms it.
    declined: Option<SecuritySettings>,
    prompt_open: bool,
    /// [`CONFIRMED_FILE`]: written whenever the settings in force change, read at start.
    confirmed_file: Option<PathBuf>,
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
/// settings file: the lower value applies only once the user confirms. The settings in force are
/// kept in [`CONFIRMED_FILE`], so at the next start a lowering the user never confirmed is still
/// held back and asked about again (security-2 T3).
pub fn init(cx: &mut App) {
    init_with_confirmed_file(Some(::paths::data_dir().join(CONFIRMED_FILE)), cx);
}

/// [`init`] without [`CONFIRMED_FILE`], for tests: they must not read or write the user's data
/// folder. A lowering then waits only while the app runs.
#[doc(hidden)]
pub fn init_without_persistence(cx: &mut App) {
    init_with_confirmed_file(None, cx);
}

fn init_with_confirmed_file(confirmed_file: Option<PathBuf>, cx: &mut App) {
    // One small local file, read once before any project exists.
    let confirmed = confirmed_file
        .as_ref()
        .and_then(|path| std::fs::read(path).ok())
        .map(|bytes| SecuritySettings(SecurityConfig::from_confirmed_toml(&bytes)));
    cx.set_global(Guard {
        confirmed_file,
        ..Guard::default()
    });
    if let Some(confirmed) = confirmed {
        let requested = SecuritySettings::get_global(cx).clone();
        if lowers(&confirmed, &requested) {
            log::warn!(
                "fyx_guard: the settings file asks for {} but the user last confirmed {}; keeping \
                 {} until the user confirms",
                describe(&requested),
                describe(&confirmed),
                describe(&confirmed),
            );
            cx.global_mut::<Guard>().effective = Some(confirmed.clone());
            apply(confirmed, cx);
        }
    }
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
    // A lowering held back at start has had no window to ask in until now.
    let ask = guard.pending.is_some() && !guard.prompt_open;
    cx.subscribe(&project, on_project_event).detach();
    load_project_policies(&project, cx);
    if ask {
        cx.defer(ask_to_lower);
    }
}

/// Whether `requested` protects less than `current` ([`SecurityConfig::lowered_by`]).
fn lowers(current: &SecuritySettings, requested: &SecuritySettings) -> bool {
    current.lowered_by(requested)
}

fn describe(settings: &SecuritySettings) -> String {
    settings.describe()
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
    if let Some(path) = guard.confirmed_file.clone()
        && let Some(bytes) = settings.to_confirmed_toml()
    {
        cx.background_executor()
            .spawn(async move {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).ok();
                }
                if let Err(error) = std::fs::write(&path, bytes) {
                    log::error!("fyx_guard: cannot save {}: {error}", path.display());
                }
            })
            .detach();
    }
    let guard = cx.global_mut::<Guard>();
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
            "fyx_guard: no window to confirm lowering security in; keeping {} until one opens",
            describe(&current)
        );
        return;
    };
    let message = format!(
        "Lower Fyx's security mode from {} to {}?",
        describe(&current),
        describe(&requested)
    );
    let detail = "`fyx_security` in your settings file asks for less protection than you last \
                  confirmed. An agent's terminal can write that file, so a change that lowers \
                  protection waits for you. If you keep the current mode, Fyx asks again the next \
                  time it starts.";
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
            "fyx_guard: the user kept the current security mode; {} is asked about again at the \
             next start",
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
/// parsed, the worktree is treated as fully restricted (fail closed). Loaded in every mode: the
/// agent hooks ignore it while the shield is off, but the extension host's checks
/// ([`extension_read_denial`], [`extension_write_denial`]) always apply it.
fn load_policy(project: &Entity<Project>, worktree: &Entity<Worktree>, cx: &mut App) {
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

    /// [`init`] without a confirmed file: tests must not read or write the user's data folder.
    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            init_with_confirmed_file(None, cx);
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

        // Without a confirmed file the next start reads the settings file as it is.
        cx.update(|cx| init_with_confirmed_file(None, cx));
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Off);
    }

    /// security-2 T3: a lowering the user declined, or never saw, is still held back after a
    /// restart, and asked about again once a project window opens.
    #[gpui::test]
    async fn declined_lowering_survives_a_restart(cx: &mut TestAppContext) {
        let directory = std::env::temp_dir().join(format!("fyx-guard-{}", std::process::id()));
        let confirmed = directory.join(CONFIRMED_FILE);
        std::fs::remove_file(&confirmed).ok();
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            init_with_confirmed_file(Some(confirmed.clone()), cx);
        });
        cx.add_window(|_, _| TestView);
        set_mode(FyxSecurityMode::Shield, cx);
        cx.executor().allow_parking();
        cx.run_until_parked();
        set_mode(FyxSecurityMode::Off, cx);
        cx.simulate_prompt_answer("Keep Current Mode");
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        let saved = std::fs::read(&confirmed).expect("saved");
        assert_eq!(
            SecurityConfig::from_confirmed_toml(&saved).mode,
            SecurityMode::Shield
        );

        // Restart: the file still says "off", the confirmed value says "shield".
        cx.update(|cx| init_with_confirmed_file(Some(confirmed.clone()), cx));
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Shield);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(path!("/p"), json!({ "a.rs": "" })).await;
        let _project = Project::test(fs, [path!("/p").as_ref()], cx).await;
        cx.run_until_parked();
        let (message, _) = cx.pending_prompt().expect("asked again");
        assert!(message.contains("\"shield\" to \"off\""), "{message}");
        cx.simulate_prompt_answer("Lower Protection");
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| mode(cx)), SecurityMode::Off);

        // A damaged file counts as shield.
        assert_eq!(
            SecurityConfig::from_confirmed_toml(b"{nope").mode,
            SecurityMode::Shield
        );
        std::fs::remove_dir_all(&directory).ok();
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

    /// Design wit-0.4 §C.6 rule 5: account dirs are off limits like `.claude/`, for agents under
    /// the shield and for extensions in every mode, and the reason names the account, not the dir.
    #[gpui::test]
    async fn account_dirs_are_protected(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            cx.update_global::<SettingsStore, _>(|store, cx| {
                store.update_user_settings(cx, |settings| {
                    let accounts = settings
                        .fyx
                        .get_or_insert_default()
                        .accounts
                        .get_or_insert_default();
                    accounts.tools = Some(
                        [(
                            "claude".to_string(),
                            settings::FyxAccountToolContent {
                                env: Some("CLAUDE_CONFIG_DIR".into()),
                                ..Default::default()
                            },
                        )]
                        .into_iter()
                        .collect(),
                    );
                    accounts.profiles = Some(vec![
                        settings::FyxAccountProfileContent {
                            name: Some("Work".into()),
                            tool: Some("claude".into()),
                            dir: Some(path!("/p/accounts/work").into()),
                            color: None,
                        },
                        settings::FyxAccountProfileContent {
                            name: Some("Home".into()),
                            tool: Some("claude".into()),
                            dir: Some("~/.claude-fyx-guard-test".into()),
                            color: None,
                        },
                    ]);
                });
            });
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/p"),
            json!({
                "accounts": { "work": { ".credentials.json": "{}" }, "notes.md": "" },
                "src": { "main.rs": "" },
            }),
        )
        .await;
        let project = Project::test(fs, [path!("/p").as_ref()], cx).await;
        cx.run_until_parked();
        let worktree_id = project.read_with(cx, |project, cx| {
            project.worktrees(cx).next().unwrap().read(cx).id()
        });
        let project_path = |path: &str| ProjectPath {
            worktree_id,
            path: rel_path(path).into(),
        };
        let extension_read = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                extension_read_denial(project, &project_path(path), cx)
            })
        };
        let extension_write = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                extension_write_denial(project, Path::new(path), cx)
            })
        };

        // Extensions: in every mode.
        let reason = extension_read("accounts/work/.credentials.json", cx).expect("read refused");
        assert!(reason.contains("\"Work\""), "{reason}");
        assert!(
            !reason.contains("accounts"),
            "the dir stays hidden: {reason}"
        );
        assert!(extension_read("accounts/notes.md", cx).is_none());
        assert!(extension_read("src/main.rs", cx).is_none());
        assert!(extension_write(path!("/p/accounts/work/settings.json"), cx).is_some());
        assert!(extension_write(path!("/p/Accounts/Work/settings.json"), cx).is_some());
        assert!(extension_write(path!("/p/accounts/notes.md"), cx).is_none());
        let home = util::paths::home_dir().join(".claude-fyx-guard-test");
        assert!(extension_write(&home.join("x.json").to_string_lossy(), cx).is_some());
        assert!(extension_write("~/.claude-fyx-guard-test/x.json", cx).is_some());

        // Agents: only under the shield.
        let agent_read = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_private_path(project, &project_path(path), cx).is_ok()
            })
        };
        let agent_write = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_config_write(project, Path::new(path), cx).is_ok()
            })
        };
        assert!(agent_read("accounts/work/.credentials.json", cx));
        set_mode(FyxSecurityMode::Shield, cx);
        assert!(!agent_read("accounts/work/.credentials.json", cx));
        assert!(agent_read("accounts/notes.md", cx));
        assert!(!agent_write(path!("/p/accounts/work/settings.json"), cx));
        assert!(!agent_write("~/.claude-fyx-guard-test/x.json", cx));
        assert!(agent_write(path!("/p/src/main.rs"), cx));
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

        // Off: the agent hooks check nothing. The repository file is still read, because the
        // extension host applies it in every mode (integration-5 S1).
        cx.run_until_parked();
        assert!(read(".env", cx));
        assert!(read("keys/prod.json", cx));
        assert!(write(path!("/p/.claude/settings.json"), cx));
        assert!(write(&::paths::settings_file().to_string_lossy(), cx));
        assert!(cx.update(|cx| !cx.global::<Guard>().worktrees.is_empty()));

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
        assert!(!write(&::paths::settings_file().to_string_lossy(), cx));
        assert!(!write(
            &::paths::config_dir().join("tasks.json").to_string_lossy(),
            cx
        ));
        assert!(!write("~/.claude/settings.json", cx));
        assert!(!write(path!("/p/Makefile"), cx));
        assert!(write(path!("/p/src/main.rs"), cx));
        assert!(!write(path!("/p/AGENTS.md"), cx));
        assert!(!write(path!("/p/.vscode/tasks.json"), cx));
        assert!(!write(
            &::paths::data_dir().join(CONFIRMED_FILE).to_string_lossy(),
            cx
        ));

        // The built-in agent's read tool sees the repository's private files too (F7).
        let agent_read = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_agent_read(project, &project_path(path), cx).is_ok()
            })
        };
        assert!(!agent_read("keys/prod.json", cx));
        assert!(agent_read("src/main.rs", cx));

        // Windows aliases (triage-3 top-7 #1): `.env.` and `.env::$DATA` open `.env`; a trailing
        // dot or a stream on a protected name cannot be written either.
        assert!(!read(".env.", cx));
        assert!(!read(".env::$DATA", cx));
        assert!(!agent_read("keys./prod.json", cx));
        assert!(!write(path!("/p/.claude./settings.json"), cx));
        assert!(!write(path!("/p/src/main.rs "), cx));
        assert!(!write(path!("/p/AGENTS.md::$DATA"), cx));
        assert!(write(path!("/p/src/main.rs"), cx));
        // Case: on Windows and macOS `.ENV` is `.env`.
        if cfg!(any(windows, target_os = "macos")) {
            assert!(!read(".ENV", cx));
            assert!(!read("Keys/Prod.json", cx));
            assert!(!write(path!("/p/MAKEFILE"), cx));
        }

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

    /// bugs-1 #15: a project inside an agent's worktree folder is not read-only as a whole.
    #[gpui::test]
    async fn projects_under_agent_folders_are_writable(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/repo/.claude/worktrees/feature"),
            json!({ "src": { "main.rs": "" }, ".claude": { "settings.json": "{}" } }),
        )
        .await;
        let project =
            Project::test(fs, [path!("/repo/.claude/worktrees/feature").as_ref()], cx).await;
        set_mode(FyxSecurityMode::Shield, cx);
        cx.run_until_parked();
        let write = |path: &str, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_config_write(project, Path::new(path), cx).is_ok()
            })
        };
        assert!(write(
            path!("/repo/.claude/worktrees/feature/src/main.rs"),
            cx
        ));
        assert!(!write(
            path!("/repo/.claude/worktrees/feature/.claude/settings.json"),
            cx
        ));
        // Outside the project, the absolute path still counts.
        assert!(!write(path!("/repo/.claude/settings.json"), cx));
    }

    /// security-2 S1: a committed link to a protected or outside file does not get around the
    /// shield. Real files: links need the real file system.
    #[cfg(unix)]
    #[gpui::test]
    async fn links_are_followed(cx: &mut TestAppContext) {
        init_test(cx);
        let temp_dir = std::env::temp_dir().join(format!("fyx-guard-links-{}", std::process::id()));
        std::fs::remove_dir_all(&temp_dir).ok();
        let temp = temp_dir.as_path();
        let root = temp.join("p");
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        std::fs::write(root.join(".claude/settings.json"), "{}").unwrap();
        std::fs::write(temp.join("outside.txt"), "").unwrap();
        std::os::unix::fs::symlink(root.join(".env"), root.join("notes.txt")).unwrap();
        std::os::unix::fs::symlink(root.join(".claude"), root.join("config")).unwrap();
        std::os::unix::fs::symlink(temp.join("outside.txt"), root.join("out.txt")).unwrap();
        // The real file system's watcher runs on its own thread.
        cx.executor().allow_parking();
        let fs = fs::RealFs::new(None, cx.executor());
        let project = Project::test(fs, [root.as_path()], cx).await;
        set_mode(FyxSecurityMode::Shield, cx);
        cx.run_until_parked();
        let worktree_id = project.read_with(cx, |project, cx| {
            project.worktrees(cx).next().unwrap().read(cx).id()
        });
        let read = |path: &str, cx: &mut TestAppContext| {
            let path = ProjectPath {
                worktree_id,
                path: rel_path(path).into(),
            };
            project.read_with(cx, |project, cx| {
                check_private_path(project, &path, cx).is_ok()
            })
        };
        let write = |path: &Path, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_config_write(project, path, cx).is_ok()
            })
        };
        assert!(!read("notes.txt", cx), "the link opens .env");
        assert!(
            !write(&root.join("config/settings.json"), cx),
            "through a link into .claude/"
        );
        assert!(!write(&root.join("out.txt"), cx), "out of the project");
        assert!(write(&root.join("new.txt"), cx));
        drop(project);
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    /// The directory cases of [`links_are_followed`] on Windows, with junctions, which need no
    /// privilege (file symlinks do, so those stay Unix-only).
    #[cfg(windows)]
    #[gpui::test]
    async fn junctions_are_followed(cx: &mut TestAppContext) {
        // Test setup, before any executor work: blocking on `mklink` here is fine.
        #[allow(clippy::disallowed_methods)]
        fn junction(link: &Path, target: &Path) {
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(target)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "mklink /J {}", link.display());
        }

        init_test(cx);
        let temp_dir =
            std::env::temp_dir().join(format!("fyx-guard-junctions-{}", std::process::id()));
        std::fs::remove_dir_all(&temp_dir).ok();
        let temp = temp_dir.as_path();
        let root = temp.join("p");
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::create_dir_all(temp.join("outside")).unwrap();
        std::fs::write(root.join(".claude/settings.json"), "{}").unwrap();
        junction(&root.join("config"), &root.join(".claude"));
        junction(&root.join("shared"), &temp.join("outside"));
        // The real file system's watcher runs on its own thread.
        cx.executor().allow_parking();
        let fs = fs::RealFs::new(None, cx.executor());
        let project = Project::test(fs, [root.as_path()], cx).await;
        set_mode(FyxSecurityMode::Shield, cx);
        cx.run_until_parked();
        let write = |path: &Path, cx: &mut TestAppContext| {
            project.read_with(cx, |project, cx| {
                check_config_write(project, path, cx).is_ok()
            })
        };
        assert!(
            !write(&root.join("config/settings.json"), cx),
            "through a junction into .claude/"
        );
        assert!(
            !write(&root.join("config/new.json"), cx),
            "a new file through a junction into .claude/"
        );
        assert!(
            !write(&root.join("shared/notes.txt"), cx),
            "through a junction out of the project"
        );
        assert!(write(&root.join("new.txt"), cx));
        drop(project);
        // `remove_dir_all` removes a junction without following it.
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
