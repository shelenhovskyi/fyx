//! `fyx.terminal` in settings (schema in `settings_content::FyxTerminalSettingsContent`, default in
//! `assets/settings/fyx.json`).

use gpui::App;
use settings::{RegisterSetting, Settings};

#[derive(Clone, Debug, RegisterSetting)]
pub struct FyxTerminalSettings {
    pub notifications: bool,
    pub shell_integration: bool,
    /// USD; `None` for no cap.
    pub daily_cost_cap: Option<f64>,
    /// Plan 7.14 (a).
    pub restore_profile_terminals: bool,
    pub resume_profile_sessions: bool,
}

impl Settings for FyxTerminalSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let terminal = content.fyx.as_ref().and_then(|fyx| fyx.terminal.as_ref());
        Self {
            notifications: terminal
                .and_then(|terminal| terminal.notifications)
                .unwrap_or(true),
            shell_integration: terminal
                .and_then(|terminal| terminal.shell_integration)
                .unwrap_or(true),
            daily_cost_cap: terminal
                .and_then(|terminal| terminal.daily_cost_cap)
                .filter(|cap| *cap > 0.0)
                .map(f64::from),
            restore_profile_terminals: terminal
                .and_then(|terminal| terminal.restore_profile_terminals)
                .unwrap_or(true),
            resume_profile_sessions: terminal
                .and_then(|terminal| terminal.resume_profile_sessions)
                .unwrap_or(true),
        }
    }
}

/// Plan 5.17: keeps `terminal_ext`'s shell integration on in `dir` while the setting says so.
pub fn init_shell_integration(dir: std::path::PathBuf, cx: &mut App) {
    let apply = move |cx: &mut App| {
        let enabled = !cx.has_global::<settings::SettingsStore>()
            || FyxTerminalSettings::get_global(cx).shell_integration;
        let wanted = enabled.then(|| dir.clone());
        let current = terminal_ext::shell_integration(cx).map(|shell| shell.dir().to_path_buf());
        if wanted != current {
            terminal_ext::set_shell_integration(wanted, cx);
        }
    };
    apply(cx);
    if cx.has_global::<settings::SettingsStore>() {
        cx.observe_global::<settings::SettingsStore>(apply).detach();
    }
}

/// Whether to pop up notifications; on when no settings are loaded (tests).
pub fn notifications_enabled(cx: &App) -> bool {
    !cx.has_global::<settings::SettingsStore>() || FyxTerminalSettings::get_global(cx).notifications
}

#[cfg(test)]
mod tests {
    use gpui::UpdateGlobal as _;
    use settings::SettingsStore;

    use super::*;

    #[gpui::test]
    fn notifications_follow_the_setting(cx: &mut App) {
        let store = SettingsStore::test(cx);
        cx.set_global(store);
        assert!(notifications_enabled(cx), "on by default");
        SettingsStore::update_global(cx, |store, cx| {
            store
                .set_user_settings(
                    r#"{ "fyx": { "terminal": { "notifications": false } } }"#,
                    cx,
                )
                .result()
                .unwrap();
        });
        assert!(!notifications_enabled(cx));
    }

    #[gpui::test]
    fn shell_integration_follows_the_setting(cx: &mut gpui::TestAppContext) {
        let dir = std::path::PathBuf::from("/data/shell");
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
            init_shell_integration(dir.clone(), cx);
        });
        let current = |cx: &mut gpui::TestAppContext| {
            cx.update(|cx| terminal_ext::shell_integration(cx).map(|s| s.dir().to_path_buf()))
        };
        assert_eq!(current(cx), Some(dir), "on by default");
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store
                    .set_user_settings(
                        r#"{ "fyx": { "terminal": { "shell_integration": false } } }"#,
                        cx,
                    )
                    .result()
                    .unwrap();
            });
        });
        assert_eq!(current(cx), None);
    }
}
