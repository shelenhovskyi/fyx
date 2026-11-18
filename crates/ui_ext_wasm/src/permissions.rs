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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    T0,
    T1,
    T2,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Tier::T0 => "T0",
            Tier::T1 => "T1",
            Tier::T2 => "T2",
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
];

pub const T1_CAPABILITIES: &[&str] = &[
    "workspace:read-buffers",
    "workspace:read-files",
    "workspace:diagnostics",
];

pub const T2_CAPABILITIES: &[&str] = &[
    "network:fetch",
    "process:spawn",
    "workspace:edit",
    "agent:observe",
];

/// The tier of a known capability.
pub fn tier_of(capability: &str) -> Option<Tier> {
    if T0_CAPABILITIES.contains(&capability) {
        Some(Tier::T0)
    } else if T1_CAPABILITIES.contains(&capability) {
        Some(Tier::T1)
    } else if T2_CAPABILITIES.contains(&capability) {
        Some(Tier::T2)
    } else {
        None
    }
}

/// The tier of a capability this host can grant: unknown ones and T2 ones are refused.
pub fn grantable_tier(capability: &str) -> Result<Tier> {
    match tier_of(capability) {
        Some(Tier::T2) => bail!(
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
    /// revoked one (decision 26).
    fn changes(
        &self,
        _extension_id: &str,
    ) -> futures::channel::mpsc::UnboundedReceiver<(String, PermissionState)> {
        futures::channel::mpsc::unbounded().1
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
