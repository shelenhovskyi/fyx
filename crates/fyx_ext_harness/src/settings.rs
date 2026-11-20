//! Extension settings (`fyx:ui` 0.4, design.md §D, plan 6.1): the checks of
//! `[[fyx_ui.settings]]` and of the user's values, and a fake of the host's store.
//!
//! These checks are the reference: W2c ports them into `ui_ext::settings` for the host, and
//! `harness_conformance` pins the two together. `fyx-ext check` runs [`SettingSpec::parse_all`]
//! on the manifest.
//!
//! The staged WIT has no `settings` interface yet (the diff for W0 is in design.md), so the
//! component cannot call the store: tests set the user's values with
//! [`Harness::set_user_settings`](crate::Harness::set_user_settings) and read what the
//! extension would get.

use std::collections::{BTreeMap, VecDeque};

/// At most this many settings per extension.
pub const MAX_SETTINGS: usize = 32;
const MAX_KEY_BYTES: usize = 64;
const MAX_TITLE_BYTES: usize = 80;
const MAX_DESCRIPTION_BYTES: usize = 500;
const DEFAULT_MAX_LENGTH: usize = 256;
const MAX_MAX_LENGTH: usize = 4096;
const DEFAULT_MAX_ITEMS: usize = 16;
const MAX_MAX_ITEMS: usize = 64;
const MIN_CHOICES: usize = 2;
const MAX_CHOICES: usize = 32;
/// Keys that would invite a credential into the plain settings file (§D.1).
const SECRET_WORDS: [&str; 5] = ["token", "secret", "password", "api-key", "apikey"];

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
    pub fn to_json(&self) -> serde_json::Value {
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

/// One `[[fyx_ui.settings]]` entry, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingSpec {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: SettingKind,
    pub default: SettingValue,
}

/// `settings.setting`: the effective value the extension gets.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub key: String,
    pub value: SettingValue,
    /// False when the user set a valid value (which may equal the default).
    pub is_default: bool,
}

fn is_key(text: &str) -> bool {
    text.len() <= MAX_KEY_BYTES
        && text.starts_with(|first: char| first.is_ascii_lowercase())
        && text
            .chars()
            .all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-')
}

fn one_line(text: &str, max: usize, what: &str) -> Result<(), String> {
    if text.is_empty() || text.len() > max {
        return Err(format!("{what} must be 1 to {max} bytes"));
    }
    if text.chars().any(char::is_control) {
        return Err(format!(
            "{what} must be one line, without control characters"
        ));
    }
    Ok(())
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

/// A whole number of TOML or JSON, within `min..=max` when given.
fn bounded(value: usize, min: usize, max: usize, what: &str) -> Result<usize, String> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{what} must be {min} to {max}"))
    }
}

