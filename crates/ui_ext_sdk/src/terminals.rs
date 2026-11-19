//! `fyx:ui` 0.4, staged: terminals (design.md B.1-B.5, B.11, C.3), with the `fyx-ui-0-4`
//! feature.
//!
//! - **Pseudo-terminals** (`terminal:pseudo`): [`open_pseudo`] opens a terminal with no process;
//!   the extension is its backend. It writes output with [`write_output`] and gets the user's
//!   keys and resizes in [`Extension::handle_io`](crate::Extension) as `IoEvent::TerminalInput`
//!   and `IoEvent::TerminalResized`. A [`TerminalProps`](crate::TerminalProps) node
//!   ([`TreeBuilder::terminal`](crate::TreeBuilder::terminal)) shows one inside the extension's
//!   panel.
//! - **Created terminals** (`terminal:create`): [`create`] runs a `[[fyx_ui.terminal_programs]]`
//!   program in a pty. Its output arrives as `IoEvent::TerminalOutput`, its end as
//!   `IoEvent::TerminalExited`; [`send_input`] types into it.
//! - [`set_status`] on the extension's own terminals, [`list`] with `terminal:observe`, [`paste`],
//!   and the T3 [`request_read`] and [`request_write`].
//!
//! As with the other host functions, a component imports only what it calls. Exported as 0.2 or
//! 0.1, every function here fails with `unavailable`. Off WASM each one records a
//! [`HostEffect::Terminal`](crate::HostEffect::Terminal) and succeeds, handing out the shared
//! ids 1, 2, ….

use crate::HostError;
pub use crate::bindings::fyx::ui::terminals::{
    CreateRequest, PseudoRequest, ReadScope, StatusState, TerminalHandle, TerminalInfo,
    TerminalOrigin, TerminalStatus,
};

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_terminal_open_pseudo(request: &PseudoRequest) -> Result<u64, HostError>;
    safe fn __fyx_ui_terminal_write_output(handle: u64, bytes: &[u8]) -> Result<(), HostError>;
    safe fn __fyx_ui_terminal_create(request: &CreateRequest) -> Result<u64, HostError>;
    safe fn __fyx_ui_terminal_send_input(handle: u64, bytes: &[u8]) -> Result<(), HostError>;
    safe fn __fyx_ui_terminal_close(handle: u64);
    safe fn __fyx_ui_terminal_set_status(
        handle: u64,
        status: Option<&TerminalStatus>,
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_terminal_list() -> Result<Vec<TerminalInfo>, HostError>;
    safe fn __fyx_ui_terminal_paste(handle: u64, text: &str) -> Result<(), HostError>;
    safe fn __fyx_ui_terminal_request_read(
        handle: u64,
        scope: ReadScope,
        lines: u16,
    ) -> Result<u64, HostError>;
    safe fn __fyx_ui_terminal_request_write(
        handle: u64,
        text: &str,
        submit: bool,
    ) -> Result<u64, HostError>;
}

/// What a terminal function did, recorded off WASM in
/// [`HostEffect::Terminal`](crate::HostEffect::Terminal).
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalEffect {
    OpenPseudo {
        handle: TerminalHandle,
        request: PseudoRequest,
    },
    WriteOutput {
        handle: TerminalHandle,
        bytes: Vec<u8>,
    },
    Create {
        handle: TerminalHandle,
        request: CreateRequest,
    },
    SendInput {
        handle: TerminalHandle,
        bytes: Vec<u8>,
    },
    Close(TerminalHandle),
    SetStatus {
        handle: TerminalHandle,
        status: Option<TerminalStatus>,
    },
    Paste {
        handle: TerminalHandle,
        text: String,
    },
    RequestRead {
        id: u64,
        handle: TerminalHandle,
        scope: ReadScope,
        lines: u16,
    },
    RequestWrite {
        id: u64,
        handle: TerminalHandle,
        text: String,
        submit: bool,
    },
}

#[cfg(not(target_arch = "wasm32"))]
fn record(effect: TerminalEffect) -> Result<(), HostError> {
    crate::host::record(crate::HostEffect::Terminal(effect))
}

impl PseudoRequest {
    /// A pseudo-terminal titled `title`, opened as a tab in the terminal panel at the host's
    /// size.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            cols: None,
            rows: None,
            in_panel: false,
        }
    }

    /// Shown by a `terminal` node in the extension's panel rather than as a tab.
    pub fn in_panel(mut self) -> Self {
        self.in_panel = true;
        self
    }

    /// The size the extension would like; the view decides, and reports it with
    /// `IoEvent::TerminalResized`.
    pub fn size(mut self, cols: u16, rows: u16) -> Self {
        self.cols = Some(cols);
        self.rows = Some(rows);
        self
    }
}

impl CreateRequest {
    /// Runs `program`, a `[[fyx_ui.terminal_programs]]` name, with no arguments in the active
    /// worktree.
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            title: None,
        }
    }

    /// Adds an argument; the program's `args` patterns must allow it.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// A worktree-relative directory to start in.
    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

