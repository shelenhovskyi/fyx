//! Fake terminals for the staged `fyx:ui` 0.4 (design.md B.2-B.5), with the host's checks and
//! limits from `ui_ext::terminal`, so an extension's terminal code runs headless.
//!
//! - A **pseudo-terminal** keeps what the extension writes ([`FakeTerminal::output`]); a test
//!   types into it with [`crate::Harness::type_into_terminal`] and resizes it with
//!   [`crate::Harness::resize_terminal`], which reach the extension as io events.
//! - A **created terminal** follows the declared-program rule: the program is a
//!   `[[fyx_ui.terminal_programs]]` entry and the arguments fit its patterns. Its fake program
//!   echoes: every `send-input` comes back as `terminal-output`, as from a pty in cooked mode.
//!   [`crate::Harness::exit_terminal`] ends it.
//! - A **profile terminal** is one the user starts from a `[[fyx_ui.terminal_profiles]]` entry
//!   in "+" ([`crate::Harness::open_profile`], design.md B.1). It belongs to the user: the
//!   extension sees it in `list` and `terminal-created` (with `terminal:observe`), may set its
//!   status and paste into it while its program has bracketed paste on
//!   ([`crate::Harness::set_bracketed_paste`]), and never reads or writes it.
//! - Status, `list` (with `terminal:observe`) and `paste` work on the extension's own terminals;
//!   the T3 requests are `unavailable`, as no terminal of the user's exists here.

use std::collections::{BTreeMap, VecDeque};

use ui_ext::process::args_match;
use ui_ext::terminal::{
    self as model_terminal, CAPABILITY_CREATE, CAPABILITY_OBSERVE, CAPABILITY_PSEUDO,
    MAX_CREATED_TERMINALS, MAX_PASTE_BYTES, MAX_SEND_INPUT_BYTES, MAX_WRITE_OUTPUT_BYTES,
    PseudoRequest, TerminalEvent, TerminalHandle, TerminalInfo, TerminalOrigin, TerminalProfile,
    TerminalProgramSpec, TerminalStatus,
};
use ui_ext::v0_2::{HostError, HostErrorCode};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

/// One of the extension's terminals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeTerminal {
    pub origin: TerminalOrigin,
    pub title: String,
    /// Shown by a `terminal` node in the extension's panel rather than as a tab.
    pub in_panel: bool,
    /// Pseudo: every byte the extension wrote. Created: every byte the program printed.
    pub output: Vec<u8>,
    /// Created and profile: the program and arguments it runs (a profile's with `${package}`
    /// and `${relay}` resolved).
    pub command: Option<(String, Vec<String>)>,
    /// Profile: the `[[fyx_ui.terminal_profiles]]` id it was started from.
    pub profile: Option<String>,
    /// Profile: the account profile's name the user picked (design.md C.6), never its path.
    pub account: Option<String>,
    /// Profile: whether its program has bracketed paste on, which `paste` needs. Starts on.
    pub bracketed_paste: bool,
    pub status: Option<TerminalStatus>,
    /// Text pasted into it, in order.
    pub pasted: Vec<String>,
    /// The last size a test gave it.
    pub size: Option<(u16, u16)>,
    /// Created: the program ended (`terminal-exited` was queued).
    pub exited: bool,
}

impl FakeTerminal {
    fn blank(origin: TerminalOrigin) -> Self {
        Self {
            origin,
            title: String::new(),
            in_panel: false,
            output: Vec::new(),
            command: None,
            profile: None,
            account: None,
            bracketed_paste: false,
            status: None,
            pasted: Vec::new(),
            size: None,
            exited: false,
        }
    }

    /// [`Self::output`] as text without escape sequences: CSI, OSC and other `ESC` sequences are
    /// dropped, `\r` too, and the rest decoded lossily. Enough to assert what a TUI drew.
    pub fn plain_output(&self) -> String {
        plain_text(&self.output)
    }
}

/// `bytes` without terminal escape sequences and carriage returns.
pub fn plain_text(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            0x1b => {
                index += 1;
                match bytes.get(index) {
                    // CSI: parameters and intermediates, then one final byte in 0x40..=0x7e.
                    Some(b'[') => {
                        index += 1;
                        while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                            index += 1;
                        }
                        index += 1;
                    }
                    // OSC: up to BEL or ST (`ESC \`).
                    Some(b']') => {
                        index += 1;
                        while index < bytes.len() {
                            if bytes[index] == 0x07 {
                                index += 1;
                                break;
                            }
                            if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                                index += 2;
                                break;
                            }
                            index += 1;
                        }
                    }
                    Some(_) => index += 1,
                    None => {}
                }
            }
            b'\r' => index += 1,
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The extension's terminals and the events queued for it.
#[derive(Debug, Default)]
pub(crate) struct FakeTerminals {
    capabilities: Vec<String>,
    programs: Vec<TerminalProgramSpec>,
    profiles: Vec<TerminalProfile>,
    next_handle: u64,
    terminals: BTreeMap<u64, FakeTerminal>,
    /// For `handle-io`, oldest first.
    pub(crate) events: VecDeque<TerminalEvent>,
}

