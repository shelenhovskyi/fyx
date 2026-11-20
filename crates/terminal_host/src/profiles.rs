//! Extensions' terminal profiles in the terminal panel's "+" menu (`terminal:profiles`, design
//! §B.1) and their `terminal:env` (§B.4).
//!
//! The runtime hands the host each loaded extension's checked profiles and granted env
//! ([`TerminalIntegration`], through `TerminalHost::set_integration`). A profile terminal belongs to
//! the user: the user starts it from the menu, it has no extension origin and no tap. Its program
//! is found on `PATH` outside the worktrees, or at its declared `path` in a trusted worktree; no
//! shell is started unless the profile names one.
//!
//! Env values are resolved here, never by the extension: `${package}` to the extension's package
//! dir, `${account.<tool>}` to the terminal's account dir (§C.6) and `${listen.<name>.port|token}`
//! to the extension's open listener (plan 5.11, kept by the discovery writer). A variable whose
//! listener is not open is left out. `${relay}` is the `fyx-relay` binary next to Fyx (§B.9);
//! without one, a variable using it is left out and a profile whose arguments use it does not
//! start. Names that select an
//! account's dir (the presets' and the user's `fyx.accounts.tools`) are dropped whatever the
//! manifest check let through.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, Global, SharedString, Window};
use settings::Settings as _;
use settings::settings_content::{FyxAccount, FyxAccountsSettingsContent};
use terminal::terminal_settings::TerminalSettings;
use terminal::{TerminalBuilder, TerminalMode};
use terminal_ext::{ACCOUNT_CHOICE_ENV, MenuEntry};
use ui_ext::terminal::{
    ProfileCwd, Template, TemplateContext, TemplatePart, TerminalEnv, TerminalProfile,
};
use util::paths::PathStyle;
use workspace::Workspace;

use crate::accounts;

/// What an extension declared and the user granted for terminals.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalIntegration {
    /// The extension's display name, for menu entries.
    pub extension_name: String,
    /// Checked `[[fyx_ui.terminal_profiles]]` (`terminal:profiles`).
    pub profiles: Vec<TerminalProfile>,
    /// Checked `[fyx_ui.terminal_env]`, when `terminal:env` is granted.
    pub env: Option<TerminalEnv>,
    /// The second install checkbox: the env also goes into every new terminal.
    pub env_all_new: bool,
    /// The installed package's dir, for `${package}`.
    pub package_dir: Option<PathBuf>,
}

impl TerminalIntegration {
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.env.is_none()
    }
}

#[derive(Default)]
struct Integrations(BTreeMap<String, Arc<TerminalIntegration>>);

impl Global for Integrations {}

/// Main thread: replaces `ext`'s integration; an empty one removes it.
pub(crate) fn set(ext: String, integration: TerminalIntegration, cx: &mut App) {
    let integrations = cx.default_global::<Integrations>();
    if integration.is_empty() {
        integrations.0.remove(&ext);
    } else {
        integrations.0.insert(ext, Arc::new(integration));
    }
}

