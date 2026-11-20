//! Extension settings (design wit-0.4 §D, plan 6.1), the part the Settings form needs: the
//! declarations and checks of `ui_ext::settings`, the user's values under `fyx_extensions.<id>`
//! in the **user** settings file (§D.2), and the store the form reads and writes.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{App, Global};
use serde_json::{Map, Value};
pub use ui_ext::settings::{RawValue, SettingChoice, SettingKind, SettingSpec, SettingValue};

/// The JSON side of a declaration: what the settings file holds and what the form's fields give.
pub trait SettingSpecJson {
    /// A user's value from the settings file, checked against this declaration (§D.2) by the
    /// same check the extension's values go through.
    fn value_from_json(&self, value: &Value) -> Result<SettingValue, String>;
    /// What the user typed into this setting's field, as the settings file would hold it, or why
    /// it cannot be one: `Integer` and `Number` fields parse the text, `Text` takes it as it is.
    fn json_from_field(&self, text: &str) -> Result<Value, String>;
}

impl SettingSpecJson for SettingSpec {
    fn value_from_json(&self, value: &Value) -> Result<SettingValue, String> {
        let raw = raw_value(value)
            .ok_or_else(|| format!("{value} is not a valid {}", self.kind.type_name()))?;
        self.check_value(&raw)
    }

    fn json_from_field(&self, text: &str) -> Result<Value, String> {
        let text = text.trim_end_matches(['\r', '\n']);
        match &self.kind {
            SettingKind::Integer { .. } => text
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| "Enter a whole number".to_owned()),
            SettingKind::Number { .. } => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .map(Value::from)
                .ok_or_else(|| "Enter a number".to_owned()),
            _ => Ok(Value::from(text)),
        }
    }
}

/// A settings-file value as the runtime reads it; `None` for one it cannot hold (a number beyond
/// `i64` and `f64`).
pub fn raw_value(value: &Value) -> Option<RawValue> {
    serde_json::from_value(value.clone()).ok()
}

/// How a value shows in a one-line field.
pub fn field_text(value: &SettingValue) -> String {
    match value {
        SettingValue::Boolean(value) => value.to_string(),
        SettingValue::Integer(value) => value.to_string(),
        SettingValue::Number(value) => value.to_string(),
        SettingValue::Text(value) | SettingValue::Choice(value) => value.clone(),
        SettingValue::TextList(items) => items.join(", "),
    }
}

/// One declared setting as the form shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingRow {
    pub spec: SettingSpec,
    /// What the user settings file holds for the key, valid or not.
    pub user: Option<Value>,
    /// What the extension gets: the user's value when valid, else the default.
    pub effective: SettingValue,
    /// Why the user's value is not used.
    pub error: Option<String>,
}

impl SettingRow {
    /// "Reset to default" shows while the user settings file holds a value for the key.
    pub fn can_reset(&self) -> bool {
        self.user.is_some()
    }
}

/// The Settings form of one extension.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SettingsForm {
    /// In manifest order.
    pub rows: Vec<SettingRow>,
    /// Keys under the extension's id that it does not declare (an update dropped them): shown
    /// greyed out, never passed on, kept until reset.
    pub unknown: Vec<(String, Value)>,
}

