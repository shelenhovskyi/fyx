//! Capability tiers (platform spec decision 21) and the check that host imports make before they
//! act for an extension (plan 1.5).
//!
//! The tiers say how a capability is granted, not what it does:
//! - **T0** (UI surfaces and the like): no prompt; listed in the install dialog and on the chips.
//! - **T1** (reads of the workspace): granted once, in the install dialog; revocable.
//! - **T2** (network, processes, edits, agent): consent at first use. Nothing in `fyx:ui@0.1.0` or
//!   this host provides them yet, so a manifest that asks for one is refused.
//!
//! An unknown capability is refused at load, so a typo never quietly grants nothing (or, in a
//! later version, something else).
//!
//! [`CapabilityCheck`] is implemented by the host (`ui_ext_host`'s permission broker) and handed
//! to [`crate::UiExtension`] at load ([`crate::LoadOptions`]). A host import that reads or acts
//! calls [`crate::UiState::check_capability`] first; it answers from the broker's current state,
//! so a revocation applies to the next call at once.

use std::{fmt, sync::Arc};

use futures::FutureExt as _;

use anyhow::{Result, bail};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Tier {
    T0,
    T1,
    T2,
    /// 0.4 (design §C.3): at the moment of action, on one terminal.
    T3,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Tier::T0 => "T0",
            Tier::T1 => "T1",
            Tier::T2 => "T2",
            Tier::T3 => "T3",
        })
    }
}

pub const T0_CAPABILITIES: &[&str] = &[
    "ui:panel",
    "ui:status-bar",
    "ui:modal",
    "ui:notifications",
    "ui:editor-decorations",
    "ui:images",
    "ui:canvas",
    "ui:picker",
    "ui:code-lens",
    "ui:hover",
    "ui:virtual-documents",
    "ui:commands",
    // 0.4 (docs/fyx/wit-0.4/design.md §C.1).
    "package:files",
    "ui:approvals",
];

pub const T1_CAPABILITIES: &[&str] = &[
    "workspace:read-buffers",
    "workspace:read-files",
    "workspace:diagnostics",
    // 0.4.
    "context:editor",
    "workspace:open",
    "terminal:profiles",
    "terminal:pseudo",
    "terminal:status",
    "review:propose",
];

/// The 0.4 additions are accepted at install, one checkbox per item (design §C.2); until the
/// broker has that model (plan 5.6) they are consented like 0.3's. `terminal:read` and
/// `terminal:write` (T3, §C.3) are not here yet: a manifest that declares them is refused.
pub const T2_CAPABILITIES: &[&str] = &[
    "network:fetch",
    "process:spawn",
    "workspace:edit",
    "agent:observe",
    // 0.4.
    "network:listen",
    "terminal:create",
    "terminal:env",
    "terminal:observe",
    "fs:discovery-file",
    "relay:hooks",
];

/// 0.4 (design §C.3): asked on a card each time, never in advance except "Always allow".
pub const T3_CAPABILITIES: &[&str] = &[
    ui_ext::terminal::CAPABILITY_READ,
    ui_ext::terminal::CAPABILITY_WRITE,
];

/// The tier of a known capability.
pub fn tier_of(capability: &str) -> Option<Tier> {
    if T0_CAPABILITIES.contains(&capability) {
        Some(Tier::T0)
    } else if T1_CAPABILITIES.contains(&capability) {
        Some(Tier::T1)
    } else if T2_CAPABILITIES.contains(&capability) {
        Some(Tier::T2)
    } else if T3_CAPABILITIES.contains(&capability) {
        Some(Tier::T3)
    } else {
        None
    }
}

/// The tier of a capability a 0.1 or 0.2 extension can be granted: unknown ones, T2 and T3 are
/// refused.
pub fn grantable_tier(capability: &str) -> Result<Tier> {
    match tier_of(capability) {
        Some(Tier::T2 | Tier::T3) => bail!(
            "capability {capability:?} (T2) is not available in this version of Fyx; it needs \
             consent at first use, which comes with fyx:ui 0.3"
        ),
        Some(tier) => Ok(tier),
        None => bail!("unknown capability {capability:?}"),
    }
}

/// A capability an extension may not use now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityDenied {
    pub extension_id: String,
    pub capability: String,
    pub reason: String,
}

impl fmt::Display for CapabilityDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} may not use {}: {}",
            self.extension_id, self.capability, self.reason
        )
    }
}

impl std::error::Error for CapabilityDenied {}

/// The state of a T2 capability for one extension (design.md §7); defined in `ui_ext` so the
/// Apache harness shares it.
pub use ui_ext::network::PermissionState;

pub use crate::consent_items::{
    ConsentDiff, ConsentItem, ItemLine, PLUGIN_ENV_NAMES, WARNING_RUNS_ANY_CODE, Widened,
    consent_diff, consent_diff_lines, consent_items, consent_warnings,
    title_of as capability_title,
};

/// The two T3 capabilities.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum T3Capability {
    Read,
    Write,
}

impl T3Capability {
    pub fn name(self) -> &'static str {
        match self {
            T3Capability::Read => ui_ext::terminal::CAPABILITY_READ,
            T3Capability::Write => ui_ext::terminal::CAPABILITY_WRITE,
        }
    }

    pub fn from_name(capability: &str) -> Option<Self> {
        [T3Capability::Read, T3Capability::Write]
            .into_iter()
            .find(|case| case.name() == capability)
    }
}

/// The host's id of a terminal, which a T3 card names: not the extension's handle. Its form is
/// the terminal host's (W1) and may change when T3 is wired.
pub type HostTerminalId = u64;

