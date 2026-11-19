//! Notifications and progress of `fyx:ui@0.2.0` (design.md, section M): the types of the
//! `notifications` interface, their events, the limits the host and the renderer share, and the
//! rules checked when `notify`, `progress-start` and `progress-update` are called.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::api_version::ApiVersion;

/// The manifest capability for notifications and progress (tier T0).
pub const CAPABILITY_NOTIFICATIONS: &str = "ui:notifications";

/// Accepted `notify` calls per extension per [`RATE_WINDOW`].
pub const MAX_NOTIFY_PER_WINDOW: usize = 5;
/// Notifications of one extension open at once.
pub const MAX_OPEN_NOTIFICATIONS: usize = 3;
/// `notify` and `dismiss-notification` effects per export call.
pub const MAX_NOTIFICATION_EFFECTS_PER_CALL: usize = 4;
/// Progresses of one extension running at once.
pub const MAX_RUNNING_PROGRESSES: usize = 4;
/// `progress-start` calls per extension per [`RATE_WINDOW`].
pub const MAX_PROGRESS_STARTS_PER_WINDOW: usize = 30;
/// `progress-update` calls per export call, as for pushes.
pub const MAX_PROGRESS_UPDATES_PER_CALL: usize = 64;
/// The rolling window of the rate caps.
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Renderer rules (design.md M.1, M.2): how long an auto-dismissed notification stays, how long
/// its buttons ignore clicks after it appears, how often a progress is redrawn, and how many
/// progresses one panel stacks.
pub const AUTO_DISMISS_INFO: Duration = Duration::from_secs(6);
pub const AUTO_DISMISS_WARNING: Duration = Duration::from_secs(10);
/// The least a renderer must apply. Fyx's host applies at least 1 s to every card, notification
/// buttons included (integration-6 Q10), so a guest must not count on a click landing sooner.
pub const BUTTON_CLICK_GUARD: Duration = Duration::from_millis(750);
pub const MAX_PROGRESS_REDRAWS_PER_SECOND: u32 = 10;
pub const MAX_PROGRESSES_PER_PANEL: usize = 4;

