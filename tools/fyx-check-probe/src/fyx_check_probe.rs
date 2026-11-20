//! Check Probe: a `fyx:ui` 0.4 test extension for dev-build checks of T3 terminal cards,
//! approval cards and `terminal:observe` (see README.md). One panel with a command input and a
//! log; the commands are parsed by [`Command::parse`], which is pure and tested without a host.

use std::collections::VecDeque;

use ui_ext_sdk::*;

const PANEL: &str = "check-probe";
const LOG_LINES: usize = 16;
const DEFAULT_READ_LINES: u16 = 40;
const MAX_READ_LINES: u16 = 500;
const HELP: &str = "commands: list, read <n> [lines], read-command <n>, write <n> <text>, \
                    write-enter <n> <text>, open, approve, clear";

/// One line of the command input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    List,
    Read {
        handle: u64,
        scope: ReadKind,
        lines: u16,
    },
    Write {
        handle: u64,
        text: String,
        submit: bool,
    },
    Open,
    Approve,
    Clear,
}

/// [`terminals::ReadScope`], comparable without the bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    LastLines,
    LastCommand,
}

impl Command {
    pub fn parse(line: &str) -> Result<Self, String> {
        let line = line.trim();
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim_start();
        match word {
            "" | "help" => Ok(Self::Help),
            "list" => Ok(Self::List),
            "open" => Ok(Self::Open),
            "approve" => Ok(Self::Approve),
            "clear" => Ok(Self::Clear),
            "read" => {
                let (handle, rest) = split_handle(rest)?;
                let lines = match rest.trim() {
                    "" => DEFAULT_READ_LINES,
                    lines => lines
                        .parse::<u16>()
                        .ok()
                        .filter(|lines| (1..=MAX_READ_LINES).contains(lines))
                        .ok_or_else(|| format!("lines must be 1 to {MAX_READ_LINES}"))?,
                };
                Ok(Self::Read {
                    handle,
                    scope: ReadKind::LastLines,
                    lines,
                })
            }
            "read-command" => {
                let (handle, _) = split_handle(rest)?;
                Ok(Self::Read {
                    handle,
                    scope: ReadKind::LastCommand,
                    lines: MAX_READ_LINES,
                })
            }
            "write" | "write-enter" => {
                let (handle, text) = split_handle(rest)?;
                if text.is_empty() {
                    return Err(format!("usage: {word} <n> <text>"));
                }
                Ok(Self::Write {
                    handle,
                    text: text.to_owned(),
                    submit: word == "write-enter",
                })
            }
            other => Err(format!("{other}: unknown command; {HELP}")),
        }
    }
}

fn split_handle(rest: &str) -> Result<(u64, &str), String> {
    let (number, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let handle = number
        .parse::<u64>()
        .map_err(|_| "a terminal number from `list` is needed".to_owned())?;
    Ok((handle, rest))
}

fn origin_name(origin: terminals::TerminalOrigin) -> &'static str {
    match origin {
        terminals::TerminalOrigin::Pseudo => "pseudo",
        terminals::TerminalOrigin::Created => "created",
        terminals::TerminalOrigin::Profile => "profile",
        terminals::TerminalOrigin::User => "user",
    }
}

/// The last [`LOG_LINES`] lines the panel shows, newest last.
#[derive(Debug, Default)]
pub struct Log {
    lines: VecDeque<String>,
}

impl Log {
    pub fn push(&mut self, text: impl AsRef<str>) {
        for line in text.as_ref().lines() {
            if self.lines.len() == LOG_LINES {
                self.lines.pop_front();
            }
            self.lines.push_back(line.to_owned());
        }
    }