/// A T3 card (design §C.3), asked after the runtime's checks passed: a gesture for a read, a
/// trusted worktree, the user's profile, and for a write a terminal shared for writing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct T3Prompt {
    pub capability: T3Capability,
    pub terminal: HostTerminalId,
    pub terminal_title: String,
    /// A read's scope and line count.
    pub scope: Option<(ui_ext::terminal::ReadScope, u16)>,
    /// A write's exact text.
    pub text: Option<String>,
    /// A write: Enter after the text.
    pub submit: bool,
}

/// The answer to a [`T3Prompt`]. A closed card, a dropped sender, or a cap reached is `Deny`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum T3Answer {
    Once,
    /// This terminal until it closes.
    UntilClosed,
    Deny,
}

/// Pending T3 cards per (extension, terminal) and per extension; past them the answer is
/// `Deny` with no card.
pub const MAX_T3_PROMPTS_PER_TERMINAL: usize = 1;
pub const MAX_T3_PROMPTS_PER_EXTENSION: usize = 4;

/// Answers whether an extension may use a capability right now. Implemented by the host's
/// permission broker; called from host imports on the extension's call queue and from tokio,
/// so it must be cheap and thread-safe.
pub trait CapabilityCheck: Send + Sync + 'static {
    /// For T0 and T1, and for T2 only when it is `Granted` (`Ask`, `Denied` and `Unavailable`
    /// are errors with that reason).
    fn check(&self, extension_id: &str, capability: &str) -> Result<(), CapabilityDenied>;

    /// A T2 capability's state, for `permissions.query` and the T2 imports.
    fn state(&self, _extension_id: &str, _capability: &str) -> PermissionState {
        PermissionState::Unavailable
    }

    /// First use (or `permissions.request`): in state `Ask`, shows the prompt in the active
    /// window (concurrent requests share one) and resolves with the answer, `Granted` or
    /// `Denied` (refused or closed). With no window it resolves `Ask` at once and records
    /// nothing. Any other state resolves at once. Awaited outside any call budget, after the
    /// call that started the operation returned.
    fn request(
        &self,
        _extension_id: &str,
        _capability: &str,
    ) -> futures::future::BoxFuture<'static, PermissionState> {
        futures::future::ready(PermissionState::Ask).boxed()
    }

    /// Every state change of this extension's T2 capabilities, as `(capability, state)`: the
    /// host sends `io-event::permission-changed` for each and cancels in-flight work of a
    /// revoked one (decision 26). 0.4 (§C.2): a change of one item comes as its
    /// [`ConsentItem::key`] (`process:spawn#codex`), which only stops that item's work; the
    /// capability's own line follows when its state changed too.
    fn changes(
        &self,
        _extension_id: &str,
    ) -> futures::channel::mpsc::UnboundedReceiver<(String, PermissionState)> {
        futures::channel::mpsc::unbounded().1
    }

    /// 0.4 (design §C.2): the state of one item, such as a single program. A broker without
    /// items answers with the item's capability.
    fn item_state(&self, extension_id: &str, item: &ConsentItem) -> PermissionState {
        self.state(extension_id, item.capability())
    }

    /// 0.4 (design §C.3): show a T3 card and resolve with the answer. Without a card (no
    /// broker support, no window) the answer is `Deny`.
    fn t3_request(
        &self,
        _extension_id: &str,
        _prompt: T3Prompt,
    ) -> futures::future::BoxFuture<'static, T3Answer> {
        futures::future::ready(T3Answer::Deny).boxed()
    }
}

/// The broker when a T2 operation is applied (design.md §0.3 step 3, §7): `granted` goes
/// ahead, `ask` waits for the prompt, anything else refuses. Without a broker, T2 is refused.
pub(crate) async fn consent(
    broker: Option<&Arc<dyn CapabilityCheck>>,
    extension_id: &str,
    capability: &str,
) -> bool {
    let Some(broker) = broker else {
        return false;
    };
    match broker.state(extension_id, capability) {
        PermissionState::Granted => true,
        PermissionState::Ask => {
            broker.request(extension_id, capability).await == PermissionState::Granted
        }
        PermissionState::Denied | PermissionState::Unavailable => false,
    }
}

/// The check when the host gave none: T1 and T2 are denied (T0 needs no grant).
pub(crate) fn check_without_broker(
    check: Option<&Arc<dyn CapabilityCheck>>,
    extension_id: &str,
    capability: &str,
) -> Result<(), CapabilityDenied> {
    if let Some(check) = check {
        return check.check(extension_id, capability);
    }
    match tier_of(capability) {
        Some(Tier::T0) => Ok(()),
        _ => Err(CapabilityDenied {
            extension_id: extension_id.to_string(),
            capability: capability.to_string(),
            reason: "no permission broker".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiers() {
        assert_eq!(tier_of("ui:panel"), Some(Tier::T0));
        assert_eq!(tier_of("workspace:read-buffers"), Some(Tier::T1));
        assert_eq!(tier_of("network:fetch"), Some(Tier::T2));
        assert_eq!(tier_of("ui:panels"), None);
        assert!(grantable_tier("ui:status-bar").is_ok());
        assert!(grantable_tier("workspace:diagnostics").is_ok());
        let error = grantable_tier("process:spawn").unwrap_err();
        assert!(format!("{error}").contains("not available"), "{error}");
        let error = grantable_tier("ui:pannel").unwrap_err();
        assert!(format!("{error}").contains("unknown capability"), "{error}");
        assert!(check_without_broker(None, "x", "ui:panel").is_ok());
        assert!(check_without_broker(None, "x", "workspace:read-files").is_err());
    }
}