impl SettingSpec {
    /// Every `[[fyx_ui.settings]]` entry, checked as the host checks them (§D.1). The error
    /// names the entry.
    pub fn parse_all(entries: &toml::Value) -> Result<Vec<Self>, String> {
        let entries = entries
            .as_array()
            .ok_or("[[fyx_ui.settings]] must be an array of tables")?;
        if entries.len() > MAX_SETTINGS {
            return Err(format!(
                "[[fyx_ui.settings]] has {} entries; at most {MAX_SETTINGS}",
                entries.len()
            ));
        }
        let mut specs: Vec<Self> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let spec = Self::parse(entry)
                .map_err(|why| format!("[[fyx_ui.settings]] entry {}: {why}", index + 1))?;
            if specs.iter().any(|other| other.key == spec.key) {
                return Err(format!(
                    "[[fyx_ui.settings]]: the key {:?} is declared twice",
                    spec.key
                ));
            }
            specs.push(spec);
        }
        Ok(specs)
    }

    /// One entry.
    pub fn parse(entry: &toml::Value) -> Result<Self, String> {
        const FIELDS: [&str; 10] = [
            "key",
            "title",
            "description",
            "type",
            "default",
            "min",
            "max",
            "max_length",
            "max_items",
            "choices",
        ];
        let table = entry.as_table().ok_or("an entry must be a table")?;
        if let Some(unknown) = table.keys().find(|field| !FIELDS.contains(&field.as_str())) {
            return Err(format!("unknown field {unknown:?}"));
        }
        let text = |field: &str| -> Result<Option<&str>, String> {
            match table.get(field) {
                None => Ok(None),
                Some(value) => value
                    .as_str()
                    .map(Some)
                    .ok_or_else(|| format!("{field} must be a string")),
            }
        };
        let size = |field: &str| -> Result<Option<usize>, String> {
            match table.get(field) {
                None => Ok(None),
                Some(value) => value
                    .as_integer()
                    .and_then(|value| usize::try_from(value).ok())
                    .map(Some)
                    .ok_or_else(|| format!("{field} must be a whole number")),
            }
        };
        let key = text("key")?.ok_or("key is required")?;
        if !is_key(key) {
            return Err(format!(
                "key {key:?} must be 1 to {MAX_KEY_BYTES} bytes of a-z, 0-9 and -, starting with a letter"
            ));
        }
        if let Some(word) = SECRET_WORDS.iter().find(|word| key.contains(*word)) {
            return Err(format!(
                "key {key:?} contains {word:?}: settings are plain text in settings.json and never hold credentials"
            ));
        }
        let title = text("title")?.ok_or("title is required")?;
        one_line(title, MAX_TITLE_BYTES, "title")?;
        let description = text("description")?;
        if let Some(description) = description
            && description.len() > MAX_DESCRIPTION_BYTES
        {
            return Err(format!(
                "description must be at most {MAX_DESCRIPTION_BYTES} bytes"
            ));
        }
        let kind_name = text("type")?.ok_or("type is required")?;
        let only = |fields: &[&str]| -> Result<(), String> {
            for field in ["min", "max", "max_length", "max_items", "choices"] {
                if table.contains_key(field) && !fields.contains(&field) {
                    return Err(format!("{field} does not apply to type {kind_name:?}"));
                }
            }
            Ok(())
        };
        let kind = match kind_name {
            "boolean" => {
                only(&[])?;
                SettingKind::Boolean
            }
            "integer" => {
                only(&["min", "max"])?;
                let whole = |field: &str| -> Result<Option<i64>, String> {
                    match table.get(field) {
                        None => Ok(None),
                        Some(value) => value
                            .as_integer()
                            .map(Some)
                            .ok_or_else(|| format!("{field} must be a whole number")),
                    }
                };
                let (min, max) = (whole("min")?, whole("max")?);
                if let (Some(min), Some(max)) = (min, max)
                    && min > max
                {
                    return Err("min is above max".into());
                }
                SettingKind::Integer { min, max }
            }
            "number" => {
                only(&["min", "max"])?;
                let finite = |field: &str| -> Result<Option<f64>, String> {
                    match table.get(field) {
                        None => Ok(None),
                        Some(value) => value
                            .as_float()
                            .or_else(|| value.as_integer().map(|value| value as f64))
                            .filter(|value| value.is_finite())
                            .map(Some)
                            .ok_or_else(|| format!("{field} must be a finite number")),
                    }
                };
                let (min, max) = (finite("min")?, finite("max")?);
                if let (Some(min), Some(max)) = (min, max)
                    && min > max
                {
                    return Err("min is above max".into());
                }
                SettingKind::Number { min, max }
            }
            "text" => {
                only(&["max_length"])?;
                let max_length = size("max_length")?.unwrap_or(DEFAULT_MAX_LENGTH);
                SettingKind::Text {
                    max_length: bounded(max_length, 1, MAX_MAX_LENGTH, "max_length")?,
                }
            }
            "text-list" => {
                only(&["max_length", "max_items"])?;
                let max_length = size("max_length")?.unwrap_or(DEFAULT_MAX_LENGTH);
                let max_items = size("max_items")?.unwrap_or(DEFAULT_MAX_ITEMS);
                SettingKind::TextList {
                    max_items: bounded(max_items, 1, MAX_MAX_ITEMS, "max_items")?,
                    max_length: bounded(max_length, 1, MAX_MAX_LENGTH, "max_length")?,
                }
            }
            "choice" => {
                only(&["choices"])?;
                let entries = table
                    .get("choices")
                    .and_then(toml::Value::as_array)
                    .ok_or("choices is required for type \"choice\"")?;
                if !(MIN_CHOICES..=MAX_CHOICES).contains(&entries.len()) {
                    return Err(format!(
                        "choices must have {MIN_CHOICES} to {MAX_CHOICES} entries"
                    ));
                }
                let mut choices: Vec<SettingChoice> = Vec::new();
                for choice in entries {
                    let field = |name: &str| {
                        choice
                            .get(name)
                            .and_then(toml::Value::as_str)
                            .ok_or_else(|| format!("a choice needs a string {name}"))
                    };
                    let (value, title) = (field("value")?, field("title")?);
                    if !is_key(value) {
                        return Err(format!("choice value {value:?} must be spelled like a key"));
                    }
                    one_line(title, MAX_TITLE_BYTES, "a choice's title")?;
                    if choices.iter().any(|other| other.value == value) {
                        return Err(format!("choice value {value:?} is listed twice"));
                    }
                    choices.push(SettingChoice {
                        value: value.to_owned(),
                        title: title.to_owned(),
                    });
                }
                SettingKind::Choice { choices }
            }
            other => {
                return Err(format!(
                    "type {other:?} is not one of boolean, integer, number, text, choice, text-list"
                ));
            }
        };
        let default = table.get("default").ok_or("default is required")?;
        let default = serde_json::to_value(default).map_err(|error| format!("default: {error}"))?;
        let mut spec = Self {
            key: key.to_owned(),
            title: title.to_owned(),
            description: description.map(str::to_owned),
            kind,
            default: SettingValue::Boolean(false),
        };
        spec.default = spec
            .value_from_json(&default)
            .map_err(|why| format!("default: {why}"))?;
        Ok(spec)
    }

    /// A user's value from the settings file, checked against this declaration (§D.2).
    pub fn value_from_json(&self, value: &serde_json::Value) -> Result<SettingValue, String> {
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
}

