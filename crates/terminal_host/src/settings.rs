//! `fyx.terminal` in settings (schema in `settings_content::FyxTerminalSettingsContent`, default in
//! `assets/settings/fyx.json`).

use gpui::App;
use settings::{RegisterSetting, Settings};

#[derive(Clone, Debug, RegisterSetting)]
pub struct FyxTerminalSettings {
    pub notifications: bool,
}

impl Settings for FyxTerminalSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            notifications: content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.terminal.as_ref())
                .and_then(|terminal| terminal.notifications)
                .unwrap_or(true),
        }
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
}
