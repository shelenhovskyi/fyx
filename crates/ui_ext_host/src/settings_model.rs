//! Extension settings (design wit-0.4 §D, plan 6.1), the part the Settings form needs: the
//! declarations, the checks of a user's value (§D.2), and the store the form reads and writes.
//!
//! **Stub.** W2c's `ui_ext::settings` (with `FyxUiTable.settings` and the user settings store
//! under `fyx_extensions.<id>`) is not there yet. The types below mirror the reference in
//! `fyx_ext_harness::settings` (same names and fields), so the form switches to W2c's model by
//! replacing this module's types with re-exports and [`MemorySettingsStore`] with W2c's store,
//! set through [`set_extension_settings_store`].

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui::{App, Global};
use serde_json::{Map, Value};

/// `settings.setting-value`.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    /// The choice's `value`, never its title.
    Choice(String),
    TextList(Vec<String>),
}

impl SettingValue {
    /// As the user settings file spells it.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Boolean(value) => (*value).into(),
            Self::Integer(value) => (*value).into(),
            Self::Number(value) => (*value).into(),
            Self::Text(value) | Self::Choice(value) => value.clone().into(),
            Self::TextList(items) => items.clone().into(),
        }
    }
}

/// One `{ value, title }` of a `choice` setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingChoice {
    pub value: String,
    pub title: String,
}

/// A setting's `type`, with the bounds that belong to it.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingKind {
    Boolean,
    Integer { min: Option<i64>, max: Option<i64> },
    Number { min: Option<f64>, max: Option<f64> },
    Text { max_length: usize },
    Choice { choices: Vec<SettingChoice> },
    TextList { max_items: usize, max_length: usize },
}

/// One `[[fyx_ui.settings]]` entry, checked by the manifest parser.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingSpec {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: SettingKind,
    pub default: SettingValue,
}

fn text_value(text: &str, max_length: usize, what: &str) -> Result<(), String> {
    if text.len() > max_length {
        return Err(format!("{what} is longer than {max_length} bytes"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!(
            "{what} must be one line, without control characters"
        ));
    }
    Ok(())
}

fn range<T: std::fmt::Display>(min: &Option<T>, max: &Option<T>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{min} to {max}"),
        (Some(min), None) => format!("{min} or more"),
        (None, Some(max)) => format!("{max} or less"),
        (None, None) => "any".to_owned(),
    }
}

impl SettingSpec {
    /// A user's value from the settings file, checked against this declaration (§D.2).
    pub fn value_from_json(&self, value: &Value) -> Result<SettingValue, String> {
        let wrong = || format!("{value} is not a valid {}", self.type_name());
        match &self.kind {
            SettingKind::Boolean => value.as_bool().map(SettingValue::Boolean).ok_or_else(wrong),
            SettingKind::Integer { min, max } => {
                let value = value.as_i64().ok_or_else(wrong)?;
                if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
                    return Err(format!("{value} is outside {}", range(min, max)));
                }
                Ok(SettingValue::Integer(value))
            }
            SettingKind::Number { min, max } => {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(wrong)?;
                if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
                    return Err(format!("{value} is outside {}", range(min, max)));
                }
                Ok(SettingValue::Number(value))
            }
            SettingKind::Text { max_length } => {
                let text = value.as_str().ok_or_else(wrong)?;
                text_value(text, *max_length, "the text")?;
                Ok(SettingValue::Text(text.to_owned()))
            }
            SettingKind::Choice { choices } => {
                let chosen = value.as_str().ok_or_else(wrong)?;
                if !choices.iter().any(|choice| choice.value == chosen) {
                    return Err(format!("{chosen:?} is not one of the choices"));
                }
                Ok(SettingValue::Choice(chosen.to_owned()))
            }
            SettingKind::TextList {
                max_items,
                max_length,
            } => {
                let items = value.as_array().ok_or_else(wrong)?;
                if items.len() > *max_items {
                    return Err(format!("{} items; at most {max_items}", items.len()));
                }
                items
                    .iter()
                    .map(|item| {
                        let text = item.as_str().ok_or_else(wrong)?;
                        text_value(text, *max_length, "an item")?;
                        Ok(text.to_owned())
                    })
                    .collect::<Result<Vec<_>, String>>()
                    .map(SettingValue::TextList)
            }
        }
    }

    fn type_name(&self) -> &'static str {
        match self.kind {
            SettingKind::Boolean => "boolean",
            SettingKind::Integer { .. } => "integer",
            SettingKind::Number { .. } => "number",
            SettingKind::Text { .. } => "text",
            SettingKind::Choice { .. } => "choice",
            SettingKind::TextList { .. } => "text-list",
        }
    }

    /// What the user typed into this setting's field, as the settings file would hold it, or why
    /// it cannot be one: `Integer` and `Number` fields parse the text, `Text` takes it as it is.
    pub fn json_from_field(&self, text: &str) -> Result<Value, String> {
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

/// Where the form gets the declarations and the user's values, and where it writes. W2c's store
/// reads `fyx_extensions.<id>` from the user settings file (never project settings) and writes
/// through Zed's settings store.
pub trait ExtensionSettingsStore {
    /// The extension's `[[fyx_ui.settings]]`, in manifest order; empty when it has none.
    fn specs(&self, extension_id: &str, cx: &App) -> Vec<SettingSpec>;
    /// `fyx_extensions.<id>` of the user settings file.
    fn user_values(&self, extension_id: &str, cx: &App) -> Map<String, Value>;
    fn write(&self, extension_id: &str, edit: SettingsEdit, cx: &mut App);
}

struct GlobalSettingsStore(Rc<dyn ExtensionSettingsStore>);

impl Global for GlobalSettingsStore {}

/// Sets the store the Settings form uses. Without one, the form uses [`MemorySettingsStore`],
/// which knows no declarations, so no extension shows a Settings section.
pub fn set_extension_settings_store(store: Rc<dyn ExtensionSettingsStore>, cx: &mut App) {
    cx.set_global(GlobalSettingsStore(store));
}

pub(crate) fn settings_store(cx: &mut App) -> Rc<dyn ExtensionSettingsStore> {
    if let Some(store) = cx.try_global::<GlobalSettingsStore>() {
        return store.0.clone();
    }
    let store: Rc<dyn ExtensionSettingsStore> = Rc::new(MemorySettingsStore::default());
    cx.set_global(GlobalSettingsStore(store.clone()));
    store
}

/// The stand-in store until W2c's lands: declarations and values in memory, for tests.
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
