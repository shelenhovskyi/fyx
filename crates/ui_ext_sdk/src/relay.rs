//! `fyx:ui` 0.4: command hooks through `fyx-relay` (design.md B.9), with the
//! `fyx-ui-0-4` feature and the `relay:hooks` capability.
//!
//! A tool's hook runs `"$FYX_RELAY" --channel <extension-id>/<name> [args…]`. The host delivers
//! it as `IoEvent::RelayRequest((id, channel, terminal, argv, stdin))` in
//! [`Extension::handle_io`](crate::Extension), with `channel` the name of one of the manifest's
//! `[[fyx_ui.relay_channels]]` (without `<extension-id>/`), `terminal` the Fyx terminal the hook ran in (if any) and `stdin`
//! what the hook read (at most 1 MiB). [`respond`] answers it once: the relay prints `stdout`
//! and `stderr` and exits with `exit_code`. A request left unanswered ends with the channel's
//! `on_failure` after its `timeout_ms`.
//!
//! [`RelayRequest::from_event`] gives the event's fields names. Exported as 0.2 or 0.1,
//! [`respond`] fails with `unavailable`. Off WASM it records a
//! [`HostEffect::Relay`](crate::HostEffect::Relay).

use crate::HostError;
use crate::IoEvent;
use crate::terminals::TerminalHandle;

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_relay_respond(
        id: u64,
        exit_code: u8,
        stdout: &str,
        stderr: &str,
    ) -> Result<(), HostError>;
}

/// `IoEvent::RelayRequest`, with names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayRequest {
    /// [`respond`] names it.
    pub id: u64,
    /// The channel's name, as `[[fyx_ui.relay_channels]]` declares it.
    pub channel: String,
    /// The Fyx terminal the hook ran in.
    pub terminal: Option<TerminalHandle>,
    pub argv: Vec<String>,
    pub stdin: String,
}

impl RelayRequest {
    /// The request an `IoEvent::RelayRequest` carries; `None` for any other event.
    pub fn from_event(event: &IoEvent) -> Option<Self> {
        match event {
            IoEvent::RelayRequest((id, channel, terminal, argv, stdin)) => Some(Self {
                id: *id,
                channel: channel.clone(),
                terminal: *terminal,
                argv: argv.clone(),
                stdin: stdin.clone(),
            }),
            _ => None,
        }
    }

    /// The channel's name, also when a host passed it on as the hook wrote it
    /// (`<extension_id>/<name>`).
    pub fn channel_name(&self, extension_id: &str) -> &str {
        self.channel
            .strip_prefix(extension_id)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(&self.channel)
    }
}

/// What [`respond`] did, recorded off WASM in [`HostEffect::Relay`](crate::HostEffect::Relay).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEffect {
    pub id: u64,
    pub exit_code: u8,
    pub stdout: String,
    pub stderr: String,
}

/// `relay:hooks`: answers relay request `id`; see the module docs. An id that was answered,
/// timed out or never came is `invalid-argument`; `stdout` and `stderr` are at most 1 MiB
/// together.
pub fn respond(id: u64, exit_code: u8, stdout: &str, stderr: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_relay_respond(id, exit_code, stdout, stderr);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Relay(RelayEffect {
        id,
        exit_code,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }))
}

/// What `register_extension!`'s relay shim calls, per exported world.
#[doc(hidden)]
pub mod glue {
    /// The 0.4 import.
    pub mod newest {
        use crate::HostError;
        use crate::bindings::fyx::ui::relay as wit;

        pub fn respond(
            id: u64,
            exit_code: u8,
            stdout: &str,
            stderr: &str,
        ) -> Result<(), HostError> {
            wit::respond(id, exit_code, stdout, stderr)
        }
    }

    /// Worlds without the relay: `unavailable`.
    macro_rules! older {
        ($world:ident, $version:literal) => {
            pub mod $world {
                use crate::{HostError, HostErrorCode};

                pub fn respond(
                    _id: u64,
                    _exit_code: u8,
                    _stdout: &str,
                    _stderr: &str,
                ) -> Result<(), HostError> {
                    Err(HostError {
                        code: HostErrorCode::Unavailable,
                        message: format!(
                            "relay::respond needs fyx:ui 0.4; this extension is exported as {}",
                            $version
                        ),
                    })
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
    fn requests_and_answers_off_wasm() {
        take_host_effects();
        let event = IoEvent::RelayRequest((
            7,
            "fyx-claude-code/stop".into(),
            Some(3),
            vec!["--channel".into(), "fyx-claude-code/stop".into()],
            "{}".into(),
        ));
        let request = RelayRequest::from_event(&event).unwrap();
        assert_eq!(request.id, 7);
        assert_eq!(request.terminal, Some(3));
        assert_eq!(request.channel_name("fyx-claude-code"), "stop");
        assert_eq!(request.channel_name("other"), "fyx-claude-code/stop");
        assert_eq!(
            RelayRequest::from_event(&IoEvent::DiagnosticsChanged(Vec::new())),
            None
        );
        respond(7, 0, "", "").unwrap();
        assert_eq!(
            take_host_effects(),
            [HostEffect::Relay(RelayEffect {
                id: 7,
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            })]
        );
        let error = glue::v0_2::respond(7, 0, "", "").unwrap_err();
        assert_eq!(error.code, crate::HostErrorCode::Unavailable);
        assert!(error.message.contains("respond needs fyx:ui 0.4"));
    }
}
