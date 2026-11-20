//! Extension settings (`fyx:ui` 0.4, docs/fyx/wit-0.4/design.md §D, plan 6.1): the
//! `[[fyx_ui.settings]]` declarations, the checks of §D.1 (manifest) and §D.2 (the user's
//! values), and the effective values an extension gets.
//!
//! A port of W4's reference checks in `fyx_ext_harness::settings`; `harness_conformance` pins
//! the two together. No I/O and no format crate: [`RawValue`] deserializes from the manifest's
//! TOML and from the user's `settings.json` alike, so the host, the harness and `fyx-ext check`
//! run the same rules.

use std::collections::BTreeMap;

use serde::Deserialize;

/// At most this many settings per extension.
pub const MAX_SETTINGS: usize = 32;
pub const MAX_KEY_BYTES: usize = 64;
pub const MAX_TITLE_BYTES: usize = 80;
pub const MAX_DESCRIPTION_BYTES: usize = 500;
const DEFAULT_MAX_LENGTH: usize = 256;
pub const MAX_MAX_LENGTH: usize = 4096;
const DEFAULT_MAX_ITEMS: usize = 16;
pub const MAX_MAX_ITEMS: usize = 64;
const MIN_CHOICES: usize = 2;
const MAX_CHOICES: usize = 32;
/// Keys that would invite a credential into the plain settings file (§D.1).
const SECRET_WORDS: [&str; 5] = ["token", "secret", "password", "api-key", "apikey"];

/// A float as written, compared bit for bit so that a manifest holding one can be `Eq`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(transparent)]
pub struct Float(pub f64);

impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Float {}

/// A TOML or JSON value as written, before it is checked against a declaration.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RawValue {
    /// JSON `null`; never valid for a setting.
    Null,
    Boolean(bool),
    Integer(i64),
    Float(Float),
    Text(String),
    List(Vec<RawValue>),
    Table(BTreeMap<String, RawValue>),
}

impl RawValue {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(value) => Some(*value as f64),
            Self::Float(value) => Some(value.0),
            _ => None,
        }
    }
}

impl std::fmt::Display for RawValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => formatter.write_str("null"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{}", value.0),
            Self::Text(value) => write!(formatter, "{value:?}"),
            Self::List(_) => formatter.write_str("a list"),
            Self::Table(_) => formatter.write_str("a table"),
        }
    }
}

/// `settings.setting-value`.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    /// The choice's `value`, never its title.
    Choice(String),
    TextList(Vec<String>),
}

/// One `{ value, title }` of a `choice` setting.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingChoice {
    pub value: String,
    pub title: String,
}

/// A setting's `type`, with the bounds that belong to it.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    Boolean,
    Integer { min: Option<i64>, max: Option<i64> },
    Number { min: Option<f64>, max: Option<f64> },
    Text { max_length: usize },
    Choice { choices: Vec<SettingChoice> },
    TextList { max_items: usize, max_length: usize },
}

impl SettingKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Integer { .. } => "integer",
            Self::Number { .. } => "number",
            Self::Text { .. } => "text",
            Self::Choice { .. } => "choice",
            Self::TextList { .. } => "text-list",
        }
    }
}

/// One `[[fyx_ui.settings]]` entry as written; [`check_settings`] turns them into
/// [`SettingSpec`]s.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingEntry {
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub default: Option<RawValue>,
    #[serde(default)]
    pub min: Option<RawValue>,
    #[serde(default)]
    pub max: Option<RawValue>,
    #[serde(default)]
    pub max_length: Option<i64>,
    #[serde(default)]
    pub max_items: Option<i64>,
    #[serde(default)]
    pub choices: Option<Vec<SettingChoice>>,
}

/// One `[[fyx_ui.settings]]` entry, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub kind: SettingKind,
    pub default: SettingValue,
}

/// `settings.setting`: the effective value the extension gets.
#[derive(Clone, Debug, PartialEq)]
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