impl FakeTerminals {
    pub(crate) fn new(capabilities: &[String], programs: Vec<TerminalProgramSpec>) -> Self {
        Self {
            capabilities: capabilities.to_vec(),
            programs,
            ..Self::default()
        }
    }

    /// The capabilities and `[[fyx_ui.terminal_programs]]`, keeping the profiles and terminals.
    pub(crate) fn set_programs(
        &mut self,
        capabilities: &[String],
        programs: Vec<TerminalProgramSpec>,
    ) {
        self.capabilities = capabilities.to_vec();
        self.programs = programs;
    }

    pub(crate) fn set_profiles(&mut self, profiles: Vec<TerminalProfile>) {
        self.profiles = profiles;
    }

    pub(crate) fn profiles(&self) -> &[TerminalProfile] {
        &self.profiles
    }

    fn declares(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|name| name == capability)
    }

    fn require(&self, capability: &str) -> Result<(), HostError> {
        if self.declares(capability) {
            Ok(())
        } else {
            Err(error(
                HostErrorCode::NotPermitted,
                format!("{capability:?} is not in the manifest"),
            ))
        }
    }

    pub(crate) fn get(&self, handle: u64) -> Option<&FakeTerminal> {
        self.terminals.get(&handle)
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = (u64, &FakeTerminal)> {
        self.terminals
            .iter()
            .map(|(handle, terminal)| (*handle, terminal))
    }

    fn own(&mut self, handle: u64) -> Result<&mut FakeTerminal, HostError> {
        self.terminals
            .get_mut(&handle)
            .ok_or_else(|| model_terminal::TerminalError::NotFound.into())
    }

    fn add(&mut self, terminal: FakeTerminal) -> u64 {
        self.next_handle += 1;
        self.terminals.insert(self.next_handle, terminal);
        self.next_handle
    }

    pub(crate) fn open_pseudo(&mut self, request: PseudoRequest) -> Result<u64, HostError> {
        self.require(CAPABILITY_PSEUDO)?;
        request.check()?;
        Ok(self.add(FakeTerminal {
            title: request.title,
            in_panel: request.in_panel,
            size: request.cols.zip(request.rows),
            ..FakeTerminal::blank(TerminalOrigin::Pseudo)
        }))
    }

    pub(crate) fn write_output(&mut self, handle: u64, bytes: &[u8]) -> Result<(), HostError> {
        if bytes.len() > MAX_WRITE_OUTPUT_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("write-output takes at most {MAX_WRITE_OUTPUT_BYTES} bytes per call"),
            ));
        }
        let terminal = self.own(handle)?;
        if terminal.origin != TerminalOrigin::Pseudo {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "write-output is for pseudo-terminals; send-input types into a created one",
            ));
        }
        terminal.output.extend_from_slice(bytes);
        Ok(())
    }

    /// The declared-program rule of `terminals.create` (design.md B.3).
    pub(crate) fn create(
        &mut self,
        program: String,
        args: Vec<String>,
        title: Option<String>,
    ) -> Result<u64, HostError> {
        self.require(CAPABILITY_CREATE)?;
        let Some(spec) = self
            .programs
            .iter()
            .find(|spec| spec.name.eq_ignore_ascii_case(&program))
        else {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{program:?} is not in [[fyx_ui.terminal_programs]]"),
            ));
        };
        if !args_match(&spec.args, &args) {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!(
                    "the arguments {args:?} do not fit the args of {:?} in [[fyx_ui.terminal_programs]]",
                    spec.name
                ),
            ));
        }
        let live = self
            .terminals
            .values()
            .filter(|terminal| terminal.origin == TerminalOrigin::Created && !terminal.exited)
            .count();
        if live >= MAX_CREATED_TERMINALS {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("at most {MAX_CREATED_TERMINALS} created terminals per extension"),
            ));
        }
        if let Some(title) = &title
            && title.len() > model_terminal::MAX_TITLE_BYTES
        {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "the title is too long",
            ));
        }
        let title = title.unwrap_or_else(|| spec.name.clone());
        Ok(self.add(FakeTerminal {
            title,
            command: Some((spec.name.clone(), args)),
            ..FakeTerminal::blank(TerminalOrigin::Created)
        }))
    }

    /// The user starts profile `id` from "+" (design.md B.1), with the command line the host
    /// resolved. With `terminal:observe` the extension gets `terminal-created`.
    pub(crate) fn open_profile(
        &mut self,
        id: &str,
        command: (String, Vec<String>),
        account: Option<String>,
    ) -> Result<u64, HostError> {
        let Some(profile) = self.profiles.iter().find(|profile| profile.id == id) else {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{id:?} is not in [[fyx_ui.terminal_profiles]]"),
            ));
        };
        if account.is_some() && profile.account_tool.is_none() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("profile {id:?} has no account_tool, so no account profile is picked"),
            ));
        }
        let title = profile.name.clone();
        let handle = self.add(FakeTerminal {
            title,
            command: Some(command),
            profile: Some(id.to_owned()),
            account,
            bracketed_paste: true,
            ..FakeTerminal::blank(TerminalOrigin::Profile)
        });
        if self.declares(CAPABILITY_OBSERVE)
            && let Some(info) = self.info(handle)
        {
            self.events.push_back(TerminalEvent::Created(info));
        }
        Ok(handle)
    }

    /// The user closes a profile terminal; with `terminal:observe` the extension gets
    /// `terminal-closed`.
    pub(crate) fn close_profile(&mut self, handle: u64) -> Result<(), HostError> {
        if self.own(handle)?.origin != TerminalOrigin::Profile {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "only a profile terminal is the user's to close",
            ));
        }
        self.close(handle);
        if self.declares(CAPABILITY_OBSERVE) {
            self.events
                .push_back(TerminalEvent::Closed(TerminalHandle(handle)));
        }
        Ok(())
    }

    pub(crate) fn set_bracketed_paste(&mut self, handle: u64, on: bool) -> Result<(), HostError> {
        self.own(handle)?.bracketed_paste = on;
        Ok(())
    }

    fn info(&self, handle: u64) -> Option<TerminalInfo> {
        let terminal = self.terminals.get(&handle)?;
        Some(TerminalInfo {
            handle: TerminalHandle(handle),
            origin: terminal.origin,
            title: terminal.title.clone(),
            cwd: None,
            status: terminal.status.clone(),
            account: terminal.account.clone(),
        })
    }

    /// The fake program echoes what it is sent.
    pub(crate) fn send_input(&mut self, handle: u64, bytes: &[u8]) -> Result<(), HostError> {
        if bytes.len() > MAX_SEND_INPUT_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("send-input takes at most {MAX_SEND_INPUT_BYTES} bytes per call"),
            ));
        }
        let terminal = self.own(handle)?;
        if terminal.origin != TerminalOrigin::Created {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "send-input is for created terminals; a pseudo-terminal's input comes from the user",
            ));
        }
        if terminal.exited {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "the program has exited",
            ));
        }
        terminal.output.extend_from_slice(bytes);
        self.events.push_back(TerminalEvent::Output(
            TerminalHandle(handle),
            bytes.to_vec(),
            0,
        ));
        Ok(())
    }

    pub(crate) fn close(&mut self, handle: u64) {
        self.terminals.remove(&handle);
        self.events
            .retain(|event| event_handle(event) != Some(handle));
    }

    pub(crate) fn set_status(
        &mut self,
        handle: u64,
        status: Option<TerminalStatus>,
    ) -> Result<(), HostError> {
        if let Some(status) = &status {
            status.check()?;
        }
        // As the host: free on the extension's own terminals, `terminal:status` on others.
        let theirs = matches!(
            self.own(handle)?.origin,
            TerminalOrigin::Profile | TerminalOrigin::User
        );
        if theirs {
            self.require(model_terminal::CAPABILITY_STATUS)?;
        }
        let terminal = self.own(handle)?;
        terminal.status = status;
        let profile = terminal.origin == TerminalOrigin::Profile;
        if profile
            && self.declares(CAPABILITY_OBSERVE)
            && let Some(info) = self.info(handle)
        {
            self.events.push_back(TerminalEvent::StatusChanged(info));
        }
        Ok(())
    }

    pub(crate) fn list(&self) -> Result<Vec<TerminalInfo>, HostError> {
        self.require(CAPABILITY_OBSERVE)?;
        Ok(self
            .terminals
            .keys()
            .filter_map(|handle| self.info(*handle))
            .collect())
    }

    pub(crate) fn paste(&mut self, handle: u64, text: String) -> Result<(), HostError> {
        if text.len() > MAX_PASTE_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("paste takes at most {MAX_PASTE_BYTES} bytes"),
            ));
        }
        let terminal = self.own(handle)?;
        if terminal.origin == TerminalOrigin::Profile && !terminal.bracketed_paste {
            return Err(error(
                HostErrorCode::Unavailable,
                "the terminal's program has bracketed paste off, so a newline would run a command",
            ));
        }
        terminal.pasted.push(text);
        Ok(())
    }

    /// A key press or text from the user, for a pseudo-terminal's backend.
    pub(crate) fn type_into(&mut self, handle: u64, bytes: &[u8]) -> Result<(), HostError> {
        let terminal = self.own(handle)?;
        if terminal.origin != TerminalOrigin::Pseudo {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "only a pseudo-terminal's input goes to the extension",
            ));
        }
        self.events
            .push_back(TerminalEvent::Input(TerminalHandle(handle), bytes.to_vec()));
        Ok(())
    }

    /// A new size; only the last one queued is delivered, as the host coalesces them.
    pub(crate) fn resize(&mut self, handle: u64, cols: u16, rows: u16) -> Result<(), HostError> {
        self.own(handle)?.size = Some((cols, rows));
        self.events.retain(
            |event| !matches!(event, TerminalEvent::Resized(TerminalHandle(h), ..) if *h == handle),
        );
        self.events
            .push_back(TerminalEvent::Resized(TerminalHandle(handle), cols, rows));
        Ok(())
    }

    pub(crate) fn exit(&mut self, handle: u64, code: Option<i32>) -> Result<(), HostError> {
        let terminal = self.own(handle)?;
        if terminal.origin != TerminalOrigin::Created || terminal.exited {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "only a running created terminal exits",
            ));
        }
        terminal.exited = true;
        self.events
            .push_back(TerminalEvent::Exited(TerminalHandle(handle), code));
        Ok(())
    }

    /// The `terminal` nodes of one tree: each names one of the extension's own pseudo-terminals
    /// opened `in-panel`, or a created terminal, and no handle shows twice.
    pub(crate) fn check_nodes(&self, handles: &[u64]) -> Result<(), String> {
        for (index, handle) in handles.iter().enumerate() {
            let Some(terminal) = self.terminals.get(handle) else {
                return Err(format!(
                    "a terminal node names {handle}, which is not one of this extension's terminals"
                ));
            };
            if terminal.origin == TerminalOrigin::Profile {
                return Err(format!(
                    "a terminal node names profile terminal {handle}, which is the user's tab"
                ));
            }
            if terminal.origin == TerminalOrigin::Pseudo && !terminal.in_panel {
                return Err(format!(
                    "a terminal node names pseudo-terminal {handle}, which was opened as a tab; \
                     open it with in-panel"
                ));
            }
            if handles[..index].contains(handle) {
                return Err(format!("terminal {handle} is shown by two terminal nodes"));
            }
        }
        Ok(())
    }
}

