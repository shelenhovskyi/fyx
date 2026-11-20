//! `fyx:ui` 0.4, designed: the extension's own settings (design.md §D, plan 6.1), with the
//! `fyx-ui-0-4` feature. No capability.
//!
//! The manifest declares them in `[[fyx_ui.settings]]` (key, title, type, default and bounds);
//! the host draws the form, stores the user's values and checks them, so [`get`] and [`all`]
//! only ever return a valid value: the user's, or the default. When an effective value changes,
//! `io-event::settings-changed(keys)` arrives in `handle_io`; read the keys again then. An
//! extension never writes a value; [`open`] shows the user its Settings tab.
//!
//! Exported as 0.2 or 0.1, every call fails with `unavailable`. Off WASM the calls answer from
//! [`set_settings`], for an extension's unit tests, and [`open`] records a
//! [`HostEffect::Settings`](crate::HostEffect::Settings).

use crate::HostError;
#[cfg(not(target_arch = "wasm32"))]
use crate::HostErrorCode;

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_settings_get(key: &str) -> Result<SettingValue, HostError>;
    safe fn __fyx_ui_settings_all() -> Result<Vec<Setting>, HostError>;
    safe fn __fyx_ui_settings_open(key: Option<&str>) -> Result<(), HostError>;
}

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
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    /// A `number`, or an `integer` as a number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Integer(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_choice(&self) -> Option<&str> {
        match self {
            Self::Choice(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_text_list(&self) -> Option<&[String]> {
        match self {
            Self::TextList(items) => Some(items),
            _ => None,
        }
    }
}

/// `settings.setting`: one declared key's effective value.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub key: String,
    pub value: SettingValue,
    /// False when the user set a valid value (which may equal the default).
    pub is_default: bool,
}

impl Setting {
    pub fn new(key: impl Into<String>, value: SettingValue) -> Self {
        Self {
            key: key.into(),
            value,
            is_default: true,
        }
    }

    /// The user's value rather than the default.
    pub fn set_by_user(mut self) -> Self {
        self.is_default = false;
        self
    }
}

/// What [`open`] did, recorded off WASM in [`HostEffect::Settings`](crate::HostEffect::Settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsEffect {
    Open(Option<String>),
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static SETTINGS: std::cell::RefCell<Vec<Setting>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Off WASM, the settings [`get`] and [`all`] answer from on this thread, in manifest order.
/// For unit tests of an extension.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_settings(settings: &[Setting]) {
    SETTINGS.with_borrow_mut(|known| *known = settings.to_vec());
}

/// The effective value of `key`, which the manifest must declare (`invalid-argument` if not).
pub fn get(key: &str) -> Result<SettingValue, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_settings_get(key);
    #[cfg(not(target_arch = "wasm32"))]
    SETTINGS.with_borrow(|settings| {
        settings
            .iter()
            .find(|setting| setting.key == key)
            .map(|setting| setting.value.clone())
            .ok_or_else(|| HostError {
                code: HostErrorCode::InvalidArgument,
                message: format!("{key:?} is not in [[fyx_ui.settings]]"),
            })
    })
}

/// Every declared key, in manifest order.
pub fn all() -> Result<Vec<Setting>, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_settings_all();
    #[cfg(not(target_arch = "wasm32"))]
    Ok(SETTINGS.with_borrow(Clone::clone))
}

/// Shows the extension's Settings tab, scrolled to `key` when given. Only while the user acts on
/// one of the extension's surfaces (in `handle_event`); otherwise `not-permitted`.
pub fn open(key: Option<&str>) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_settings_open(key);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Settings(SettingsEffect::Open(
        key.map(str::to_owned),
    )))
}

/// What `register_extension!`'s settings shims call, per exported world.
#[doc(hidden)]
pub mod glue {
    /// The 0.4 import.
    pub mod newest {
        use super::super::{Setting, SettingValue};
        use crate::HostError;
        use crate::bindings::fyx::ui::settings as wit;

        fn value(value: wit::SettingValue) -> SettingValue {
            match value {
                wit::SettingValue::Boolean(value) => SettingValue::Boolean(value),
                wit::SettingValue::Integer(value) => SettingValue::Integer(value),
                wit::SettingValue::Number(value) => SettingValue::Number(value),
                wit::SettingValue::Text(value) => SettingValue::Text(value),
                wit::SettingValue::Choice(value) => SettingValue::Choice(value),
                wit::SettingValue::TextList(items) => SettingValue::TextList(items),
            }
        }

        pub fn get(key: &str) -> Result<SettingValue, HostError> {
            wit::get(key).map(value)
        }

        pub fn all() -> Result<Vec<Setting>, HostError> {
            Ok(wit::all()
                .into_iter()
                .map(|setting| Setting {
                    key: setting.key,
                    value: value(setting.value),
                    is_default: setting.is_default,
                })
                .collect())
        }

        pub fn open(key: Option<&str>) -> Result<(), HostError> {
            wit::open(key)
        }
    }

    /// Worlds without settings: every call is `unavailable`.
    macro_rules! older {
        ($world:ident, $version:literal) => {
            pub mod $world {
                use super::super::{Setting, SettingValue};
                use crate::{HostError, HostErrorCode};

                fn unavailable(what: &str) -> HostError {
                    HostError {
                        code: HostErrorCode::Unavailable,
                        message: format!(
                            "settings::{what} needs fyx:ui 0.4; this extension is exported as {}",
                            $version
                        ),
                    }
                }

                pub fn get(_key: &str) -> Result<SettingValue, HostError> {
                    Err(unavailable("get"))
                }

                pub fn all() -> Result<Vec<Setting>, HostError> {
                    Err(unavailable("all"))
                }

                pub fn open(_key: Option<&str>) -> Result<(), HostError> {
                    Err(unavailable("open"))
                }
            }
        };
    }

    older!(v0_2, "0.2");
    older!(v0_1, "0.1");
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{HostEffect, take_host_effects};

    #[test]
    fn settings_answer_off_wasm() {
        set_settings(&[
            Setting::new("block-on-diff", SettingValue::Boolean(false)).set_by_user(),
            Setting::new("max-open-diffs", SettingValue::Integer(8)),
            Setting::new("severity", SettingValue::Choice("warning".into())),
        ]);
        assert_eq!(get("block-on-diff").unwrap().as_bool(), Some(false));
        assert_eq!(get("max-open-diffs").unwrap().as_number(), Some(8.0));
        assert_eq!(get("severity").unwrap().as_choice(), Some("warning"));
        assert_eq!(get("severity").unwrap().as_text(), None);
        assert_eq!(
            get("missing").unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
        let keys: Vec<(String, bool)> = all()
            .unwrap()
            .into_iter()
            .map(|setting| (setting.key, setting.is_default))
            .collect();
        assert_eq!(
            keys,
            [
                ("block-on-diff".to_owned(), false),
                ("max-open-diffs".to_owned(), true),
                ("severity".to_owned(), true),
            ]
        );
        take_host_effects();
        open(Some("severity")).unwrap();
        assert_eq!(
            take_host_effects(),
            [HostEffect::Settings(SettingsEffect::Open(Some(
                "severity".into()
            )))]
        );
        set_settings(&[]);
    }

    #[test]
    fn older_worlds_have_no_settings() {
        assert_eq!(
            glue::v0_2::get("x").unwrap_err().code,
            HostErrorCode::Unavailable
        );
        assert_eq!(
            glue::v0_1::all().unwrap_err().code,
            HostErrorCode::Unavailable
        );
        assert_eq!(
            glue::v0_2::open(None).unwrap_err().code,
            HostErrorCode::Unavailable
        );
    }
}