fn integrations(cx: &App) -> Vec<(String, Arc<TerminalIntegration>)> {
    cx.try_global::<Integrations>()
        .map(|integrations| {
            integrations
                .0
                .iter()
                .map(|(ext, integration)| (ext.clone(), integration.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// An extension's open listener by name: its port and host token.
type Listeners<'a> = &'a dyn Fn(&str) -> Option<(u16, String)>;

struct Values<'a> {
    package: Option<&'a Path>,
    accounts: &'a [FyxAccount],
    listeners: Listeners<'a>,
    relay: Option<&'a Path>,
}

impl TemplateContext for Values<'_> {
    fn package(&self) -> Option<String> {
        self.package
            .map(|package| package.to_string_lossy().into_owned())
    }

    fn listen_port(&self, listener: &str) -> Option<u16> {
        (self.listeners)(listener).map(|(port, _)| port)
    }

    /// The manifest check let `${listen.<name>.token}` through only for a listener with
    /// `token_in_env`.
    fn listen_token(&self, listener: &str) -> Option<String> {
        (self.listeners)(listener).map(|(_, token)| token)
    }

    fn relay(&self) -> Option<String> {
        self.relay.map(|relay| relay.to_string_lossy().into_owned())
    }

    fn account_dir(&self, tool: &str) -> Option<String> {
        self.accounts
            .iter()
            .find(|account| account.tool == tool)
            .map(|account| account.dir.to_string_lossy().into_owned())
    }
}

/// The variable names that select an account's dir in the user's `fyx.accounts.tools` (the
/// manifest check's own list is in [`ui_ext::terminal::is_forbidden_terminal_env`]).
fn is_user_tool_env(name: &str, settings: &FyxAccountsSettingsContent) -> bool {
    settings
        .tools
        .iter()
        .flatten()
        .filter_map(|(_, tool)| tool.env.as_deref())
        .any(|env| env.trim().eq_ignore_ascii_case(name))
}

/// `integration`'s env for a terminal with `accounts`, resolved; a variable without a value or
/// with a refused name is left out.
fn resolve_env(
    integration: &TerminalIntegration,
    accounts: &[FyxAccount],
    listeners: Listeners,
    relay: Option<&Path>,
    settings: &FyxAccountsSettingsContent,
) -> Vec<(String, String)> {
    let values = Values {
        package: integration.package_dir.as_deref(),
        accounts,
        listeners,
        relay,
    };
    resolve_with(integration, &values, settings)
}

/// `integration`'s env with `values`; a variable without a value or with a refused name is left
/// out.
fn resolve_with(
    integration: &TerminalIntegration,
    values: &dyn TemplateContext,
    settings: &FyxAccountsSettingsContent,
) -> Vec<(String, String)> {
    let Some(env) = &integration.env else {
        return Vec::new();
    };
    env.vars
        .iter()
        .filter(|(name, template)| {
            let refused = is_user_tool_env(name, settings)
                || (ui_ext::terminal::is_forbidden_terminal_env(name)
                    && !is_host_token(name, template));
            if refused {
                log::warn!("terminal:env: dropped {name}");
            }
            !refused
        })
        .filter_map(|(name, template)| Some((name.clone(), template.resolve(values)?)))
        .collect()
}

/// What a remote shell's command line may hold (plan 5.17): ports the host's SSH tunnel forwards,
/// nothing else. No token, package path, account dir, relay or local pid.
struct RemoteValues<'a> {
    forwarded: &'a dyn Fn(&str) -> Option<u16>,
}

impl TemplateContext for RemoteValues<'_> {
    fn package(&self) -> Option<String> {
        None
    }

    fn listen_port(&self, listener: &str) -> Option<u16> {
        (self.forwarded)(listener)
    }

    fn listen_token(&self, _: &str) -> Option<String> {
        None
    }

    fn relay(&self) -> Option<String> {
        None
    }

    fn account_dir(&self, _: &str) -> Option<String> {
        None
    }

    fn pid(&self) -> Option<u32> {
        None
    }
}

/// For a shell on an SSH host (`remote`, the `RemoteClient` entity): the env of every extension
/// allowed into every new terminal, with only what [`RemoteValues`] gives a value.
pub(crate) fn remote_all_new_env(
    remote: gpui::EntityId,
    settings: &FyxAccountsSettingsContent,
    cx: &App,
) -> Vec<(String, String)> {
    integrations(cx)
        .iter()
        .filter(|(_, integration)| integration.env_all_new)
        .flat_map(|(ext, integration)| {
            let forwarded =
                |listener: &str| crate::discovery::forwarded_port(remote, ext, listener, cx);
            resolve_with(
                integration,
                &RemoteValues {
                    forwarded: &forwarded,
                },
                settings,
            )
        })
        .collect()
}

/// The manifest check's one exception, checked again: a credential-suffix name whose value is
/// exactly one listener's host token (`${listen.<name>.token}` with `token_in_env`).
fn is_host_token(name: &str, template: &Template) -> bool {
    let [TemplatePart::ListenToken(listener)] = template.0.as_slice() else {
        return false;
    };
    let spec = ui_ext::terminal::TerminalEnvSpec {
        vars: BTreeMap::from([(name.to_owned(), format!("${{listen.{listener}.token}}"))]),
        scope: ui_ext::terminal::EnvScope::Profiles,
        reason: "recheck".into(),
    };
    ui_ext::terminal::check_terminal_env(spec, &|_| Some(true)).is_ok()
}

/// For the env hook: the env of every extension whose `terminal:env` the user allowed in every
/// new terminal.
pub(crate) fn all_new_env(
    accounts: &[FyxAccount],
    settings: &FyxAccountsSettingsContent,
    cx: &App,
) -> Vec<(String, String)> {
    let writer = crate::discovery::discovery_writer(cx);
    let relay = crate::relay::relay_binary(cx);
    integrations(cx)
        .iter()
        .filter(|(_, integration)| integration.env_all_new)
        .flat_map(|(ext, integration)| {
            let listeners = listeners_of(writer.as_deref(), ext);
            resolve_env(
                integration,
                accounts,
                &listeners,
                relay.as_deref(),
                settings,
            )
        })
        .collect()
}

/// `ext`'s open listeners, as the discovery writer knows them.
fn listeners_of<'a>(
    writer: Option<&'a crate::discovery::DiscoveryWriter>,
    ext: &'a str,
) -> impl Fn(&str) -> Option<(u16, String)> + 'a {
    move |name| writer?.listener(ext, name)
}