impl TerminalStatus {
    /// A status the host draws on the terminal's tab, next to the extension's name.
    pub fn new(state: StatusState, label: impl Into<String>) -> Self {
        Self {
            state,
            label: label.into(),
            detail: None,
            progress: None,
            attention: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 0 to 100.
    pub fn progress(mut self, percent: u8) -> Self {
        self.progress = Some(percent);
        self
    }

    /// Sorts the terminal first in the agent sessions panel.
    pub fn needs_attention(mut self) -> Self {
        self.attention = true;
        self
    }
}

/// `terminal:pseudo`: opens a pseudo-terminal and returns its handle.
pub fn open_pseudo(request: &PseudoRequest) -> Result<TerminalHandle, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_open_pseudo(request);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let handle = crate::host::next_id();
        record(TerminalEffect::OpenPseudo {
            handle,
            request: request.clone(),
        })?;
        Ok(handle)
    }
}

/// Output of one of the extension's pseudo-terminals: at most 64 KiB per call and 4 MiB/s per
/// terminal (above it, `limit-exceeded`). The host removes the sequences a terminal answers
/// (DA, DSR, OSC 52, …) before the terminal sees them.
pub fn write_output(handle: TerminalHandle, bytes: &[u8]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_write_output(handle, bytes);
    #[cfg(not(target_arch = "wasm32"))]
    record(TerminalEffect::WriteOutput {
        handle,
        bytes: bytes.to_vec(),
    })
}

/// `terminal:create`: runs a declared program in a pty, as the extension's own terminal. Not in
/// an untrusted worktree. At most 8 per extension.
pub fn create(request: &CreateRequest) -> Result<TerminalHandle, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_create(request);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let handle = crate::host::next_id();
        record(TerminalEffect::Create {
            handle,
            request: request.clone(),
        })?;
        Ok(handle)
    }
}

/// Input for one of the extension's created terminals, at most 64 KiB per call.
pub fn send_input(handle: TerminalHandle, bytes: &[u8]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_send_input(handle, bytes);
    #[cfg(not(target_arch = "wasm32"))]
    record(TerminalEffect::SendInput {
        handle,
        bytes: bytes.to_vec(),
    })
}

/// Closes one of the extension's own terminals; a created one is killed with its process tree.
pub fn close(handle: TerminalHandle) {
    #[cfg(target_arch = "wasm32")]
    __fyx_ui_terminal_close(handle);
    #[cfg(not(target_arch = "wasm32"))]
    record(TerminalEffect::Close(handle)).ok();
}

/// Sets or clears the status on a terminal's tab. No capability on the extension's own
/// terminals, `terminal:status` on others. At most 4 updates per second per terminal.
pub fn set_status(
    handle: TerminalHandle,
    status: Option<&TerminalStatus>,
) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_set_status(handle, status);
    #[cfg(not(target_arch = "wasm32"))]
    record(TerminalEffect::SetStatus {
        handle,
        status: status.cloned(),
    })
}

/// `terminal:observe`: every terminal, with command lines redacted. Off WASM, none.
pub fn list() -> Result<Vec<TerminalInfo>, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_list();
    #[cfg(not(target_arch = "wasm32"))]
    Ok(Vec::new())
}

/// Pastes `text` into the extension's profile or own terminal, after a gesture, in
/// bracketed-paste mode. At most 16 KiB.
pub fn paste(handle: TerminalHandle, text: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_paste(handle, text);
    #[cfg(not(target_arch = "wasm32"))]
    record(TerminalEffect::Paste {
        handle,
        text: text.to_owned(),
    })
}

/// T3 `terminal:read`, after a gesture: the user confirms on a card. `lines` at most 500. The
/// answer arrives as `IoEvent::TerminalReadDone` with the returned id.
pub fn request_read(
    handle: TerminalHandle,
    scope: ReadScope,
    lines: u16,
) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_request_read(handle, scope, lines);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = crate::host::next_id();
        record(TerminalEffect::RequestRead {
            id,
            handle,
            scope,
            lines,
        })?;
        Ok(id)
    }
}

/// T3 `terminal:write`, after a gesture: the user confirms the exact text on a card. The answer
/// arrives as `IoEvent::TerminalWriteDone` with the returned id.
pub fn request_write(handle: TerminalHandle, text: &str, submit: bool) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_terminal_request_write(handle, text, submit);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let id = crate::host::next_id();
        record(TerminalEffect::RequestWrite {
            id,
            handle,
            text: text.to_owned(),
            submit,
        })?;
        Ok(id)
    }
}

/// What `register_extension!`'s terminal shims call, per exported world.
#[doc(hidden)]
pub mod glue {
    /// The 0.4 imports.
    pub mod newest {
        use super::super::{CreateRequest, PseudoRequest, ReadScope, TerminalInfo, TerminalStatus};
        use crate::HostError;
        use crate::bindings::fyx::ui::terminals as wit;

        pub fn open_pseudo(request: &PseudoRequest) -> Result<u64, HostError> {
            wit::open_pseudo(request)
        }

