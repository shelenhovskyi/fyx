//! 0.4 extension settings, runtime side (design wit-0.4 §D.2, §D.4, plan 6.1): the checked
//! declarations, the effective values the `settings` import answers from, and the keys a store
//! change sends as `settings-changed`.
//!
//! The host reads the extension's object under `fyx_extensions.<id>` from the **user** settings
//! file only (never project settings, §D.2) and hands it to [`SettingsHandle::update`] on every
//! settings-file change; this module never reads a file. The values are fixed for one export
//! call: the import reads a snapshot, and a change made during a call is delivered after it.

use std::sync::{Mutex, PoisonError};

use ui_ext::settings::{EffectiveSettings, RawValue, Setting, SettingSpec, SettingValue};
use ui_ext::v0_2::{HostError, HostErrorCode};

/// One extension instance's settings.
#[derive(Debug, Default)]
pub(crate) struct SettingsHandle {
    specs: Vec<SettingSpec>,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    effective: EffectiveSettings,
    /// `settings-changed` lists not yet delivered, oldest first.
    changed: Vec<Vec<String>>,
}

impl SettingsHandle {
    /// `specs` are the manifest's checked `[[fyx_ui.settings]]` (none below 0.4), `user` the
    /// user's object for this extension at load.
    pub(crate) fn new(specs: Vec<SettingSpec>, user: Option<&RawValue>) -> Self {
        let effective = EffectiveSettings::compute(&specs, user);
        Self {
            specs,
            state: Mutex::new(State {
                effective,
                changed: Vec::new(),
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn specs(&self) -> &[SettingSpec] {
        &self.specs
    }

    /// What the extension gets now, with the form's errors and unknown keys.
    pub(crate) fn effective(&self) -> EffectiveSettings {
        self.state().effective.clone()
    }

    /// `settings.get`: `invalid-argument` for a key the manifest does not declare.
    // Linked by the `settings` import once W0 stages it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn get(&self, key: &str) -> Result<SettingValue, HostError> {
        self.state()
            .effective
            .get(key)
            .map(|setting| setting.value.clone())
            .ok_or_else(|| HostError {
                code: HostErrorCode::InvalidArgument,
                message: format!("no setting {key:?} is declared in [[fyx_ui.settings]]"),
            })
    }

    /// `settings.all`: every declared key, in manifest order.
    // Linked by the `settings` import once W0 stages it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn all(&self) -> Vec<Setting> {
        self.state().effective.settings.clone()
    }

    /// The user's object changed: recomputes and returns the keys whose effective value
    /// changed, in manifest order, also queued for [`SettingsHandle::take_changed`]. Nothing is
    /// queued when no effective value changed (§D.4).
    pub(crate) fn update(&self, user: Option<&RawValue>) -> Vec<String> {
        let next = EffectiveSettings::compute(&self.specs, user);
        let mut state = self.state();
        let changed = next.changed_since(&state.effective);
        state.effective = next;
        if !changed.is_empty() {
            state.changed.push(changed.clone());
        }
        changed
    }

    /// The `settings-changed` lists to deliver, oldest first.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn take_changed(&self) -> Vec<Vec<String>> {
        std::mem::take(&mut self.state().changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::settings::{SettingEntry, check_settings};

    fn handle(user: Option<serde_json::Value>) -> SettingsHandle {
        let entries: Vec<SettingEntry> = serde_json::from_value(serde_json::json!([
            {"key": "block-on-diff", "title": "Wait for review", "type": "boolean", "default": true},
            {"key": "max-open-diffs", "title": "Open diffs", "type": "integer", "default": 8,
             "min": 1, "max": 64},
        ]))
        .unwrap();
        let user = user.map(|value| serde_json::from_value::<RawValue>(value).unwrap());
        SettingsHandle::new(check_settings(&entries).unwrap(), user.as_ref())
    }

    fn raw(value: serde_json::Value) -> RawValue {
        serde_json::from_value(value).unwrap()
    }

    /// §D.4, §D.7: get/all, an unknown key, and changes per effective value.
    #[test]
    fn get_all_and_changes() {
        let settings = handle(Some(serde_json::json!({"max-open-diffs": 4})));
        assert_eq!(
            settings.get("max-open-diffs").unwrap(),
            SettingValue::Integer(4)
        );
        assert_eq!(
            settings.get("nope").unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
        let keys: Vec<_> = settings
            .all()
            .into_iter()
            .map(|setting| setting.key)
            .collect();
        assert_eq!(keys, ["block-on-diff", "max-open-diffs"]);

        // Invalid: the default (8) reaches the extension, the form gets the error.
        assert_eq!(
            settings.update(Some(&raw(serde_json::json!({"max-open-diffs": 100})))),
            ["max-open-diffs"]
        );
        assert_eq!(
            settings.get("max-open-diffs").unwrap(),
            SettingValue::Integer(8)
        );
        assert!(settings.effective().errors.contains_key("max-open-diffs"));
        // Another invalid value, or an undeclared key: nothing changes, nothing is sent.
        assert!(
            settings
                .update(Some(&raw(serde_json::json!({"max-open-diffs": 0, "x": 1}))))
                .is_empty()
        );
        // The user's explicit default: `is_default` flips, the value does not, nothing is sent.
        assert!(
            settings
                .update(Some(&raw(serde_json::json!({"max-open-diffs": 8}))))
                .is_empty()
        );
        assert_eq!(
            settings.update(Some(&raw(serde_json::json!({"block-on-diff": false})))),
            ["block-on-diff"]
        );
        assert_eq!(
            settings.take_changed(),
            [
                vec!["max-open-diffs".to_string()],
                vec!["block-on-diff".to_string()]
            ]
        );
        assert!(settings.take_changed().is_empty());
    }
}