fn bounded(value: Option<i64>, default: usize, max: usize, what: &str) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(default);
    };
    usize::try_from(value)
        .ok()
        .filter(|value| (1..=max).contains(value))
        .ok_or_else(|| format!("{what} must be 1 to {max}"))
}

fn range<T: std::fmt::Display>(min: &Option<T>, max: &Option<T>) -> String {
    match (min, max) {
        (Some(min), Some(max)) => format!("{min}..={max}"),
        (Some(min), None) => format!("{min}.."),
        (None, Some(max)) => format!("..={max}"),
        (None, None) => "any".to_owned(),
    }
}

/// Every `[[fyx_ui.settings]]` entry, checked (§D.1). The error names the entry.
pub fn check_settings(entries: &[SettingEntry]) -> Result<Vec<SettingSpec>, String> {
    if entries.len() > MAX_SETTINGS {
        return Err(format!(
            "[[fyx_ui.settings]] has {} entries; at most {MAX_SETTINGS}",
            entries.len()
        ));
    }
    let mut specs: Vec<SettingSpec> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let spec = SettingSpec::check(entry)
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

impl SettingSpec {
    /// One entry (§D.1).
    pub fn check(entry: &SettingEntry) -> Result<Self, String> {
        let key = entry.key.as_str();
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
        one_line(&entry.title, MAX_TITLE_BYTES, "title")?;
        if let Some(description) = &entry.description
            && description.len() > MAX_DESCRIPTION_BYTES
        {
            return Err(format!(
                "description must be at most {MAX_DESCRIPTION_BYTES} bytes"
            ));
        }
        let kind_name = entry.kind.as_str();
        let only = |fields: &[&str]| -> Result<(), String> {
            let present = [
                ("min", entry.min.is_some()),
                ("max", entry.max.is_some()),
                ("max_length", entry.max_length.is_some()),
                ("max_items", entry.max_items.is_some()),
                ("choices", entry.choices.is_some()),
            ];
            match present
                .iter()
                .find(|(field, present)| *present && !fields.contains(field))
            {
                Some((field, _)) => Err(format!("{field} does not apply to type {kind_name:?}")),
                None => Ok(()),
            }
        };
        let kind = match kind_name {
            "boolean" => {
                only(&[])?;
                SettingKind::Boolean
            }
            "integer" => {
                only(&["min", "max"])?;
                let whole = |value: &Option<RawValue>, field: &str| match value {
                    None => Ok(None),
                    Some(RawValue::Integer(value)) => Ok(Some(*value)),
                    Some(_) => Err(format!("{field} must be a whole number")),
                };
                let (min, max) = (whole(&entry.min, "min")?, whole(&entry.max, "max")?);
                if let (Some(min), Some(max)) = (min, max)
                    && min > max
                {
                    return Err("min is above max".into());
                }
                SettingKind::Integer { min, max }
            }
            "number" => {
                only(&["min", "max"])?;
                let finite = |value: &Option<RawValue>, field: &str| match value {
                    None => Ok(None),
                    Some(value) => value
                        .as_f64()
                        .filter(|value| value.is_finite())
                        .map(Some)
                        .ok_or_else(|| format!("{field} must be a finite number")),
                };
                let (min, max) = (finite(&entry.min, "min")?, finite(&entry.max, "max")?);
                if let (Some(min), Some(max)) = (min, max)
                    && min > max
                {
                    return Err("min is above max".into());
                }
                SettingKind::Number { min, max }
            }
            "text" => {
                only(&["max_length"])?;
                SettingKind::Text {
                    max_length: bounded(
                        entry.max_length,
                        DEFAULT_MAX_LENGTH,
                        MAX_MAX_LENGTH,
                        "max_length",
                    )?,
                }
            }
            "text-list" => {
                only(&["max_length", "max_items"])?;
                SettingKind::TextList {
                    max_items: bounded(
                        entry.max_items,
                        DEFAULT_MAX_ITEMS,
                        MAX_MAX_ITEMS,
                        "max_items",
                    )?,
                    max_length: bounded(
                        entry.max_length,
                        DEFAULT_MAX_LENGTH,
                        MAX_MAX_LENGTH,
                        "max_length",
                    )?,
                }
            }
            "choice" => {
                only(&["choices"])?;
                let entries = entry
                    .choices
                    .as_ref()
                    .ok_or("choices is required for type \"choice\"")?;
                if !(MIN_CHOICES..=MAX_CHOICES).contains(&entries.len()) {
                    return Err(format!(
                        "choices must have {MIN_CHOICES} to {MAX_CHOICES} entries"
                    ));
                }
                let mut choices: Vec<SettingChoice> = Vec::new();
                for choice in entries {
                    if !is_key(&choice.value) {
                        return Err(format!(
                            "choice value {:?} must be spelled like a key",
                            choice.value
                        ));
                    }
                    one_line(&choice.title, MAX_TITLE_BYTES, "a choice's title")?;
                    if choices.iter().any(|other| other.value == choice.value) {
                        return Err(format!("choice value {:?} is listed twice", choice.value));
                    }
                    choices.push(choice.clone());
                }
                SettingKind::Choice { choices }
            }
            other => {
                return Err(format!(
                    "type {other:?} is not one of boolean, integer, number, text, choice, text-list"
                ));
            }
        };
        let default = entry.default.as_ref().ok_or("default is required")?;
        let mut spec = Self {
            key: key.to_owned(),
            title: entry.title.clone(),
            description: entry.description.clone(),
            kind,
            default: SettingValue::Boolean(false),
        };
        spec.default = spec
            .check_value(default)
            .map_err(|why| format!("default: {why}"))?;
        Ok(spec)
    }

    /// A user's value from the settings file, checked against this declaration (§D.2).
    pub fn check_value(&self, value: &RawValue) -> Result<SettingValue, String> {
        let wrong = || format!("{value} is not a valid {}", self.kind.type_name());
        match &self.kind {
            SettingKind::Boolean => match value {
                RawValue::Boolean(value) => Ok(SettingValue::Boolean(*value)),
                _ => Err(wrong()),
            },
            SettingKind::Integer { min, max } => {
                let RawValue::Integer(value) = value else {
                    return Err(wrong());
                };
                let value = *value;
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
                let RawValue::Text(text) = value else {
                    return Err(wrong());
                };
                text_value(text, *max_length, "the text")?;
                Ok(SettingValue::Text(text.clone()))
            }
            SettingKind::Choice { choices } => {
                let RawValue::Text(chosen) = value else {
                    return Err(wrong());
                };
                if !choices.iter().any(|choice| &choice.value == chosen) {
                    return Err(format!("{chosen:?} is not one of the choices"));
                }
                Ok(SettingValue::Choice(chosen.clone()))
            }
            SettingKind::TextList {
                max_items,
                max_length,
            } => {
                let RawValue::List(items) = value else {
                    return Err(wrong());
                };
                if items.len() > *max_items {
                    return Err(format!("{} items; at most {max_items}", items.len()));
                }
                items
                    .iter()
                    .map(|item| {
                        let RawValue::Text(text) = item else {
                            return Err(wrong());
                        };
                        text_value(text, *max_length, "an item")?;
                        Ok(text.clone())
                    })
                    .collect::<Result<Vec<_>, String>>()
                    .map(SettingValue::TextList)
            }
        }
    }
}

/// What one extension gets from its declarations and the user's values (§D.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EffectiveSettings {
    /// Every declared key, in manifest order.
    pub settings: Vec<Setting>,
    /// Why a user's value was not passed on, by key, for the form.
    pub errors: BTreeMap<String, String>,
    /// Keys under the extension's id that it does not declare, shown greyed out.
    pub unknown: Vec<String>,
}