impl SettingsForm {
    pub fn new(specs: &[SettingSpec], user: &Map<String, Value>) -> Self {
        let rows = specs
            .iter()
            .map(|spec| {
                let user = user.get(&spec.key).cloned();
                let (effective, error) =
                    match user.as_ref().map(|value| spec.value_from_json(value)) {
                        Some(Ok(value)) => (value, None),
                        Some(Err(error)) => (spec.default.clone(), Some(error)),
                        None => (spec.default.clone(), None),
                    };
                SettingRow {
                    spec: spec.clone(),
                    user,
                    effective,
                    error,
                }
            })
            .collect();
        let unknown = user
            .iter()
            .filter(|(key, _)| !specs.iter().any(|spec| &spec.key == *key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Self { rows, unknown }
    }

    pub fn row(&self, key: &str) -> Option<&SettingRow> {
        self.rows.iter().find(|row| row.spec.key == key)
    }

    pub fn has_user_values(&self) -> bool {
        self.rows.iter().any(SettingRow::can_reset) || !self.unknown.is_empty()
    }
}

/// A change the form makes to `fyx_extensions.<id>` in the user settings file.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsEdit {
    Set {
        key: String,
        value: Value,
    },
    /// Removes the key: the default applies again.
    Reset {
        key: String,
    },
    /// Removes every key of the extension, unknown ones too.
    ResetAll,
}

impl SettingsEdit {
    /// Applies the edit to the extension's object of the settings file.
    pub fn apply(&self, values: &mut Map<String, Value>) {
        match self {
            Self::Set { key, value } => {
                values.insert(key.clone(), value.clone());
            }
            Self::Reset { key } => {
                values.remove(key);
            }
            Self::ResetAll => values.clear(),
        }
    }
}

/// Where the form gets the declarations and the user's values, and where it writes.
/// [`UserSettingsFileStore`] is the real one; tests set a [`MemorySettingsStore`].
pub trait ExtensionSettingsStore {
    /// The extension's `[[fyx_ui.settings]]`, in manifest order; empty when it has none.
    fn specs(&self, extension_id: &str, cx: &App) -> Vec<SettingSpec>;
    /// `fyx_extensions.<id>` of the user settings file.
    fn user_values(&self, extension_id: &str, cx: &App) -> Map<String, Value>;
    fn write(&self, extension_id: &str, edit: SettingsEdit, cx: &mut App);
}

struct GlobalSettingsStore(Rc<dyn ExtensionSettingsStore>);

impl Global for GlobalSettingsStore {}

/// Sets the store the Settings form uses. Without one, the form uses [`UserSettingsFileStore`].
pub fn set_extension_settings_store(store: Rc<dyn ExtensionSettingsStore>, cx: &mut App) {
    cx.set_global(GlobalSettingsStore(store));
}

pub(crate) fn settings_store(cx: &mut App) -> Rc<dyn ExtensionSettingsStore> {
    if let Some(store) = cx.try_global::<GlobalSettingsStore>() {
        return store.0.clone();
    }
    let store: Rc<dyn ExtensionSettingsStore> = Rc::new(UserSettingsFileStore);
    cx.set_global(GlobalSettingsStore(store.clone()));
    store
}

/// The user's values of extension `extension_id`: its object under `fyx_extensions` in the
/// **user** settings file, as written (§D.2). Project settings never count.
pub(crate) fn user_object(extension_id: &str, cx: &App) -> Option<Value> {
    cx.try_global::<settings::SettingsStore>()?
        .raw_user_settings()?
        .fyx_extension_settings(extension_id)
        .cloned()
}

/// The declarations of the loaded extensions (`UiExtension::setting_specs`), the user settings
/// file's `fyx_extensions.<id>`, and writes through Zed's settings store.
pub struct UserSettingsFileStore;

impl ExtensionSettingsStore for UserSettingsFileStore {
    fn specs(&self, extension_id: &str, cx: &App) -> Vec<SettingSpec> {
        crate::UiExtensionRegistry::global(cx)
            .and_then(|registry| {
                registry
                    .read(cx)
                    .ui_extension(extension_id)
                    .map(|extension| extension.setting_specs().to_vec())
            })
            .unwrap_or_default()
    }

    fn user_values(&self, extension_id: &str, cx: &App) -> Map<String, Value> {
        match user_object(extension_id, cx) {
            Some(Value::Object(values)) => values,
            _ => Map::new(),
        }
    }

    fn write(&self, extension_id: &str, edit: SettingsEdit, cx: &mut App) {
        let fs = <dyn fs::Fs>::global(cx);
        let extension_id = extension_id.to_owned();
        settings::update_settings_file(fs, cx, move |settings, _| {
            let extensions = settings.fyx_extensions.get_or_insert_default();
            // A value that is not an object (a hand-written mistake) is replaced by the edit.
            let mut values = match extensions.remove(&extension_id) {
                Some(Value::Object(values)) => values,
                _ => Map::new(),
            };
            edit.apply(&mut values);
            if !values.is_empty() {
                extensions.insert(extension_id, Value::Object(values));
            }
        });
    }
}

/// Declarations and values in memory, for tests.
#[derive(Default)]
pub struct MemorySettingsStore {
    specs: RefCell<HashMap<String, Vec<SettingSpec>>>,
    values: RefCell<HashMap<String, Map<String, Value>>>,
}

impl MemorySettingsStore {
    pub fn set_specs(&self, extension_id: &str, specs: Vec<SettingSpec>) {
        self.specs.borrow_mut().insert(extension_id.into(), specs);
    }

    pub fn set_user_values(&self, extension_id: &str, values: Map<String, Value>) {
        self.values.borrow_mut().insert(extension_id.into(), values);
    }
}

impl ExtensionSettingsStore for MemorySettingsStore {
    fn specs(&self, extension_id: &str, _: &App) -> Vec<SettingSpec> {
        self.specs
            .borrow()
            .get(extension_id)
            .cloned()
            .unwrap_or_default()
    }

    fn user_values(&self, extension_id: &str, _: &App) -> Map<String, Value> {
        self.values
            .borrow()
            .get(extension_id)
            .cloned()
            .unwrap_or_default()
    }

    fn write(&self, extension_id: &str, edit: SettingsEdit, _: &mut App) {
        let mut values = self.values.borrow_mut();
        edit.apply(values.entry(extension_id.into()).or_default());
    }
}