/// A "+" menu entry for a profile: which extension, which profile, which account.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileItem {
    ext: String,
    profile: String,
    /// The header the entry sits under ("Open Claude Code as…"), `None` for a profile without
    /// accounts.
    group: Option<String>,
    label: String,
    /// `<tool>=<name>`, or `None` for the project's account (if any).
    account_choice: Option<String>,
}

/// A shell started with a command to run (`-c`, `-e`, `/c`): what it runs is in its arguments,
/// not typed by the user.
fn runs_its_arguments(profile: &TerminalProfile) -> bool {
    profile.args.iter().any(|arg| match arg.0.as_slice() {
        [ui_ext::terminal::TemplatePart::Literal(text)] => {
            text == "-c" || text == "-e" || text.eq_ignore_ascii_case("/c")
        }
        _ => false,
    })
}

fn profile_items(
    integrations: &[(String, Arc<TerminalIntegration>)],
    settings: &FyxAccountsSettingsContent,
) -> Vec<ProfileItem> {
    let accounts = settings.accounts();
    // The extension's name only tells apart profiles of the same name.
    let mut names = HashMap::<&str, usize>::default();
    for (_, integration) in integrations {
        for profile in &integration.profiles {
            *names.entry(profile.name.as_str()).or_default() += 1;
        }
    }
    let mut items = Vec::new();
    for (ext, integration) in integrations {
        for profile in &integration.profiles {
            let marker = if !profile.runs_any_code() {
                ""
            } else if runs_its_arguments(profile) {
                " (runs the command in its arguments)"
            } else {
                " (runs any command you type)"
            };
            let name = if names
                .get(profile.name.as_str())
                .is_some_and(|count| *count > 1)
            {
                format!("{} ({})", profile.name, integration.extension_name)
            } else {
                profile.name.clone()
            };
            let tool_accounts = profile
                .account_tool
                .as_deref()
                .map(|tool| {
                    accounts
                        .iter()
                        .filter(|account| account.tool == tool)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if tool_accounts.is_empty() {
                items.push(ProfileItem {
                    ext: ext.clone(),
                    profile: profile.id.clone(),
                    group: None,
                    label: format!("Open {name}{marker}"),
                    account_choice: None,
                });
            }
            for account in tool_accounts {
                items.push(ProfileItem {
                    ext: ext.clone(),
                    profile: profile.id.clone(),
                    group: Some(format!("Open {name} as…")),
                    label: format!("{}{marker}", account.name),
                    account_choice: Some(accounts::account_choice(account)),
                });
            }
        }
    }
    items
}

/// "+" menu: the extensions' profiles.
pub(crate) fn menu_entries(cx: &App) -> Vec<MenuEntry> {
    let integrations = integrations(cx);
    let items = profile_items(&integrations, &accounts::settings(cx));
    if items.is_empty() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    let mut group = None;
    for item in items {
        if entries.is_empty() || item.group != group {
            entries.push(MenuEntry::Separator);
            if let Some(header) = &item.group {
                entries.push(MenuEntry::Header(header.clone().into()));
            }
            group = item.group.clone();
        }
        let label = SharedString::from(item.label.clone());
        entries.push(MenuEntry::Entry {
            label,
            on_select: Rc::new(move |window, cx| {
                launch(
                    &item.ext,
                    &item.profile,
                    item.account_choice.clone(),
                    window,
                    cx,
                )
            }),
        });
    }
    entries
}

/// Starts `ext`'s profile in the window's terminal panel.
fn launch(
    ext: &str,
    profile_id: &str,
    account_choice: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(workspace) = Workspace::for_window(window, cx) else {
        return;
    };
    let Some(integration) = integrations(cx)
        .into_iter()
        .find(|(id, _)| id == ext)
        .map(|(_, integration)| integration)
    else {
        return;
    };
    let Some(profile) = integration
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
    else {
        return;
    };
    let roots = local_roots(workspace.read(cx), cx);
    let trusted_root = roots
        .first()
        .filter(|root| crate::created::in_trusted_worktree(root, Some(window), cx))
        .cloned();
    let relay = crate::relay::relay_binary(cx);
    let start = match plan(
        profile,
        &integration,
        &roots,
        trusted_root.as_deref(),
        relay.as_deref(),
    ) {
        Ok(start) => start,
        Err(why) => {
            log::warn!("fyx: profile {ext}/{profile_id} did not start: {why}");
            return;
        }
    };

    let settings = accounts::settings(cx);
    let terminal_accounts =
        accounts::accounts_for(&settings, Some(&start.cwd), account_choice.as_deref());
    let terminal_settings = TerminalSettings::get_global(cx);
    let mut env = terminal_settings.env.clone();
    let writer = crate::discovery::discovery_writer(cx);
    let listeners = listeners_of(writer.as_deref(), ext);
    env.extend(resolve_env(
        &integration,
        &terminal_accounts,
        &listeners,
        relay.as_deref(),
        &settings,
    ));
    if let Some(choice) = account_choice {
        env.insert(ACCOUNT_CHOICE_ENV.to_owned(), choice);
    }
    let builder = TerminalBuilder::new(
        Some(start.cwd),
        TerminalMode::interactive(),
        task::Shell::WithArguments {
            program: start.program.to_string_lossy().into_owned(),
            args: start.args,
            title_override: None,
        },
        env,
        terminal_settings.cursor_shape,
        terminal_settings.alternate_scroll,
        terminal_settings.max_scroll_history_lines,
        terminal_settings.path_hyperlink_regexes.clone(),
        Duration::from_millis(terminal_settings.path_hyperlink_timeout_ms),
        false,
        window.window_handle().window_id().as_u64(),
        cx,
        Vec::new(),
        PathStyle::local(),
    );
    let title = profile.name.clone();
    let workspace = workspace.downgrade();
    let window = window.window_handle();
    cx.spawn(async move |cx| {
        let builder = match builder.await {
            Ok(builder) => builder,
            Err(error) => {
                log::warn!("fyx: a profile terminal did not start: {error:?}");
                return;
            }
        };
        window
            .update(cx, |_, window, cx| {
                let terminal = cx.new(|cx| builder.subscribe(cx));
                // 7.14 (b): the program's own title once it sets one, the profile's name before.
                terminal.read(cx).fyx.set_fallback_title(Some(title));
                workspace
                    .update(cx, |workspace, cx| {
                        crate::pseudo::add_tab(workspace, terminal, None, true, window, cx)
                    })
                    .ok();
            })
            .ok();
    })
    .detach();
}

fn local_roots(workspace: &Workspace, cx: &App) -> Vec<PathBuf> {
    let project = workspace.project().read(cx);
    if !project.is_local() {
        return Vec::new();
    }
    project
        .visible_worktrees(cx)
        .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Start {
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
}

/// Where and what a profile starts. `roots` are the window's local worktrees; `trusted_root` is
/// the first of them when it is trusted.
fn plan(
    profile: &TerminalProfile,
    integration: &TerminalIntegration,
    roots: &[PathBuf],
    trusted_root: Option<&Path>,
    relay: Option<&Path>,
) -> Result<Start, String> {
    let cwd = match profile.cwd {
        ProfileCwd::Worktree => roots.first().cloned().ok_or("no local worktree is open")?,
        ProfileCwd::Home => util::paths::home_dir().clone(),
    };
    let program = match &profile.path {
        Some(path) => {
            let root = trusted_root.ok_or("a worktree program needs a trusted worktree")?;
            in_worktree(root, path)?
        }
        None => on_path(
            &profile.program,
            &std::env::var_os("PATH").unwrap_or_default(),
            roots,
        )?,
    };
    let values = Values {
        package: integration.package_dir.as_deref(),
        accounts: &[],
        // Profile args never name a listener (`TemplateUse::ProfileArg`).
        listeners: &|_| None,
        relay,
    };
    let args = profile
        .args
        .iter()
        .map(|arg| arg.resolve(&values))
        .collect::<Option<Vec<_>>>()
        .ok_or("an argument names something Fyx does not provide yet")?;
    Ok(Start { program, args, cwd })
}

/// `relative` below `root`, with no `..`, absolute or prefix parts, and an existing file.
fn in_worktree(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!("{relative} leaves the worktree"));
    }
    let full = root.join(path);
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let canonical = full.canonicalize().map_err(|error| error.to_string())?;
    if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
        return Err(format!("{relative} is not a file in the worktree"));
    }
    if cfg!(windows) && is_batch(&canonical) {
        return Err(format!("{relative} is a batch script"));
    }
    Ok(canonical)
}