pub const MAX_TITLE_BYTES: usize = 80;
pub const MAX_BODY_BYTES: usize = 500;
pub const MAX_BODY_LINES: usize = 4;
pub const MAX_ACTIONS: usize = 2;
pub const MAX_ACTION_LABEL_BYTES: usize = 24;
pub const MAX_PROGRESS_MESSAGE_BYTES: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationLevel {
    /// How long an `auto-dismiss` notification of this level stays.
    pub fn auto_dismiss_after(self) -> Duration {
        match self {
            NotificationLevel::Info | NotificationLevel::Success => AUTO_DISMISS_INFO,
            NotificationLevel::Warning | NotificationLevel::Error => AUTO_DISMISS_WARNING,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Notification {
    /// A live surface or decoration id of the extension; it gets the events.
    pub owner: String,
    pub level: NotificationLevel,
    pub title: String,
    pub body: Option<String>,
    pub actions: Vec<NotificationAction>,
    pub auto_dismiss: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressLocation {
    StatusBar,
    /// Under the owner panel's header; the owner must be a panel.
    Panel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProgressOptions {
    pub owner: String,
    pub title: String,
    pub location: ProgressLocation,
    pub cancellable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressValue {
    Indeterminate,
    /// 0 to 100.
    Percent(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationCloseReason {
    User,
    Expired,
    WindowClosed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct NotificationActionEvent {
    pub notification: u64,
    pub action: String,
}

/// A rule of section M that a call broke; the host returns it as `invalid-argument`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct NotificationError(pub String);

/// Characters that never belong in a notification or progress text (the registry's rule for
/// `description`): controls (Cc, a newline only where lines are allowed), format characters (Cf:
/// bidi overrides and isolates, zero-width characters, tags, the BOM) and U+2028/U+2029.
fn is_hidden(character: char, newline_allowed: bool) -> bool {
    const FORMAT_CHARACTERS: &[(u32, u32)] = &[
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    if character == '\n' {
        return !newline_allowed;
    }
    let code = u32::from(character);
    character.is_control()
        || code == 0x2028
        || code == 0x2029
        || FORMAT_CHARACTERS
            .iter()
            .any(|&(first, last)| (first..=last).contains(&code))
}

/// Plain text of `min..=max` bytes; `max_lines` of 1 means one line.
pub(crate) fn check_text(
    field: &str,
    text: &str,
    min: usize,
    max: usize,
    max_lines: usize,
) -> Result<(), NotificationError> {
    if !(min..=max).contains(&text.len()) {
        return Err(NotificationError(format!(
            "{field} must be {min} to {max} bytes, got {}",
            text.len()
        )));
    }
    if let Some(character) = text
        .chars()
        .find(|&character| is_hidden(character, max_lines > 1))
    {
        return Err(NotificationError(format!(
            "{field} must be plain text: {character:?} is a control, format or separator \
             character{}",
            if max_lines > 1 { "" } else { " (one line)" }
        )));
    }
    let lines = text.split('\n').count();
    if lines > max_lines {
        return Err(NotificationError(format!(
            "{field} has {lines} lines, at most {max_lines}"
        )));
    }
    Ok(())
}

impl Notification {
    /// The rules of design.md M.1 that need no host state (the owner, the caps and the
    /// capability are the host's).
    pub fn validate(&self) -> Result<(), NotificationError> {
        check_text("the title", &self.title, 1, MAX_TITLE_BYTES, 1)?;
        if let Some(body) = &self.body {
            check_text("the body", body, 0, MAX_BODY_BYTES, MAX_BODY_LINES)?;
        }
        if self.actions.len() > MAX_ACTIONS {
            return Err(NotificationError(format!(
                "a notification has at most {MAX_ACTIONS} actions, got {}",
                self.actions.len()
            )));
        }
        for (index, action) in self.actions.iter().enumerate() {
            if ApiVersion::V0_2_0.check_surface_id(&action.id).is_err() {
                return Err(NotificationError(format!(
                    "action id {:?} must be 1 to 64 bytes of a-z, 0-9, '.', '_' and '-'",
                    action.id
                )));
            }
            if self.actions[..index]
                .iter()
                .any(|other| other.id == action.id)
            {
                return Err(NotificationError(format!(
                    "action id {:?} is used twice",
                    action.id
                )));
            }
            check_text(
                "an action label",
                &action.label,
                1,
                MAX_ACTION_LABEL_BYTES,
                1,
            )?;
        }
        Ok(())
    }
}

impl ProgressOptions {
    /// The rules of design.md M.2 that need no host state.
    pub fn validate(&self) -> Result<(), NotificationError> {
        check_text("the title", &self.title, 1, MAX_TITLE_BYTES, 1)
    }
}

/// The value and message of a `progress-update` (design.md M.2).
pub fn check_progress_update(
    value: ProgressValue,
    message: Option<&str>,
) -> Result<(), NotificationError> {
    if let ProgressValue::Percent(percent) = value
        && percent > 100
    {
        return Err(NotificationError(format!(
            "a progress is 0 to 100 percent, got {percent}"
        )));
    }
    if let Some(message) = message {
        check_text("the message", message, 0, MAX_PROGRESS_MESSAGE_BYTES, 1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification() -> Notification {
        Notification {
            owner: "panel".into(),
            level: NotificationLevel::Info,
            title: "Build finished".into(),
            body: Some("3 problems found.\nSee the panel.".into()),
            actions: vec![
                NotificationAction {
                    id: "open".into(),
                    label: "Open".into(),
                },
                NotificationAction {
                    id: "later".into(),
                    label: "Later".into(),
                },
            ],
            auto_dismiss: true,
        }
    }

    #[test]
    fn notification_rules() {
        assert_eq!(notification().validate(), Ok(()));
        let broken: Vec<(&str, Box<dyn Fn(&mut Notification)>)> = vec![
            ("empty title", Box::new(|n| n.title.clear())),
            ("newline in title", Box::new(|n| n.title = "a\nb".into())),
            ("81-byte title", Box::new(|n| n.title = "a".repeat(81))),
            (
                "bidi override",
                Box::new(|n| n.title = "Fyx\u{202e}gnp".into()),
            ),
            ("zero width", Box::new(|n| n.title = "Fy\u{200b}x".into())),
            (
                "line separator",
                Box::new(|n| n.body = Some("a\u{2028}b".into())),
            ),
            (
                "5 lines",
                Box::new(|n| n.body = Some("1\n2\n3\n4\n5".into())),
            ),
            ("long body", Box::new(|n| n.body = Some("a".repeat(501)))),
            (
                "control in body",
                Box::new(|n| n.body = Some("a\u{1b}[31m".into())),
            ),
            (
                "third action",
                Box::new(|n| {
                    n.actions.push(NotificationAction {
                        id: "x".into(),
                        label: "X".into(),
                    })
                }),
            ),
            (
                "duplicate action",
                Box::new(|n| n.actions[1].id = "open".into()),
            ),
            (
                "bad action id",
                Box::new(|n| n.actions[0].id = "Open".into()),
            ),
            (
                "long label",
                Box::new(|n| n.actions[0].label = "a".repeat(25)),
            ),
            ("empty label", Box::new(|n| n.actions[0].label.clear())),
        ];
        for (name, mutate) in broken {
            let mut notification = notification();
            mutate(&mut notification);
            assert!(notification.validate().is_err(), "{name}");
        }
        let mut no_body = notification();
        no_body.body = None;
        no_body.actions.clear();
        assert_eq!(no_body.validate(), Ok(()));
    }

    #[test]
    fn progress_rules() {
        let options = ProgressOptions {
            owner: "panel".into(),
            title: "Indexing".into(),
            location: ProgressLocation::StatusBar,
            cancellable: true,
        };
        assert_eq!(options.validate(), Ok(()));
        assert!(
            ProgressOptions {
                title: "a\nb".into(),
                ..options
            }
            .validate()
            .is_err()
        );
        assert_eq!(
            check_progress_update(ProgressValue::Percent(100), Some("Step 3 of 4")),
            Ok(())
        );
        assert_eq!(
            check_progress_update(ProgressValue::Indeterminate, None),
            Ok(())
        );
        assert!(check_progress_update(ProgressValue::Percent(101), None).is_err());
        assert!(check_progress_update(ProgressValue::Indeterminate, Some("a\nb")).is_err());
        assert!(
            check_progress_update(ProgressValue::Indeterminate, Some(&"a".repeat(121))).is_err()
        );
        assert_eq!(
            NotificationLevel::Error.auto_dismiss_after(),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn json_form() {
        let json = serde_json::to_value(notification()).expect("serializes");
        assert_eq!(json["auto-dismiss"], true);
        assert_eq!(json["level"], "info");
        let event =
            super::super::UiEvent::NotificationClosed(7, NotificationCloseReason::WindowClosed);
        assert_eq!(
            serde_json::to_value(&event).expect("serializes"),
            serde_json::json!({ "notification-closed": [7, "window-closed"] })
        );
    }
}