impl EffectiveSettings {
    /// `user` is the extension's object under `fyx_extensions.<id>` in the **user** settings
    /// file (never project settings, §D.2), or `None` when there is none. Anything but a table
    /// counts as none and is reported under the empty key.
    pub fn compute(specs: &[SettingSpec], user: Option<&RawValue>) -> Self {
        let mut errors = BTreeMap::new();
        let empty = BTreeMap::new();
        let values = match user {
            None => &empty,
            Some(RawValue::Table(values)) => values,
            Some(_) => {
                errors.insert(
                    String::new(),
                    "an extension's settings are a JSON object".to_owned(),
                );
                &empty
            }
        };
        let settings = specs
            .iter()
            .map(|spec| {
                let user = values.get(&spec.key).map(|value| spec.check_value(value));
                match user {
                    Some(Ok(value)) => Setting {
                        key: spec.key.clone(),
                        value,
                        is_default: false,
                    },
                    Some(Err(why)) => {
                        errors.insert(spec.key.clone(), why);
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
                }
            })
            .collect();
        let unknown = values
            .keys()
            .filter(|key| !specs.iter().any(|spec| &spec.key == *key))
            .cloned()
            .collect();
        Self {
            settings,
            errors,
            unknown,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Setting> {
        self.settings.iter().find(|setting| setting.key == key)
    }

    /// The keys whose effective value differs from `before`'s, in manifest order: the
    /// `settings-changed` list (§D.4). `is_default` alone is no change.
    pub fn changed_since(&self, before: &Self) -> Vec<String> {
        self.settings
            .iter()
            .filter(|setting| {
                before
                    .get(&setting.key)
                    .is_none_or(|old| old.value != setting.value)
            })
            .map(|setting| setting.key.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(json: serde_json::Value) -> Vec<SettingEntry> {
        serde_json::from_value(json).unwrap()
    }

    fn specs(json: serde_json::Value) -> Result<Vec<SettingSpec>, String> {
        check_settings(&entries(json))
    }

    fn raw(json: serde_json::Value) -> RawValue {
        serde_json::from_value(json).unwrap()
    }

    fn example() -> Vec<SettingSpec> {
        specs(serde_json::json!([
            {"key": "block-on-diff", "title": "Wait for review", "type": "boolean", "default": true},
            {"key": "max-open-diffs", "title": "Open diffs at most", "type": "integer",
             "default": 8, "min": 1, "max": 64},
            {"key": "ratio", "title": "Ratio", "type": "number", "default": 0.5, "min": 0, "max": 1},
            {"key": "severity", "title": "Report as", "type": "choice", "default": "warning",
             "choices": [{"value": "error", "title": "Error"}, {"value": "warning", "title": "Warning"}]},
            {"key": "extra-args", "title": "Extra arguments", "type": "text-list", "default": [],
             "max_items": 2, "max_length": 8},
            {"key": "label", "title": "Label", "type": "text", "default": "x"},
        ]))
        .unwrap()
    }

    /// §D.7: refused declarations.
    #[test]
    fn declarations_are_checked() {
        assert_eq!(example().len(), 6);
        let one = |entry: serde_json::Value| specs(serde_json::json!([entry]));
        for bad in [
            serde_json::json!({"key": "Bad", "title": "t", "type": "boolean", "default": true}),
            serde_json::json!({"key": "api-token", "title": "t", "type": "text", "default": ""}),
            serde_json::json!({"key": "my-password", "title": "t", "type": "text", "default": ""}),
            serde_json::json!({"key": "a", "title": "", "type": "boolean", "default": true}),
            serde_json::json!({"key": "a", "title": "t\nu", "type": "boolean", "default": true}),
            serde_json::json!({"key": "a", "title": "t", "type": "boolean"}),
            serde_json::json!({"key": "a", "title": "t", "type": "boolean", "default": 1}),
            serde_json::json!({"key": "a", "title": "t", "type": "boolean", "default": true, "min": 1}),
            serde_json::json!({"key": "a", "title": "t", "type": "integer", "default": 0, "min": 1}),
            serde_json::json!({"key": "a", "title": "t", "type": "integer", "default": 1, "min": 2, "max": 1}),
            serde_json::json!({"key": "a", "title": "t", "type": "integer", "default": 1.5}),
            serde_json::json!({"key": "a", "title": "t", "type": "text", "default": "", "max_length": 0}),
            serde_json::json!({"key": "a", "title": "t", "type": "text", "default": "", "max_length": 4097}),
            serde_json::json!({"key": "a", "title": "t", "type": "text-list", "default": [], "max_items": 65}),
            serde_json::json!({"key": "a", "title": "t", "type": "choice", "default": "c",
                               "choices": [{"value": "a", "title": "A"}, {"value": "b", "title": "B"}]}),
            serde_json::json!({"key": "a", "title": "t", "type": "choice", "default": "a",
                               "choices": [{"value": "a", "title": "A"}]}),
            serde_json::json!({"key": "a", "title": "t", "type": "choice", "default": "a",
                               "choices": [{"value": "a", "title": "A"}, {"value": "a", "title": "B"}]}),
            serde_json::json!({"key": "a", "title": "t", "type": "secret", "default": ""}),
            serde_json::json!({"key": "a", "title": "t", "type": "text", "default": "x".repeat(257)}),
        ] {
            assert!(one(bad.clone()).is_err(), "{bad}");
        }
        let twice = serde_json::json!([
            {"key": "a", "title": "t", "type": "boolean", "default": true},
            {"key": "a", "title": "t", "type": "boolean", "default": false},
        ]);
        assert!(specs(twice).unwrap_err().contains("declared twice"));
        let many: Vec<_> = (0..=MAX_SETTINGS)
            .map(|index| {
                serde_json::json!({"key": format!("k{index}"), "title": "t", "type": "boolean",
                                   "default": true})
            })
            .collect();
        assert!(specs(serde_json::Value::Array(many)).is_err());
        let unknown_field: Result<Vec<SettingEntry>, _> =
            serde_json::from_value(serde_json::json!([
                {"key": "a", "title": "t", "type": "boolean", "default": true, "secret": true}
            ]));
        assert!(unknown_field.is_err());
    }

    /// §D.2, §D.7: an invalid value gives the default and an error; fixing it changes that key.
    #[test]
    fn invalid_user_values_give_the_default_and_changes_are_per_key() {
        let specs = example();
        let defaults = EffectiveSettings::compute(&specs, None);
        assert!(defaults.settings.iter().all(|setting| setting.is_default));
        assert!(defaults.errors.is_empty());

        let wrong = EffectiveSettings::compute(
            &specs,
            Some(&raw(serde_json::json!({
                "max-open-diffs": 100,
                "severity": "fatal",
                "extra-args": ["a", "b", "c"],
                "block-on-diff": "no",
                "ratio": 2,
                "label": "a\tb",
                "old-key": 1,
            }))),
        );
        assert!(wrong.settings.iter().all(|setting| setting.is_default));
        assert_eq!(wrong.errors.len(), 6, "{:?}", wrong.errors);
        assert_eq!(wrong.unknown, ["old-key"]);
        assert!(wrong.changed_since(&defaults).is_empty());

        let fixed = EffectiveSettings::compute(
            &specs,
            Some(&raw(
                serde_json::json!({"max-open-diffs": 4, "severity": "warning"}),
            )),
        );
        assert_eq!(fixed.changed_since(&wrong), ["max-open-diffs"]);
        let severity = fixed.get("severity").unwrap();
        assert_eq!(severity.value, SettingValue::Choice("warning".into()));
        assert!(!severity.is_default);
        assert_eq!(
            fixed.get("max-open-diffs").unwrap().value,
            SettingValue::Integer(4)
        );

        let ratio = EffectiveSettings::compute(&specs, Some(&raw(serde_json::json!({"ratio": 1}))));
        assert_eq!(ratio.get("ratio").unwrap().value, SettingValue::Number(1.0));
        let not_object = EffectiveSettings::compute(&specs, Some(&raw(serde_json::json!([1]))));
        assert!(not_object.errors.contains_key(""));
        let null =
            EffectiveSettings::compute(&specs, Some(&raw(serde_json::json!({"label": null}))));
        assert!(null.errors.contains_key("label"));
    }
}