fn range<T: std::fmt::Display>(min: &Option<T>, max: &Option<T>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{min}..={max}"),
        (Some(min), None) => format!("{min}.."),
        (None, Some(max)) => format!("..={max}"),
        (None, None) => "any".to_owned(),
    }
}

/// The host's store for one extension: the declarations, the user's values under
/// `fyx_extensions.<id>`, and what the extension gets.
#[derive(Debug, Default)]
pub(crate) struct FakeSettings {
    specs: Vec<SettingSpec>,
    user: serde_json::Map<String, serde_json::Value>,
    effective: Vec<Setting>,
    /// Why a user's value was not passed on, by key, as the form shows it.
    errors: BTreeMap<String, String>,
    /// The `settings-changed` events the host would send, oldest first.
    pub(crate) changed: VecDeque<Vec<String>>,
}

impl FakeSettings {
    pub(crate) fn set_specs(&mut self, specs: Vec<SettingSpec>) {
        self.specs = specs;
        self.recompute();
    }

    pub(crate) fn specs(&self) -> &[SettingSpec] {
        &self.specs
    }

    pub(crate) fn effective(&self) -> &[Setting] {
        &self.effective
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Setting> {
        self.effective.iter().find(|setting| setting.key == key)
    }

    pub(crate) fn error(&self, key: &str) -> Option<&str> {
        self.errors.get(key).map(String::as_str)
    }

    /// Replaces the user's values with `values` (the extension's object in the settings file):
    /// the keys whose effective value changed, in manifest order, which are also queued as one
    /// `settings-changed` when there are any.
    pub(crate) fn set_user(&mut self, values: serde_json::Value) -> Result<Vec<String>, String> {
        let serde_json::Value::Object(values) = values else {
            return Err("an extension's settings are a JSON object".into());
        };
        self.user = values;
        let before = std::mem::take(&mut self.effective);
        self.recompute();
        let changed: Vec<String> = self
            .effective
            .iter()
            .filter(|setting| {
                before
                    .iter()
                    .find(|old| old.key == setting.key)
                    .is_none_or(|old| old.value != setting.value)
            })
            .map(|setting| setting.key.clone())
            .collect();
        if !changed.is_empty() {
            self.changed.push_back(changed.clone());
        }
        Ok(changed)
    }

    fn recompute(&mut self) {
        self.errors.clear();
        let mut effective = Vec::with_capacity(self.specs.len());
        for spec in &self.specs {
            let user = self
                .user
                .get(&spec.key)
                .map(|value| spec.value_from_json(value));
            let setting = match user {
                Some(Ok(value)) => Setting {
                    key: spec.key.clone(),
                    value,
                    is_default: false,
                },
                Some(Err(why)) => {
                    self.errors.insert(spec.key.clone(), why);
                    Setting {
                        key: spec.key.clone(),
                        value: spec.default.clone(),
                        is_default: true,
                    }
                }
                None => Setting {
                    key: spec.key.clone(),
                    value: spec.default.clone(),
                    is_default: true,
                },
            };
            effective.push(setting);
        }
        self.effective = effective;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(manifest: &str) -> Result<Vec<SettingSpec>, String> {
        let table: toml::Table = manifest.parse().unwrap();
        SettingSpec::parse_all(&table["settings"])
    }

    const MANIFEST: &str = r#"
[[settings]]
key = "block-on-diff"
title = "Wait for review"
type = "boolean"
default = true

[[settings]]
key = "max-open-diffs"
title = "Open diffs at most"
type = "integer"
default = 8
min = 1
max = 64

[[settings]]
key = "ratio"
title = "Ratio"
type = "number"
default = 0.5
max = 1

[[settings]]
key = "severity"
title = "Report as"
type = "choice"
default = "warning"
choices = [{ value = "error", title = "Error" }, { value = "warning", title = "Warning" }]

[[settings]]
key = "extra-args"
title = "Extra arguments"
type = "text-list"
default = []
max_items = 2
"#;

    #[test]
    fn declarations_are_checked() {
        let parsed = specs(MANIFEST).unwrap();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[1].default, SettingValue::Integer(8));
        assert_eq!(parsed[2].default, SettingValue::Number(0.5));
        assert_eq!(
            parsed[4].kind,
            SettingKind::TextList {
                max_items: 2,
                max_length: DEFAULT_MAX_LENGTH
            }
        );
        let refused = |entry: &str, why: &str| {
            let error = specs(&format!("[[settings]]\n{entry}")).unwrap_err();
            assert!(error.contains(why), "{entry}: {error}");
        };
        refused(
            "key = \"a\"\ntitle = \"A\"\ntype = \"integer\"\ndefault = 9\nmax = 4",
            "outside",
        );
        refused(
            "key = \"api-token\"\ntitle = \"A\"\ntype = \"text\"\ndefault = \"\"",
            "credentials",
        );
        refused(
            "key = \"Big\"\ntitle = \"A\"\ntype = \"boolean\"\ndefault = true",
            "a-z",
        );
        refused(
            "key = \"a\"\ntitle = \"A\"\ntype = \"boolean\"\ndefault = 1",
            "not a valid boolean",
        );
        refused(
            "key = \"a\"\ntitle = \"A\"\ntype = \"boolean\"\ndefault = true\nmin = 1",
            "does not apply",
        );
        refused(
            "key = \"a\"\ntitle = \"A\"\ntype = \"choice\"\ndefault = \"c\"\nchoices = [{ value = \"b\", title = \"B\" }, { value = \"d\", title = \"D\" }]",
            "not one of the choices",
        );
        refused(
            "key = \"a\"\ntitle = \"A\"\ntype = \"path\"\ndefault = \"\"",
            "not one of",
        );
        refused(
            "key = \"a\"\ntitle = \"A\"\ntype = \"text\"\ndefault = \"\"\nreadonly = true",
            "unknown field",
        );
        let twice =
            "[[settings]]\nkey = \"a\"\ntitle = \"A\"\ntype = \"boolean\"\ndefault = true\n";
        assert!(specs(&twice.repeat(2)).unwrap_err().contains("twice"));
        let many: String = (0..=MAX_SETTINGS)
            .map(|index| {
                format!("[[settings]]\nkey = \"k{index}\"\ntitle = \"K\"\ntype = \"boolean\"\ndefault = true\n")
            })
            .collect();
        assert!(specs(&many).unwrap_err().contains("at most 32"));
    }

    #[test]
    fn invalid_user_values_give_the_default_and_changes_are_per_key() {
        let mut store = FakeSettings::default();
        store.set_specs(specs(MANIFEST).unwrap());
        assert!(store.effective().iter().all(|setting| setting.is_default));

        let changed = store
            .set_user(serde_json::json!({
                "block-on-diff": false,
                "max-open-diffs": 100,
                "severity": "error",
                "unknown": 1,
            }))
            .unwrap();
        assert_eq!(changed, ["block-on-diff", "severity"]);
        assert_eq!(
            store.get("max-open-diffs").unwrap().value,
            SettingValue::Integer(8)
        );
        assert!(
            store
                .error("max-open-diffs")
                .unwrap()
                .contains("outside 1..=64")
        );
        assert!(!store.get("block-on-diff").unwrap().is_default);

        // Another invalid value for the same key changes nothing the extension sees.
        let changed = store
            .set_user(serde_json::json!({
                "block-on-diff": false,
                "max-open-diffs": "eight",
                "severity": "error",
            }))
            .unwrap();
        assert!(changed.is_empty());
        // A valid value equal to the default is the user's, but not a change.
        let changed = store
            .set_user(serde_json::json!({
                "block-on-diff": false,
                "max-open-diffs": 8,
                "severity": "error",
                "extra-args": ["--a", "--b", "--c"],
            }))
            .unwrap();
        assert!(changed.is_empty());
        assert!(!store.get("max-open-diffs").unwrap().is_default);
        assert!(store.error("extra-args").unwrap().contains("at most 2"));
        assert_eq!(store.changed.len(), 1);
        assert!(store.set_user(serde_json::json!([])).is_err());
        assert_eq!(
            SettingValue::TextList(vec!["a".into()]).to_json(),
            serde_json::json!(["a"])
        );
    }
}
