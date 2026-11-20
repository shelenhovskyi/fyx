//! 0.4 extension settings, runtime side (design wit-0.4 §D.2, §D.4, plan 6.1): the checked
//! declarations, the effective values the `settings` import answers from, and the keys a store
//! change sends as `settings-changed`.
//!
//! The host reads the extension's object under `fyx_extensions.<id>` from the **user** settings
//! file only (never project settings, §D.2) and hands it to [`SettingsHandle::update`] on every
//! settings-file change; this module never reads a file. The values are fixed for one export
//! call: the import reads a snapshot, and a change made during a call is delivered after it.

use std::sync::{Arc, Mutex, PoisonError};

use futures::{StreamExt as _, channel::mpsc};
use ui_ext::settings::{EffectiveSettings, RawValue, Setting, SettingSpec, SettingValue};
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::network::{IoEvent, Running};

/// One extension instance's settings.
#[derive(Debug)]
pub(crate) struct SettingsHandle {
    specs: Vec<SettingSpec>,
    effective: Mutex<EffectiveSettings>,
    /// `settings-changed` lists, oldest first, until [`SettingsHandle::forward`] delivers them.
    sender: mpsc::UnboundedSender<Vec<String>>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Vec<String>>>>,
}

impl SettingsHandle {
    /// `specs` are the manifest's checked `[[fyx_ui.settings]]` (none below 0.4), `user` the
    /// user's object for this extension at load.
    pub(crate) fn new(specs: Vec<SettingSpec>, user: Option<&RawValue>) -> Self {
        let effective = EffectiveSettings::compute(&specs, user);
        let (sender, receiver) = mpsc::unbounded();
        Self {
            specs,
            effective: Mutex::new(effective),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    fn current(&self) -> std::sync::MutexGuard<'_, EffectiveSettings> {
        self.effective
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Delivers the `settings-changed` lists to the running instance through `handle-io`, once
    /// it runs on tokio (0.4 only). Lists queued before it ran are delivered first.
    pub(crate) fn forward(&self, running: &Arc<Running>) {
        let Some(mut receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let running = Arc::downgrade(running);
        tokio::spawn(async move {
            while let Some(keys) = receiver.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::SettingsChanged(keys)).await;
            }
        });
    }

    pub(crate) fn specs(&self) -> &[SettingSpec] {
        &self.specs
    }

    /// What the extension gets now, with the form's errors and unknown keys.
    pub(crate) fn effective(&self) -> EffectiveSettings {
        self.current().clone()
    }

    /// `settings.get`: `invalid-argument` for a key the manifest does not declare.
    pub(crate) fn get(&self, key: &str) -> Result<SettingValue, HostError> {
        self.current()
            .get(key)
            .map(|setting| setting.value.clone())
            .ok_or_else(|| HostError {
                code: HostErrorCode::InvalidArgument,
                message: format!("no setting {key:?} is declared in [[fyx_ui.settings]]"),
            })
    }

    /// `settings.all`: every declared key, in manifest order.
    pub(crate) fn all(&self) -> Vec<Setting> {
        self.current().settings.clone()
    }

    /// The user's object changed: recomputes and returns the keys whose effective value
    /// changed, in manifest order, also sent as `settings-changed`. Nothing is sent when no
    /// effective value changed (§D.4).
    pub(crate) fn update(&self, user: Option<&RawValue>) -> Vec<String> {
        let next = EffectiveSettings::compute(&self.specs, user);
        let mut current = self.current();
        let changed = next.changed_since(&current);
        *current = next;
        if !changed.is_empty() {
            // The receiver is gone only when the instance is: nothing to tell then.
            self.sender.unbounded_send(changed.clone()).ok();
        }
        changed
    }

    /// The `settings-changed` lists not yet forwarded, oldest first (tests).
    #[cfg(test)]
    fn take_changed(&self) -> Vec<Vec<String>> {
        let mut taken = Vec::new();
        if let Some(receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            while let Ok(keys) = receiver.try_recv() {
                taken.push(keys);
            }
        }
        taken
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