fn is_batch(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
        })
}

/// The program on `PATH`, skipping relative entries and any inside the worktrees (a program
/// planted in a repository never wins), and batch scripts on Windows. The rules of
/// `ui_ext_wasm::process::resolve_on_path`.
fn on_path(name: &str, path_var: &OsStr, roots: &[PathBuf]) -> Result<PathBuf, String> {
    if name.contains(['/', '\\']) {
        return Err(format!("{name} is a path, not a program name"));
    }
    let candidates = if cfg!(windows) {
        if is_batch(Path::new(name)) {
            return Err(format!("{name} is a batch script"));
        }
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".com") {
            vec![name.to_owned()]
        } else {
            vec![format!("{name}.exe"), format!("{name}.com")]
        }
    } else {
        vec![name.to_owned()]
    };
    let roots = roots
        .iter()
        .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
        .collect::<Vec<_>>();
    for dir in std::env::split_paths(path_var) {
        if !dir.is_absolute() {
            continue;
        }
        let Ok(dir) = dir.canonicalize() else {
            continue;
        };
        if roots.iter().any(|root| dir.starts_with(root)) {
            continue;
        }
        for candidate in &candidates {
            let path = dir.join(candidate);
            if is_executable(&path) {
                return Ok(path);
            }
        }
    }
    Err(format!("{name} is not on PATH outside the worktrees"))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::terminal::TemplateUse;

    fn template(text: &str, usage: TemplateUse) -> Template {
        // `ide` has `token_in_env`.
        Template::parse(text, usage, &|name| Some(name == "ide")).unwrap()
    }

    fn profile(id: &str, account_tool: Option<&str>) -> TerminalProfile {
        TerminalProfile {
            id: id.into(),
            name: "Claude Code".into(),
            icon: None,
            program: "claude".into(),
            path: None,
            args: vec![template("--ide", TemplateUse::ProfileArg)],
            cwd: ProfileCwd::Worktree,
            account_tool: account_tool.map(Into::into),
        }
    }

    fn accounts_settings() -> FyxAccountsSettingsContent {
        settings::fallible_options::parse_json(
            r#"{
                "tools": { "claude": { "env": "CLAUDE_CONFIG_DIR" }, "amp": { "env": "MY_AMP_DIR" } },
                "profiles": [
                    { "name": "Personal", "tool": "claude", "dir": "/a/.claude" },
                    { "name": "Work", "tool": "claude", "dir": "/a/.claude-work" }
                ]
            }"#,
        )
        .0
        .unwrap()
    }

    fn integration(vars: &[(&str, &str)]) -> TerminalIntegration {
        TerminalIntegration {
            extension_name: "Claude".into(),
            profiles: vec![profile("claude", Some("claude"))],
            env: Some(TerminalEnv {
                vars: vars
                    .iter()
                    .map(|(name, value)| {
                        ((*name).to_owned(), template(value, TemplateUse::EnvValue))
                    })
                    .collect(),
                scope: ui_ext::terminal::EnvScope::Profiles,
            }),
            env_all_new: false,
            package_dir: Some(PathBuf::from("/pkg")),
        }
    }

    #[test]
    fn env_values_resolve_on_the_host_and_refused_names_drop() {
        let settings = accounts_settings();
        let work = settings.account("claude", "Work").unwrap();
        let env = resolve_env(
            &integration(&[
                ("ENABLE_IDE_INTEGRATION", "true"),
                ("CLAUDE_CODE_PLUGIN_DIRS", "${package}/claude-plugin"),
                ("CLAUDE_CODE_SSE_PORT", "${listen.ide.port}"),
                ("MY_DIR", "${account.claude}"),
                ("MY_AMP_DIR", "x"),
            ]),
            std::slice::from_ref(&work),
            &|_| None,
            None,
            &settings,
        );
        let package = Path::new("/pkg").to_string_lossy().into_owned();
        assert_eq!(
            env,
            [
                ("ENABLE_IDE_INTEGRATION".to_owned(), "true".to_owned()),
                (
                    "CLAUDE_CODE_PLUGIN_DIRS".to_owned(),
                    format!("{package}/claude-plugin")
                ),
                ("MY_DIR".to_owned(), work.dir.to_string_lossy().into_owned()),
            ],
            "no listener yet; a user's tool variable is never an extension's"
        );
        let none = resolve_env(
            &integration(&[("MY_DIR", "${account.claude}")]),
            &[],
            &|_| None,
            None,
            &settings,
        );
        assert!(none.is_empty(), "no account, no variable");
    }

    #[test]
    fn a_remote_shell_gets_forwarded_ports_and_no_secrets() {
        let settings = accounts_settings();
        let vars = integration(&[
            ("CLAUDE_CODE_SSE_PORT", "${listen.ide.port}"),
            ("HOOKS_URL", "http://127.0.0.1:${listen.hooks.port}/h"),
            ("TOOL_IDE_TOKEN", "${listen.ide.token}"),
            ("PLUGIN_DIRS", "${package}/claude-plugin"),
            ("CLAUDE_CONFIG_HOME", "${account.claude}"),
            ("TOOL_PID", "${pid}"),
            ("ENABLE_IDE_INTEGRATION", "true"),
        ]);
        let forwarded = |listener: &str| (listener == "ide").then_some(4100);
        let env = resolve_with(
            &vars,
            &RemoteValues {
                forwarded: &forwarded,
            },
            &settings,
        );
        assert_eq!(
            env,
            vec![
                ("CLAUDE_CODE_SSE_PORT".to_owned(), "4100".to_owned()),
                ("ENABLE_IDE_INTEGRATION".to_owned(), "true".to_owned()),
            ],
            "the hooks port is not forwarded; tokens, paths and the pid stay local"
        );
    }

    #[test]
    fn listen_placeholders_take_the_open_listener() {
        let settings = accounts_settings();
        let vars = integration(&[
            ("CLAUDE_CODE_SSE_PORT", "${listen.ide.port}"),
            ("TOOL_IDE_TOKEN", "${listen.ide.token}"),
            ("HOOKS_URL", "http://127.0.0.1:${listen.hooks.port}/h"),
            ("GITHUB_TOKEN", "${listen.ide.token}"),
            ("FYX_TOKEN", "${listen.ide.token}"),
        ]);
        let open = |name: &str| (name == "ide").then(|| (4100, "secret".to_owned()));
        assert_eq!(
            resolve_env(&vars, &[], &open, None, &settings),
            [
                ("CLAUDE_CODE_SSE_PORT".to_owned(), "4100".to_owned()),
                ("TOOL_IDE_TOKEN".to_owned(), "secret".to_owned()),
            ],
            "a listener that is not open leaves its variable out; a credential or Fyx name never \
             carries the token, whatever the manifest check let through"
        );
    }

    #[test]
    fn profiles_with_an_account_tool_get_one_entry_per_account_under_their_header() {
        let mut other = integration(&[]);
        other.extension_name = "Shells".into();
        other.profiles = vec![TerminalProfile {
            name: "Bash".into(),
            program: "bash".into(),
            ..profile("bash", None)
        }];
        let items = profile_items(
            &[
                ("claude".into(), Arc::new(integration(&[]))),
                ("shells".into(), Arc::new(other)),
            ],
            &accounts_settings(),
        );
        let labels = items
            .iter()
            .map(|item| {
                (
                    item.group.as_deref(),
                    item.label.as_str(),
                    item.account_choice.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                (
                    Some("Open Claude Code as…"),
                    "Personal",
                    Some("claude=Personal")
                ),
                (Some("Open Claude Code as…"), "Work", Some("claude=Work")),
                (None, "Open Bash (runs any command you type)", None),
            ]
        );
        let mut scripted = integration(&[]);
        scripted.extension_name = "Shells".into();
        scripted.profiles = ["-c", "-e", "/C"]
            .into_iter()
            .map(|flag| TerminalProfile {
                name: format!("Shell {flag}"),
                program: if flag == "/C" { "cmd" } else { "bash" }.into(),
                args: vec![
                    template(flag, TemplateUse::ProfileArg),
                    template("make watch", TemplateUse::ProfileArg),
                ],
                ..profile("shell", None)
            })
            .collect();
        let scripted = profile_items(
            &[("shells".into(), Arc::new(scripted))],
            &accounts_settings(),
        );
        assert_eq!(
            scripted
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Open Shell -c (runs the command in its arguments)",
                "Open Shell -e (runs the command in its arguments)",
                "Open Shell /C (runs the command in its arguments)",
            ]
        );
        let no_accounts = profile_items(
            &[("claude".into(), Arc::new(integration(&[])))],
            &FyxAccountsSettingsContent::default(),
        );
        assert_eq!(no_accounts[0].label, "Open Claude Code");
        let same_name = profile_items(
            &[
                ("claude".into(), Arc::new(integration(&[]))),
                ("other".into(), Arc::new(integration(&[]))),
            ],
            &FyxAccountsSettingsContent::default(),
        );
        assert_eq!(
            same_name
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["Open Claude Code (Claude)", "Open Claude Code (Claude)"],
            "two profiles of the same name name their extensions"
        );
    }

    #[test]
    fn programs_are_found_outside_the_worktrees_only() {
        let base = std::env::temp_dir().join(format!("fyx-profiles-{}", std::process::id()));
        let repo = base.join("repo");
        let tools = base.join("tools");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        let file = if cfg!(windows) { "tool.exe" } else { "tool" };
        for dir in [&repo, &tools] {
            std::fs::write(dir.join(file), b"").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(dir.join(file), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }
        let path_var = std::env::join_paths([repo.clone(), tools.clone()]).unwrap();
        let roots = [repo.clone()];
        let found = on_path("tool", &path_var, &roots).unwrap();
        assert_eq!(found, tools.canonicalize().unwrap().join(file));
        let only_repo = std::env::join_paths([repo.clone()]).unwrap();
        assert!(on_path("tool", &only_repo, &roots).is_err());
        assert!(on_path("../tool", &path_var, &roots).is_err());
        if cfg!(windows) {
            assert!(on_path("tool.cmd", &path_var, &roots).is_err());
        }

        assert!(in_worktree(&repo, "../tools/tool").is_err());
        assert_eq!(
            in_worktree(&repo, file).unwrap(),
            repo.canonicalize().unwrap().join(file)
        );

        let mut worktree_profile = profile("local", None);
        worktree_profile.path = Some(file.into());
        let integration = integration(&[]);
        assert!(
            plan(&worktree_profile, &integration, &roots, None, None).is_err(),
            "untrusted"
        );
        let start = plan(&worktree_profile, &integration, &roots, Some(&repo), None).unwrap();
        assert_eq!(start.cwd, repo);
        assert_eq!(start.args, ["--ide"]);

        let mut relay = worktree_profile;
        relay.args = vec![template("${relay}", TemplateUse::ProfileArg)];
        assert!(
            plan(&relay, &integration, &roots, Some(&repo), None).is_err(),
            "no relay binary"
        );
        let binary = repo.join("fyx-relay");
        let start = plan(&relay, &integration, &roots, Some(&repo), Some(&binary)).unwrap();
        assert_eq!(start.args, [binary.to_string_lossy().into_owned()]);
        std::fs::remove_dir_all(&base).ok();
    }

    #[gpui::test]
    fn the_env_reaches_every_new_terminal_only_when_allowed(cx: &mut App) {
        let settings = FyxAccountsSettingsContent::default();
        let mut claude = integration(&[
            ("ENABLE_IDE_INTEGRATION", "true"),
            ("CLAUDE_CONFIG_DIR", "/elsewhere"),
        ]);
        set("claude".into(), claude.clone(), cx);
        assert!(all_new_env(&[], &settings, cx).is_empty(), "profiles only");
        claude.env_all_new = true;
        set("claude".into(), claude, cx);
        assert_eq!(
            all_new_env(&[], &settings, cx),
            [("ENABLE_IDE_INTEGRATION".to_owned(), "true".to_owned())]
        );
        set("claude".into(), TerminalIntegration::default(), cx);
        assert!(all_new_env(&[], &settings, cx).is_empty(), "removed");
        assert!(menu_entries(cx).is_empty());
    }

    #[test]
    fn close_all_removes_the_integration() {
        let (commands, mut queued) = futures::channel::mpsc::unbounded();
        let host = crate::HostFront::new(commands);
        let integration = integration(&[]);
        ui_ext::terminal::TerminalHost::set_integration(
            &host,
            "claude",
            ui_ext::terminal::TerminalIntegration {
                profiles: integration.profiles,
                env: integration.env,
                env_all_new: integration.env_all_new,
                package_dir: integration.package_dir,
            },
        );
        ui_ext::terminal::TerminalHost::close_all(&host, "claude");
        let mut integrations = Vec::new();
        while let Ok(command) = queued.try_recv() {
            if let crate::host::Command::Integration(ext, integration) = command {
                integrations.push((ext, integration.is_empty()));
            }
        }
        assert_eq!(
            integrations,
            [("claude".to_owned(), false), ("claude".to_owned(), true)]
        );
    }
}
