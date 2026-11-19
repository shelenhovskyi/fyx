//! fyx:ui 0.4 approval cards (docs/fyx/wit-0.4/design.md §B.12): a tool's own permission prompt
//! shown as a host-drawn card. The answer goes back to the tool through the extension; it grants
//! nothing in Fyx.

use serde::{Deserialize, Serialize};

use crate::v0_2::notifications::{NotificationError, check_text};
use crate::v0_2::{HostError, HostErrorCode};

/// The manifest capability (tier T0).
pub const CAPABILITY_APPROVALS: &str = "ui:approvals";
/// Open cards per extension; `ask` past it is `limit-exceeded`.
pub const MAX_OPEN_APPROVALS: usize = 4;
pub const MAX_TOOL_BYTES: usize = 40;
pub const MAX_TITLE_BYTES: usize = 120;
pub const MAX_DETAIL_BYTES: usize = 2000;
pub const MAX_DETAIL_LINES: usize = 12;
pub const MAX_OPTIONS: usize = 3;

/// A button of the card, the WIT `approval-option`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalOption {
    AllowOnce,
    AlwaysAllow,
    Deny,
}

/// One `approvals.ask` call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// The tool's name as the extension states it; the card marks it "says". `None` for an
    /// empty WIT string.
    pub tool: Option<String>,
    pub title: String,
    pub detail: String,
    pub options: Vec<ApprovalOption>,
}

impl ApprovalRequest {
    /// The rules of §B.12 that need no host state (the capability and the open cap are the
    /// host's). A broken rule is `invalid-argument`.
    pub fn check(&self) -> Result<(), HostError> {
        self.check_text().map_err(|error| HostError {
            code: HostErrorCode::InvalidArgument,
            message: error.0,
        })
    }

    fn check_text(&self) -> Result<(), NotificationError> {
        if let Some(tool) = &self.tool {
            check_text("the tool", tool, 1, MAX_TOOL_BYTES, 1)?;
        }
        check_text("the title", &self.title, 1, MAX_TITLE_BYTES, 1)?;
        check_text(
            "the detail",
            &self.detail,
            0,
            MAX_DETAIL_BYTES,
            MAX_DETAIL_LINES,
        )?;
        if !(1..=MAX_OPTIONS).contains(&self.options.len()) {
            return Err(NotificationError(format!(
                "an approval has 1 to {MAX_OPTIONS} options, got {}",
                self.options.len()
            )));
        }
        for (index, option) in self.options.iter().enumerate() {
            if self.options[..index].contains(option) {
                return Err(NotificationError(format!(
                    "option {option:?} is given twice"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ApprovalRequest {
        ApprovalRequest {
            tool: Some("Codex".into()),
            title: "Run `cargo test`?".into(),
            detail: "in ~/src/fyx\nwith network".into(),
            options: vec![ApprovalOption::AllowOnce, ApprovalOption::Deny],
        }
    }

    #[test]
    fn approval_rules() {
        assert_eq!(request().check(), Ok(()));
        let broken = |change: fn(&mut ApprovalRequest)| {
            let mut request = request();
            change(&mut request);
            request.check().unwrap_err().code
        };
        let invalid = HostErrorCode::InvalidArgument;
        assert_eq!(broken(|r| r.title.clear()), invalid);
        assert_eq!(broken(|r| r.title = "a\nb".into()), invalid);
        assert_eq!(broken(|r| r.title = "x".repeat(121)), invalid);
        assert_eq!(broken(|r| r.tool = Some(String::new())), invalid);
        assert_eq!(broken(|r| r.tool = Some("x".repeat(41))), invalid);
        assert_eq!(broken(|r| r.tool = Some("a\u{202e}b".into())), invalid);
        assert_eq!(broken(|r| r.detail = "x".repeat(2001)), invalid);
        assert_eq!(broken(|r| r.detail = "x\n".repeat(12)), invalid);
        assert_eq!(broken(|r| r.options.clear()), invalid);
        assert_eq!(
            broken(|r| r.options = vec![ApprovalOption::Deny, ApprovalOption::Deny]),
            invalid
        );
        let mut ok = request();
        ok.tool = None;
        ok.detail = "x\n".repeat(11) + "x";
        ok.options = vec![
            ApprovalOption::AllowOnce,
            ApprovalOption::AlwaysAllow,
            ApprovalOption::Deny,
        ];
        assert_eq!(ok.check(), Ok(()));
    }
}
