//! Account profiles (`fyx.accounts`, design §C.6): which config dir a terminal's tool uses, chosen
//! per project in user settings, shown as a badge on the tab and in the sessions list.
//!
//! Fyx never touches credentials: an account is a name, a colour and an opaque dir that only goes
//! into an environment variable. Nothing here opens, lists or checks the dir. There is no automatic
//! switching and no "try another account": changing a project's account applies to terminals opened
//! after the change, and each tab keeps the account it started with.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock};

use gpui::{AnyElement, App, Hsla, SharedString, Window};
use settings::settings_content::{FyxAccount, FyxAccountColor, FyxAccountsSettingsContent};
use settings::{RegisterSetting, Settings, SettingsStore};
use terminal_ext::{AccountBadge, EnvAdditions, EnvRequest, MenuEntry, TerminalEnvHook};
use ui::{Indicator, prelude::*};
use workspace::Workspace;

/// `fyx.accounts`, from user settings only (project settings have no `fyx` key).
#[derive(Clone, Debug, Default, RegisterSetting)]
pub struct FyxAccountsSettings(pub FyxAccountsSettingsContent);

impl Settings for FyxAccountsSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self(
            content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.accounts.clone())
                .unwrap_or_default(),
        )
    }
}

pub(crate) fn init(cx: &mut App) {
    terminal_ext::set_env_hook(AccountsEnv, cx);
}

pub(crate) fn settings(cx: &App) -> FyxAccountsSettingsContent {
    if cx.has_global::<SettingsStore>() {
        FyxAccountsSettings::get_global(cx).0.clone()
    } else {
        FyxAccountsSettingsContent::default()
    }
}

/// [`terminal_ext::ACCOUNT_CHOICE_ENV`] for a terminal opened for `account`.
pub fn account_choice(account: &FyxAccount) -> String {
    format!("{}={}", account.tool, account.name)
}

/// [`terminal_accounts`] for a local terminal.
pub(crate) fn accounts_for(
    settings: &FyxAccountsSettingsContent,
    cwd: Option<&Path>,
    account_choice: Option<&str>,
) -> Vec<FyxAccount> {
    terminal_accounts(
        settings,
        &EnvRequest {
            cwd,
            remote: false,
            account_choice,
        },
    )
}

/// The accounts a new terminal gets: none on another machine or when opened for none; else the
/// project's choice for each tool, with the account it was opened for replacing its tool's.
fn terminal_accounts(
    settings: &FyxAccountsSettingsContent,
    request: &EnvRequest<'_>,
) -> Vec<FyxAccount> {
    if request.remote || request.account_choice == Some("none") {
        return Vec::new();
    }
    let mut accounts = request
        .cwd
        .map(|cwd| settings.project_accounts(cwd))
        .unwrap_or_default();
    let chosen = request
        .account_choice
        .and_then(|choice| choice.split_once('='))
        .and_then(|(tool, name)| settings.account(tool, name));
    if let Some(chosen) = chosen {
        accounts.retain(|account| account.tool != chosen.tool);
        accounts.push(chosen);
        accounts.sort_by(|a, b| a.tool.cmp(&b.tool));
    }
    accounts
}

fn badge_of(account: &FyxAccount) -> AccountBadge {
    AccountBadge {
        tool: account.tool.clone(),
        name: account.name.clone(),
        color: color_name(account.color).to_owned(),
    }
}

struct AccountsEnv;

impl TerminalEnvHook for AccountsEnv {
    fn env_for(&self, request: &EnvRequest<'_>, cx: &App) -> EnvAdditions {
        let settings = settings(cx);
        let accounts = terminal_accounts(&settings, request);
        let mut vars = accounts
            .iter()
            .map(FyxAccount::env_entry)
            .collect::<Vec<_>>();
        if !request.remote {
            // The relay of this Fyx (§B.9); the names are Fyx's, so no extension sets them.
            vars.extend(crate::relay::relay_env(cx));
            // Extensions the user allowed into every new terminal (§B.4), after the accounts so
            // they cannot replace an account's variable.
            let names = vars
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            vars.splice(
                0..0,
                crate::profiles::all_new_env(&accounts, &settings, cx)
                    .into_iter()
                    .filter(|(name, _)| !names.iter().any(|own| own.eq_ignore_ascii_case(name))),
            );
        }
        EnvAdditions {
            vars,
            accounts: accounts.iter().map(badge_of).collect(),
        }
    }