    pub fn line(&self, index: usize) -> &str {
        self.lines.get(index).map(String::as_str).unwrap_or("")
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

pub struct CheckProbe {
    tree: SurfaceTree,
    log: Log,
    input: Option<NodeId>,
    log_nodes: Vec<NodeId>,
    own_terminal: Option<terminals::TerminalHandle>,
}

impl CheckProbe {
    fn run(&mut self, command: Command) {
        match command {
            Command::Help => self.log.push(HELP),
            Command::Clear => self.log.clear(),
            Command::List => match terminals::list() {
                Ok(infos) if infos.is_empty() => self.log.push("no terminals"),
                Ok(infos) => {
                    for info in infos {
                        self.log.push(format!(
                            "{}:{}:{}",
                            info.handle,
                            origin_name(info.origin),
                            info.title
                        ));
                    }
                }
                Err(error) => self.log.push(format!("list failed: {}", error.message)),
            },
            Command::Read {
                handle,
                scope,
                lines,
            } => {
                let scope = match scope {
                    ReadKind::LastLines => terminals::ReadScope::LastLines,
                    ReadKind::LastCommand => terminals::ReadScope::LastCommand,
                };
                match terminals::request_read(handle, scope, lines) {
                    Ok(id) => self
                        .log
                        .push(format!("read #{id} asked for terminal {handle}")),
                    Err(error) => self.log.push(format!("read refused: {}", error.message)),
                }
            }
            Command::Write {
                handle,
                text,
                submit,
            } => match terminals::request_write(handle, &text, submit) {
                Ok(id) => self
                    .log
                    .push(format!("write #{id} asked for terminal {handle}")),
                Err(error) => self.log.push(format!("write refused: {}", error.message)),
            },
            Command::Open => {
                if let Some(handle) = self.own_terminal {
                    self.log
                        .push(format!("own terminal already open: {handle}"));
                    return;
                }
                match terminals::open_pseudo(&terminals::PseudoRequest::new("Check Probe")) {
                    Ok(handle) => {
                        self.own_terminal = Some(handle);
                        if let Err(error) =
                            terminals::write_output(handle, b"Check Probe's own terminal\r\n")
                        {
                            self.log
                                .push(format!("write-output failed: {}", error.message));
                        }
                        self.log.push(format!("opened own terminal {handle}"));
                    }
                    Err(error) => self.log.push(format!("open failed: {}", error.message)),
                }
            }
            Command::Approve => self.approve(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn approve(&mut self) {
        use ui_ext_sdk::bindings::fyx::ui::approvals::{ApprovalOption, ask};
        match ask(
            "probe_tool",
            "Check Probe wants to run probe_tool",
            "A made-up tool call, to check the approval card.",
            &[
                ApprovalOption::AllowOnce,
                ApprovalOption::AlwaysAllow,
                ApprovalOption::Deny,
            ],
        ) {
            Ok(id) => self.log.push(format!("approval #{id} asked")),
            Err(error) => self
                .log
                .push(format!("approval refused: {}", error.message)),
        }
    }

    /// The SDK has no off-WASM stand-in for approvals.
    #[cfg(not(target_arch = "wasm32"))]
    fn approve(&mut self) {
        self.log.push("approve works only in Fyx");
    }

    fn refresh(&mut self) -> Result<(), String> {
        for (index, id) in self.log_nodes.iter().enumerate() {
            self.tree
                .set_props(*id, TextProps::new(self.log.line(index)))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

impl Extension for CheckProbe {
    fn new() -> Self {
        let mut log = Log::default();
        log.push(HELP);
        Self {
            tree: SurfaceTree::default(),
            log,
            input: None,
            log_nodes: Vec::new(),
            own_terminal: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Check Probe",
            DockPosition::Right,
            Icon::Terminal,
            360,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let lines: Vec<String> = (0..LOG_LINES)
            .map(|index| self.log.line(index).to_owned())
            .collect();
        let mut input = None;
        let mut log_nodes = Vec::new();
        let nodes = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(2).padding(8), |tree| {
                input = Some(
                    tree.input(
                        InputProps::new("")
                            .placeholder("Command, then Enter (help)")
                            .label("Check Probe command"),
                    ),
                );
                for line in lines {
                    log_nodes.push(tree.text(TextProps::new(line)));
                }
            });
        })?;
        self.input = input;
        self.log_nodes = log_nodes;
        Ok(nodes)
    }

    fn handle_event(&mut self, _surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if let UiEvent::InputSubmitted((id, value)) = event
            && Some(id) == self.input
        {
            self.log.push(format!("> {value}"));
            match Command::parse(&value) {
                Ok(command) => self.run(command),
                Err(error) => self.log.push(error),
            }
            self.tree
                .set_props(
                    id,
                    InputProps::new("").placeholder("Command, then Enter (help)"),
                )
                .map_err(|error| error.to_string())?;
            self.refresh()?;
        }
        Ok(self.tree.take_update())
    }

    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::TerminalReadDone((id, Ok(text))) => {
                self.log.push(format!("read #{id}: {} bytes", text.len()));
                self.log.push(text);
            }
            IoEvent::TerminalReadDone((id, Err(error))) => {
                self.log.push(format!("read #{id} failed: {error}"));
            }
            IoEvent::TerminalWriteDone((id, Ok(()))) => {
                self.log.push(format!("write #{id} typed"));
            }
            IoEvent::TerminalWriteDone((id, Err(error))) => {
                self.log.push(format!("write #{id} failed: {error}"));
            }
            IoEvent::ApprovalAnswered((id, answer)) => {
                self.log.push(format!("approval #{id}: {answer:?}"));
            }
            IoEvent::TerminalCreated(info) => self.log.push(format!(
                "created {}:{}:{}",
                info.handle,
                origin_name(info.origin),
                info.title
            )),
            IoEvent::TerminalClosed(handle) => {
                if Some(handle) == self.own_terminal {
                    self.own_terminal = None;
                }
                self.log.push(format!("closed {handle}"));
            }
            IoEvent::TerminalInput((handle, bytes)) if Some(handle) == self.own_terminal => {
                // Echo what is typed into the probe's own terminal.
                if let Err(error) = terminals::write_output(handle, &bytes) {
                    self.log
                        .push(format!("write-output failed: {}", error.message));
                }
                return Ok(());
            }
            _ => return Ok(()),
        }
        self.refresh()?;
        self.tree.push(PANEL).map_err(|error| error.message)
    }
}

register_extension!(CheckProbe);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse() {
        assert_eq!(Command::parse("list"), Ok(Command::List));
        assert_eq!(Command::parse("  "), Ok(Command::Help));
        assert_eq!(
            Command::parse("read 3"),
            Ok(Command::Read {
                handle: 3,
                scope: ReadKind::LastLines,
                lines: DEFAULT_READ_LINES
            })
        );
        assert_eq!(
            Command::parse("read 3 10"),
            Ok(Command::Read {
                handle: 3,
                scope: ReadKind::LastLines,
                lines: 10
            })
        );
        assert!(Command::parse("read 3 501").is_err());
        assert!(Command::parse("read x").is_err());
        assert_eq!(
            Command::parse("write 1 echo hi"),
            Ok(Command::Write {
                handle: 1,
                text: "echo hi".into(),
                submit: false
            })
        );
        assert_eq!(
            Command::parse("write-enter 1 echo  hi"),
            Ok(Command::Write {
                handle: 1,
                text: "echo  hi".into(),
                submit: true
            })
        );
        assert!(Command::parse("write 1").is_err());
        assert!(Command::parse("nope").is_err());
    }

    #[test]
    fn the_log_keeps_the_newest_lines() {
        let mut log = Log::default();
        for index in 0..LOG_LINES + 2 {
            log.push(format!("line {index}"));
        }
        assert_eq!(log.line(0), "line 2");
        assert_eq!(log.line(LOG_LINES - 1), format!("line {}", LOG_LINES + 1));
        assert_eq!(log.line(LOG_LINES), "");
    }

    #[test]
    fn submitting_a_command_runs_it_and_clears_the_input() {
        take_host_effects();
        let mut probe = CheckProbe::new();
        probe.mount(PANEL).expect("mount");
        let input = probe.input.expect("input");
        probe
            .handle_event(
                PANEL,
                UiEvent::InputSubmitted((input, "write 4 echo hi".into())),
            )
            .expect("event");
        let effects = take_host_effects();
        assert!(effects.iter().any(|effect| matches!(
            effect,
            HostEffect::Terminal(terminals::TerminalEffect::RequestWrite { handle: 4, text, submit: false, .. })
                if text == "echo hi"
        )));
        assert_eq!(
            probe.tree.props(input),
            Some(
                &InputProps::new("")
                    .placeholder("Command, then Enter (help)")
                    .into()
            )
        );
    }
}