fn event_handle(event: &TerminalEvent) -> Option<u64> {
    match event {
        TerminalEvent::Input(handle, _)
        | TerminalEvent::Resized(handle, ..)
        | TerminalEvent::Output(handle, ..)
        | TerminalEvent::Exited(handle, _)
        | TerminalEvent::Closed(handle) => Some(handle.0),
        TerminalEvent::Created(info) | TerminalEvent::StatusChanged(info) => Some(info.handle.0),
        // §C.3 answers carry a request id, not a terminal handle.
        TerminalEvent::ReadDone(..) | TerminalEvent::WriteDone(..) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminals() -> FakeTerminals {
        FakeTerminals::new(
            &[
                CAPABILITY_PSEUDO.to_owned(),
                CAPABILITY_CREATE.to_owned(),
                CAPABILITY_OBSERVE.to_owned(),
            ],
            vec![TerminalProgramSpec {
                name: "tool".into(),
                args: vec!["--mode".into(), "*".into()],
                path: None,
                reason: "runs the tool".into(),
            }],
        )
    }

    #[test]
    fn plain_text_drops_escape_sequences() {
        assert_eq!(
            plain_text(b"\x1b[2J\x1b[1;1H\x1b[1mHi\x1b[0m\r\n\x1b]0;title\x07ok"),
            "Hi\nok"
        );
    }

    #[test]
    fn created_terminals_follow_the_declared_program_rule_and_echo() {
        let mut terminals = terminals();
        let error = terminals.create("sh".into(), vec![], None).unwrap_err();
        assert!(
            error
                .message
                .contains("not in [[fyx_ui.terminal_programs]]")
        );
        let error = terminals
            .create("tool".into(), vec!["--other".into()], None)
            .unwrap_err();
        assert!(error.message.contains("do not fit"), "{error:?}");
        let handle = terminals
            .create("tool".into(), vec!["--mode".into(), "fast".into()], None)
            .expect("declared");
        terminals.send_input(handle, b"hi\n").expect("input");
        assert_eq!(
            terminals.events.pop_front(),
            Some(TerminalEvent::Output(
                TerminalHandle(handle),
                b"hi\n".to_vec(),
                0
            ))
        );
        assert!(terminals.write_output(handle, b"x").is_err());
        terminals.exit(handle, Some(0)).expect("exit");
        assert!(terminals.send_input(handle, b"x").is_err());
    }

    #[test]
    fn pseudo_terminals_keep_output_and_queue_input() {
        let mut terminals = terminals();
        let handle = terminals
            .open_pseudo(PseudoRequest {
                title: "Demo".into(),
                cols: None,
                rows: None,
                in_panel: true,
            })
            .expect("open");
        terminals
            .write_output(handle, b"\x1b[1mhello")
            .expect("write");
        assert_eq!(terminals.get(handle).unwrap().plain_output(), "hello");
        let too_big = vec![b'x'; MAX_WRITE_OUTPUT_BYTES + 1];
        assert_eq!(
            terminals.write_output(handle, &too_big).unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        terminals.resize(handle, 80, 24).unwrap();
        terminals.resize(handle, 100, 30).unwrap();
        terminals.type_into(handle, b"q").unwrap();
        assert_eq!(terminals.events.len(), 2, "one resize, the last");
        assert!(terminals.check_nodes(&[handle]).is_ok());
        assert!(terminals.check_nodes(&[handle, handle]).is_err());
        assert!(terminals.check_nodes(&[99]).is_err());
        terminals.close(handle);
        assert!(terminals.events.is_empty());
        assert!(terminals.write_output(handle, b"x").is_err());
    }

    #[test]
    fn profile_terminals_are_the_users() {
        let mut terminals = terminals();
        terminals.set_profiles(
            model_terminal::check_profiles(vec![model_terminal::TerminalProfileSpec {
                id: "claude".into(),
                name: "Claude Code".into(),
                icon: None,
                program: "claude".into(),
                path: None,
                args: Vec::new(),
                cwd: model_terminal::ProfileCwd::Worktree,
                account_tool: Some("claude".into()),
            }])
            .unwrap(),
        );
        let command = ("claude".to_owned(), Vec::new());
        let handle = terminals
            .open_profile("claude", command.clone(), Some("Work".into()))
            .unwrap();
        assert!(terminals.open_profile("other", command, None).is_err());
        // With terminal:observe: terminal-created, with the account's name.
        let Some(TerminalEvent::Created(info)) = terminals.events.pop_front() else {
            panic!("terminal-created");
        };
        assert_eq!(info.origin, TerminalOrigin::Profile);
        assert_eq!(info.account.as_deref(), Some("Work"));
        assert_eq!(
            terminals.list().unwrap()[0].account.as_deref(),
            Some("Work")
        );

        // Status (with terminal:status) and paste, never output or input; no terminal node.
        let waiting = TerminalStatus {
            state: model_terminal::StatusState::Blocked,
            label: "Waiting".into(),
            detail: None,
            progress: None,
            attention: true,
        };
        let refused = terminals
            .set_status(handle, Some(waiting.clone()))
            .unwrap_err();
        assert_eq!(refused.code, HostErrorCode::NotPermitted);
        terminals
            .capabilities
            .push(model_terminal::CAPABILITY_STATUS.to_owned());
        terminals.set_status(handle, Some(waiting)).unwrap();
        assert!(matches!(
            terminals.events.pop_front(),
            Some(TerminalEvent::StatusChanged(_))
        ));
        terminals
            .paste(handle, "@src/main.rs#L5-10 ".into())
            .unwrap();
        terminals.set_bracketed_paste(handle, false).unwrap();
        let refused = terminals.paste(handle, "ls\n".into()).unwrap_err();
        assert_eq!(refused.code, HostErrorCode::Unavailable);
        assert_eq!(
            terminals.get(handle).unwrap().pasted,
            ["@src/main.rs#L5-10 "]
        );
        assert!(terminals.write_output(handle, b"x").is_err());
        assert!(terminals.send_input(handle, b"x").is_err());
        assert!(terminals.type_into(handle, b"x").is_err());
        assert!(terminals.check_nodes(&[handle]).is_err());

        terminals.close_profile(handle).unwrap();
        assert!(matches!(
            terminals.events.pop_front(),
            Some(TerminalEvent::Closed(TerminalHandle(closed))) if closed == handle
        ));
        assert!(terminals.get(handle).is_none());
    }
}