    /// Plan 5.17: no accounts on another host; extensions' all-new env without secrets.
    fn remote_env(&self, remote: gpui::EntityId, cx: &App) -> Vec<(String, String)> {
        crate::profiles::remote_all_new_env(remote, &settings(cx), cx)
    }
}

fn color_name(color: FyxAccountColor) -> &'static str {
    match color {
        FyxAccountColor::Blue | FyxAccountColor::Unrecognized => "blue",
        FyxAccountColor::Green => "green",
        FyxAccountColor::Yellow => "yellow",
        FyxAccountColor::Red => "red",
        FyxAccountColor::Magenta => "magenta",
        FyxAccountColor::Cyan => "cyan",
    }
}

fn theme_color(name: &str, cx: &App) -> Hsla {
    let colors = cx.theme().colors();
    match name {
        "green" => colors.terminal_ansi_green,
        "yellow" => colors.terminal_ansi_yellow,
        "red" => colors.terminal_ansi_red,
        "magenta" => colors.terminal_ansi_magenta,
        "cyan" => colors.terminal_ansi_cyan,
        _ => colors.terminal_ansi_blue,
    }
}

/// The account's name with a dot in its colour, for a tab or a sessions row.
pub(crate) fn badge(badge: &AccountBadge, cx: &App) -> AnyElement {
    h_flex()
        .gap_0p5()
        .child(Indicator::dot().color(Color::Custom(theme_color(&badge.color, cx))))
        .child(
            Label::new(badge.name.clone())
                .size(LabelSize::XSmall)
                .color(Color::Muted)
                .single_line(),
        )
        .into_any_element()
}