        pub fn write_output(handle: u64, bytes: &[u8]) -> Result<(), HostError> {
            wit::write_output(handle, bytes)
        }

        pub fn create(request: &CreateRequest) -> Result<u64, HostError> {
            wit::create(request)
        }

        pub fn send_input(handle: u64, bytes: &[u8]) -> Result<(), HostError> {
            wit::send_input(handle, bytes)
        }

        pub fn close(handle: u64) {
            wit::close(handle);
        }

        pub fn set_status(handle: u64, status: Option<&TerminalStatus>) -> Result<(), HostError> {
            wit::set_status(handle, status)
        }

        pub fn list() -> Result<Vec<TerminalInfo>, HostError> {
            wit::list()
        }

        pub fn paste(handle: u64, text: &str) -> Result<(), HostError> {
            wit::paste(handle, text)
        }

        pub fn request_read(handle: u64, scope: ReadScope, lines: u16) -> Result<u64, HostError> {
            wit::request_read(handle, scope, lines)
        }

        pub fn request_write(handle: u64, text: &str, submit: bool) -> Result<u64, HostError> {
            wit::request_write(handle, text, submit)
        }
    }

    /// Worlds without terminals: every call is `unavailable`.
    macro_rules! older {
        ($world:ident, $version:literal) => {
            pub mod $world {
                use super::super::{
                    CreateRequest, PseudoRequest, ReadScope, TerminalInfo, TerminalStatus,
                };
                use crate::{HostError, HostErrorCode};

                fn unavailable(what: &str) -> HostError {
                    HostError {
                        code: HostErrorCode::Unavailable,
                        message: format!(
                            "terminals::{what} needs fyx:ui 0.4; this extension is exported as {}",
                            $version
                        ),
                    }
                }

                pub fn open_pseudo(_request: &PseudoRequest) -> Result<u64, HostError> {
                    Err(unavailable("open_pseudo"))
                }

                pub fn write_output(_handle: u64, _bytes: &[u8]) -> Result<(), HostError> {
                    Err(unavailable("write_output"))
                }

                pub fn create(_request: &CreateRequest) -> Result<u64, HostError> {
                    Err(unavailable("create"))
                }

                pub fn send_input(_handle: u64, _bytes: &[u8]) -> Result<(), HostError> {
                    Err(unavailable("send_input"))
                }

                /// Nothing to close: no terminal was opened.
                pub fn close(_handle: u64) {}

                pub fn set_status(
                    _handle: u64,
                    _status: Option<&TerminalStatus>,
                ) -> Result<(), HostError> {
                    Err(unavailable("set_status"))
                }

                pub fn list() -> Result<Vec<TerminalInfo>, HostError> {
                    Err(unavailable("list"))
                }

                pub fn paste(_handle: u64, _text: &str) -> Result<(), HostError> {
                    Err(unavailable("paste"))
                }

                pub fn request_read(
                    _handle: u64,
                    _scope: ReadScope,
                    _lines: u16,
                ) -> Result<u64, HostError> {
                    Err(unavailable("request_read"))
                }

                pub fn request_write(
                    _handle: u64,
                    _text: &str,
                    _submit: bool,
                ) -> Result<u64, HostError> {
                    Err(unavailable("request_write"))
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
    fn terminal_calls_are_recorded_off_wasm() {
        take_host_effects();
        let request = PseudoRequest::new("Demo").in_panel().size(80, 24);
        let handle = open_pseudo(&request).expect("open");
        write_output(handle, b"hello\r\n").expect("write");
        let status = TerminalStatus::new(StatusState::Working, "Busy").progress(40);
        set_status(handle, Some(&status)).expect("status");
        let created = create(&CreateRequest::new("tool").args(["--flag", "x"])).expect("create");
        assert_ne!(created, handle);
        close(handle);
        assert_eq!(
            take_host_effects(),
            [
                HostEffect::Terminal(TerminalEffect::OpenPseudo { handle, request }),
                HostEffect::Terminal(TerminalEffect::WriteOutput {
                    handle,
                    bytes: b"hello\r\n".to_vec(),
                }),
                HostEffect::Terminal(TerminalEffect::SetStatus {
                    handle,
                    status: Some(status),
                }),
                HostEffect::Terminal(TerminalEffect::Create {
                    handle: created,
                    request: CreateRequest {
                        program: "tool".into(),
                        args: vec!["--flag".into(), "x".into()],
                        cwd: None,
                        title: None,
                    },
                }),
                HostEffect::Terminal(TerminalEffect::Close(handle)),
            ]
        );
    }

    #[test]
    fn older_worlds_have_no_terminals() {
        let error = glue::v0_2::open_pseudo(&PseudoRequest::new("x")).unwrap_err();
        assert_eq!(error.code, crate::HostErrorCode::Unavailable);
        assert!(
            error.message.contains("open_pseudo needs fyx:ui 0.4"),
            "{error:?}"
        );
        assert!(glue::v0_1::list().is_err());
    }
}