/// The tooltip line naming a terminal's accounts, e.g. "Claude Code account: Work".
pub(crate) fn describe(badges: &[AccountBadge], cx: &App) -> Option<String> {
    let settings = settings(cx);
    let lines = badges
        .iter()
        .map(|badge| {
            format!(
                "{} account: {}",
                settings.tool_label(&badge.tool),
                badge.name
            )
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join(" · "))
}

/// The project folder new terminals and agents of this window are matched by: the first visible
/// worktree's root.
fn project_folder(workspace: &Workspace, cx: &App) -> Option<PathBuf> {
    let project = workspace.project().read(cx);
    if !project.is_local() {
        return None;
    }
    project
        .visible_worktrees(cx)
        .next()
        .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
}

/// "+" menu: per tool with accounts, "<Tool> account in this project" and a choice of them.
pub(crate) fn menu_entries(window: &Window, cx: &App) -> Vec<MenuEntry> {
    let Some(workspace) = Workspace::for_window(window, cx) else {
        return Vec::new();
    };
    let Some(folder) = project_folder(workspace.read(cx), cx) else {
        return Vec::new();
    };
    let fs = workspace.read(cx).app_state().fs.clone();
    let settings = settings(cx);
    menu_model(&settings, &folder)
        .into_iter()
        .map(|item| match item {
            MenuItem::Separator => MenuEntry::Separator,
            MenuItem::Header(title) => MenuEntry::Header(title.into()),
            MenuItem::Choice {
                label,
                checked,
                tool,
                account,
            } => {
                let fs = fs.clone();
                let folder = folder.clone();
                MenuEntry::Toggle {
                    label: SharedString::from(label),
                    checked,
                    on_select: Rc::new(move |_, cx| {
                        let (folder, tool, account) =
                            (folder.clone(), tool.clone(), account.clone());
                        settings::update_settings_file(fs.clone(), cx, move |content, _| {
                            set_project_account(content, &folder, &tool, account.as_deref());
                        });
                    }),
                }
            }
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum MenuItem {
    Separator,
    Header(String),
    Choice {
        label: String,
        checked: bool,
        tool: String,
        /// `None`: no account (the tool's default dir).
        account: Option<String>,
    },
}

fn menu_model(settings: &FyxAccountsSettingsContent, folder: &Path) -> Vec<MenuItem> {
    let accounts = settings.accounts();
    let mut tools = accounts
        .iter()
        .map(|account| account.tool.clone())
        .collect::<Vec<_>>();
    tools.sort();
    tools.dedup();
    let current = settings.project_accounts(folder);
    let mut items = Vec::new();
    for tool in tools {
        let chosen = current
            .iter()
            .find(|account| account.tool == tool)
            .map(|account| account.name.clone());
        items.push(MenuItem::Separator);
        items.push(MenuItem::Header(format!(
            "{} account in this project",
            settings.tool_label(&tool)
        )));
        for account in accounts.iter().filter(|account| account.tool == tool) {
            items.push(MenuItem::Choice {
                label: account.name.clone(),
                checked: chosen.as_deref() == Some(account.name.as_str()),
                tool: tool.clone(),
                account: Some(account.name.clone()),
            });
        }
        items.push(MenuItem::Choice {
            label: "Default (no profile)".into(),
            checked: chosen.is_none(),
            tool,
            account: None,
        });
    }
    items
}

/// Writes "in this folder, `tool` uses `account`" into user settings; `None` removes the choice.
fn set_project_account(
    content: &mut settings::SettingsContent,
    folder: &Path,
    tool: &str,
    account: Option<&str>,
) {
    let key = folder.to_string_lossy().into_owned();
    let projects = content
        .fyx
        .get_or_insert_default()
        .accounts
        .get_or_insert_default()
        .projects
        .get_or_insert_default();
    match account {
        Some(account) => {
            projects
                .entry(key)
                .or_default()
                .insert(tool.to_owned(), account.to_owned());
        }
        None => {
            if let Some(choices) = projects.get_mut(&key) {
                choices.remove(tool);
                if choices.is_empty() {
                    projects.remove(&key);
                }
            }
        }
    }
}

/// `ui_ext::terminal::AccountDirs` (plan 5.20, §C.6) for the extension runtime: a snapshot of
/// `fyx.accounts`, replaced on every settings change. The runtime calls it off the main thread,
/// so it never reads `cx`.
struct HostAccountDirs(Arc<RwLock<FyxAccountsSettingsContent>>);

impl HostAccountDirs {
    fn snapshot(&self) -> std::sync::RwLockReadGuard<'_, FyxAccountsSettingsContent> {
        self.0.read().unwrap_or_else(|poison| poison.into_inner())
    }
}

impl ui_ext::terminal::AccountDirs for HostAccountDirs {
    fn dirs(&self) -> Vec<PathBuf> {
        self.snapshot()
            .accounts()
            .into_iter()
            .map(|account| account.dir)
            .collect()
    }

    fn env_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .snapshot()
            .tools
            .iter()
            .flatten()
            .filter_map(|(_, tool)| Some(tool.env.as_deref()?.trim().to_owned()))
            .filter(|name| !name.is_empty())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    fn account_dir(&self, tool: &str, cwd: &Path) -> Option<PathBuf> {
        // The project's choice only: no choice means no dir, never another account of the tool.
        self.snapshot()
            .project_account(tool, [cwd])
            .map(|account| account.dir)
    }
}

/// The account profiles for `ui_ext_wasm::set_account_dirs`, kept current with user settings.
/// Called once by `main.rs`, before any extension loads.
pub fn account_dirs(cx: &mut App) -> Arc<dyn ui_ext::terminal::AccountDirs> {
    let snapshot = Arc::new(RwLock::new(settings(cx)));
    if cx.has_global::<SettingsStore>() {
        let watched = snapshot.clone();
        cx.observe_global::<SettingsStore>(move |cx| {
            *watched.write().unwrap_or_else(|poison| poison.into_inner()) = settings(cx);
        })
        .detach();
    }
    Arc::new(HostAccountDirs(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> FyxAccountsSettingsContent {
        settings::fallible_options::parse_json(
            r#"{
                "tools": { "claude": { "env": "CLAUDE_CONFIG_DIR", "label": "Claude Code" },
                           "codex": { "env": "CODEX_HOME" } },
                "profiles": [
                    { "name": "Personal", "tool": "claude", "dir": "/a/.claude" },
                    { "name": "Work", "tool": "claude", "dir": "/a/.claude-work", "color": "red" },
                    { "name": "Me", "tool": "codex", "dir": "/a/.codex" }
                ],
                "projects": { "/src/work": { "claude": "Work" } }
            }"#,
        )
        .0
        .unwrap()
    }

    fn names(accounts: &[FyxAccount]) -> Vec<&str> {
        accounts
            .iter()
            .map(|account| account.name.as_str())
            .collect()
    }

    #[test]
    fn fyx_defaults_have_the_tool_presets() {
        let (content, _) = settings::fallible_options::parse_json::<settings::SettingsContent>(
            include_str!("../../../assets/settings/fyx.json"),
        );
        let accounts = content.unwrap().fyx.unwrap().accounts.unwrap();
        let tools = accounts.tools.as_ref().unwrap();
        for (tool, env) in [
            ("claude", "CLAUDE_CONFIG_DIR"),
            ("codex", "CODEX_HOME"),
            ("gemini", "GEMINI_CLI_HOME"),
            ("copilot", "COPILOT_HOME"),
            ("opencode", "OPENCODE_CONFIG_DIR"),
        ] {
            assert_eq!(tools[tool].env.as_deref(), Some(env));
            assert!(
                ui_ext::terminal::is_account_env(env),
                "{env} is refused for extensions"
            );
        }
        assert_eq!(accounts.tool_for_agent("claude-acp"), Some("claude"));
        assert!(accounts.accounts().is_empty());
    }

    #[test]
    fn terminals_get_the_project_account_or_the_one_they_were_opened_for() {
        let settings = two();
        let request = |cwd: &'static str, choice: Option<&'static str>| EnvRequest {
            cwd: Some(Path::new(cwd)),
            remote: false,
            account_choice: choice,
        };
        let work = terminal_accounts(&settings, &request("/src/work/api", None));
        assert_eq!(names(&work), ["Work"]);
        assert_eq!(
            work[0].env_entry().0,
            "CLAUDE_CONFIG_DIR",
            "only the tool's config-dir variable"
        );
        assert!(terminal_accounts(&settings, &request("/elsewhere", None)).is_empty());
        assert_eq!(
            names(&terminal_accounts(
                &settings,
                &request("/src/work", Some("claude=Personal"))
            )),
            ["Personal"],
            "two terminals of one project, two accounts"
        );
        assert_eq!(
            names(&terminal_accounts(
                &settings,
                &request("/src/work", Some("codex=Me"))
            )),
            ["Work", "Me"]
        );
        assert!(terminal_accounts(&settings, &request("/src/work", Some("none"))).is_empty());
        assert_eq!(
            names(&terminal_accounts(
                &settings,
                &request("/src/work", Some("claude=Nobody"))
            )),
            ["Work"],
            "an unknown choice falls back to the project's"
        );
        let remote = EnvRequest {
            remote: true,
            ..request("/src/work", Some("claude=Personal"))
        };
        assert!(terminal_accounts(&settings, &remote).is_empty());
    }

    #[test]
    fn the_menu_offers_each_tools_accounts_with_the_projects_checked() {
        let items = menu_model(&two(), Path::new("/src/work"));
        let checked = items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Choice {
                    label,
                    checked: true,
                    ..
                } => Some(label.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(checked, ["Work", "Default (no profile)"]);
        assert!(items.contains(&MenuItem::Header(
            "Claude Code account in this project".into()
        )));
        assert!(items.contains(&MenuItem::Header("codex account in this project".into())));
        assert!(menu_model(&FyxAccountsSettingsContent::default(), Path::new("/x")).is_empty());
    }

    #[test]
    fn choosing_writes_user_settings_by_folder() {
        let mut content = settings::SettingsContent::default();
        let folder = Path::new("/src/side");
        set_project_account(&mut content, folder, "claude", Some("Personal"));
        let projects = |content: &settings::SettingsContent| {
            content
                .fyx
                .clone()
                .unwrap()
                .accounts
                .unwrap()
                .projects
                .unwrap()
        };
        let key = folder.to_string_lossy().into_owned();
        assert_eq!(projects(&content)[&key]["claude"], "Personal".to_owned());
        set_project_account(&mut content, folder, "claude", None);
        assert!(projects(&content).is_empty());
    }

    /// Two terminals in one project, through the real `TerminalBuilder` and pty: one gets the
    /// project's account, one was opened for another; each program sees its own dir and each tab
    /// its own badge.
    #[gpui::test]
    async fn two_accounts_in_two_terminals(cx: &mut gpui::TestAppContext) {
        use std::collections::HashMap;
        use std::time::Duration;

        use gpui::UpdateGlobal as _;
        use terminal::{TerminalBuilder, TerminalMode};
        use util::paths::PathStyle;

        cx.executor().allow_parking();
        let folder = std::env::temp_dir().join(format!("fyx-accounts-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let json_path = |path: &Path| path.to_string_lossy().replace('\\', "\\\\");
        let personal = folder.join("claude-personal");
        let work = folder.join("claude-work");
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
            SettingsStore::update_global(cx, |store, cx| {
                store
                    .set_user_settings(
                        &format!(
                            r#"{{ "fyx": {{ "accounts": {{
                                "tools": {{ "claude": {{ "env": "CLAUDE_CONFIG_DIR" }} }},
                                "profiles": [
                                    {{ "name": "Personal", "tool": "claude", "dir": "{}" }},
                                    {{ "name": "Work", "tool": "claude", "dir": "{}", "color": "red" }}
                                ],
                                "projects": {{ "{}": {{ "claude": "Work" }} }}
                            }} }} }}"#,
                            json_path(&personal),
                            json_path(&work),
                            json_path(&folder)
                        ),
                        cx,
                    )
                    .result()
                    .unwrap();
            });
            init(cx);
        });

        let mut terminals = Vec::new();
        for (out, choice) in [
            ("plain.txt", None),
            ("personal.txt", Some("claude=Personal")),
        ] {
            let out = folder.join(out);
            std::fs::remove_file(&out).ok();
            #[cfg(windows)]
            let (program, args) = (
                "cmd.exe".to_owned(),
                vec![
                    "/C".to_owned(),
                    format!("echo %CLAUDE_CONFIG_DIR%> \"{}\"", out.display()),
                ],
            );
            #[cfg(not(windows))]
            let (program, args) = (
                "/bin/sh".to_owned(),
                vec![
                    "-c".to_owned(),
                    format!("printf %s \"$CLAUDE_CONFIG_DIR\" > '{}'", out.display()),
                ],
            );
            let env = choice
                .map(|choice: &str| {
                    (
                        terminal_ext::ACCOUNT_CHOICE_ENV.to_owned(),
                        choice.to_owned(),
                    )
                })
                .into_iter()
                .collect::<HashMap<_, _>>();
            let builder = cx
                .update(|cx| {
                    TerminalBuilder::new(
                        Some(folder.clone()),
                        TerminalMode::interactive(),
                        task::Shell::WithArguments {
                            program,
                            args,
                            title_override: None,
                        },
                        env.into_iter().collect(),
                        Default::default(),
                        terminal::terminal_settings::AlternateScroll::On,
                        None,
                        Vec::new(),
                        Duration::ZERO,
                        false,
                        0,
                        cx,
                        Vec::new(),
                        PathStyle::local(),
                    )
                })
                .await
                .unwrap();
            let terminal = cx.new(|cx| builder.subscribe(cx));
            terminals.push((terminal, out));
        }

        let badges = |index: usize, cx: &mut gpui::TestAppContext| {
            terminals[index].0.read_with(cx, |terminal, _| {
                terminal
                    .fyx
                    .accounts()
                    .into_iter()
                    .map(|badge| (badge.name, badge.color))
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(badges(0, cx), [("Work".to_owned(), "red".to_owned())]);
        assert_eq!(badges(1, cx), [("Personal".to_owned(), "blue".to_owned())]);

        for ((_, out), expected) in terminals.iter().zip([&work, &personal]) {
            // The pty's program is real: wait for it, not for the test scheduler.
            let mut seen = String::new();
            for _ in 0..400 {
                seen = std::fs::read_to_string(out).unwrap_or_default();
                if !seen.trim().is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            assert_eq!(seen.trim(), expected.to_string_lossy());
        }
        drop(terminals);
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn account_dirs_follow_the_snapshot() {
        use ui_ext::terminal::AccountDirs as _;
        let snapshot = Arc::new(RwLock::new(two()));
        let dirs = HostAccountDirs(snapshot.clone());
        assert_eq!(
            dirs.dirs(),
            [
                PathBuf::from("/a/.claude"),
                PathBuf::from("/a/.claude-work"),
                PathBuf::from("/a/.codex")
            ]
        );
        assert_eq!(dirs.env_names(), ["CLAUDE_CONFIG_DIR", "CODEX_HOME"]);
        let project = Path::new("/src/work/api");
        assert_eq!(
            dirs.account_dir("claude", project),
            Some(PathBuf::from("/a/.claude-work"))
        );
        // No choice for the tool, or no project: no dir, never another account.
        assert_eq!(dirs.account_dir("codex", project), None);
        assert_eq!(dirs.account_dir("claude", Path::new("/src/other")), None);

        *snapshot.write().unwrap() = FyxAccountsSettingsContent::default();
        assert!(dirs.dirs().is_empty());
        assert_eq!(dirs.account_dir("claude", project), None);
    }
}
