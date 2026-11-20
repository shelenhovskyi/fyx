//! Fyx demo 11: a pseudo-terminal of the staged `fyx:ui` 0.4 (`docs/fyx/wit-0.4/design.md`
//! §B.2), a small shell written in WASM and drawn by Fyx's native terminal inside the panel. It
//! builds only for 0.4: other builds show a panel that says so.
//!
//! One panel `"pseudo-terminal"` (bottom dock, title "WASM shell", icon `terminal`):
//! - a status line with the terminal's size, from `terminal-resized`;
//! - a `terminal` node showing a pseudo-terminal opened `in-panel` when the panel first mounts;
//! - **Clear** clears the screen; **Busy** and **Idle** set the status Fyx draws on the terminal.
//!
//! The shell ([`Shell`]) has a prompt, line editing (backspace, Ctrl-C, Ctrl-L; other escape
//! sequences are ignored) and a few commands: `help`, `echo`, `count`, `size`, `colors`, `clear`.
//! It is pure, so its tests run without a host.

use ui_ext_sdk::*;

const PANEL: &str = "pseudo-terminal";

#[cfg(feature = "fyx-ui-0-4")]
pub use shell::Shell;

/// The shell: bytes in, bytes out.
#[cfg(feature = "fyx-ui-0-4")]
mod shell {
    pub const PROMPT: &str = "\x1b[1;32mwasm\x1b[0m$ ";
    const BANNER: &str = "\x1b[1mFyx WASM shell\x1b[0m: a pseudo-terminal whose backend is a \
                          fyx:ui 0.4 extension. Type \x1b[1mhelp\x1b[0m.\r\n";

    #[derive(Debug, Default)]
    pub struct Shell {
        line: String,
        /// Inside an escape sequence of the input, which the shell ignores.
        escape: Escape,
        commands: u32,
        size: Option<(u16, u16)>,
    }

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    enum Escape {
        #[default]
        None,
        Start,
        Csi,
    }

    impl Shell {
        /// The banner and the first prompt.
        pub fn start(&self) -> Vec<u8> {
            format!("{BANNER}{PROMPT}").into_bytes()
        }

        pub fn resize(&mut self, cols: u16, rows: u16) {
            self.size = Some((cols, rows));
        }

        pub fn size(&self) -> Option<(u16, u16)> {
            self.size
        }

        pub fn clear(&mut self) -> Vec<u8> {
            self.line.clear();
            format!("\x1b[2J\x1b[H{PROMPT}").into_bytes()
        }

        /// What the terminal shows for the user's keys.
        pub fn input(&mut self, bytes: &[u8]) -> Vec<u8> {
            let mut out = String::new();
            for ch in String::from_utf8_lossy(bytes).chars() {
                match (self.escape, ch) {
                    (Escape::Start, '[') => self.escape = Escape::Csi,
                    (Escape::Start, _) => self.escape = Escape::None,
                    (Escape::Csi, '\x40'..='\x7e') => self.escape = Escape::None,
                    (Escape::Csi, _) => {}
                    (Escape::None, '\x1b') => self.escape = Escape::Start,
                    (Escape::None, '\r' | '\n') => {
                        out.push_str("\r\n");
                        let line = std::mem::take(&mut self.line);
                        out.push_str(&self.run(line.trim()));
                        out.push_str(PROMPT);
                    }
                    (Escape::None, '\x7f' | '\x08') => {
                        if self.line.pop().is_some() {
                            out.push_str("\x08 \x08");
                        }
                    }
                    (Escape::None, '\x03') => {
                        self.line.clear();
                        out.push_str("^C\r\n");
                        out.push_str(PROMPT);
                    }
                    (Escape::None, '\x0c') => {
                        out.push_str(&String::from_utf8_lossy(&self.clear()));
                    }
                    (Escape::None, ch) if ch.is_control() => {}
                    (Escape::None, ch) => {
                        // A line long enough to be a paste of something else is cut off.
                        if self.line.len() < 1024 {
                            self.line.push(ch);
                            out.push(ch);
                        }
                    }
                }
            }
            out.into_bytes()
        }

        fn run(&mut self, line: &str) -> String {
            let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
            if command.is_empty() {
                return String::new();
            }
            self.commands += 1;
            match command {
                "help" => "commands: help, echo <text>, count, size, colors, clear\r\n".into(),
                "echo" => format!("{rest}\r\n"),
                "count" => format!("{} commands so far\r\n", self.commands),
                "size" => match self.size {
                    Some((cols, rows)) => format!("{cols} columns, {rows} rows\r\n"),
                    None => "the terminal has not reported its size yet\r\n".into(),
                },
                "colors" => {
                    let mut out = String::new();
                    for color in 30..38 {
                        out.push_str(&format!("\x1b[{color}m██\x1b[0m"));
                    }
                    out.push_str("\r\n");
                    out
                }
                "clear" => "\x1b[2J\x1b[H".into(),
                other => format!("{other}: command not found\r\n"),
            }
        }
    }
}

#[cfg(feature = "fyx-ui-0-4")]
pub struct PseudoTerminalDemo {
    tree: SurfaceTree,
    shell: Shell,
    terminal: Option<terminals::TerminalHandle>,
    error: Option<String>,
    status_line: Option<NodeId>,
    clear: Option<NodeId>,
    busy: Option<NodeId>,
    idle: Option<NodeId>,
}

#[cfg(feature = "fyx-ui-0-4")]
impl PseudoTerminalDemo {
    fn status_text(&self) -> String {
        if let Some(error) = &self.error {
            return format!("No terminal: {error}");
        }
        match self.shell.size() {
            Some((cols, rows)) => format!("WASM shell, {cols}×{rows}"),
            None => "WASM shell".into(),
        }
    }

    /// The pseudo-terminal, opened on the first mount; it outlives remounts.
    fn ensure_terminal(&mut self) {
        if self.terminal.is_some() || self.error.is_some() {
            return;
        }
        let request = terminals::PseudoRequest::new("WASM shell").in_panel();
        match terminals::open_pseudo(&request) {
            Ok(handle) => {
                self.terminal = Some(handle);
                self.write(&self.shell.start());
            }
            Err(error) => self.error = Some(error.message),
        }
    }

    fn write(&self, bytes: &[u8]) {
        let Some(handle) = self.terminal else {
            return;
        };
        // At most 64 KiB per call.
        for chunk in bytes.chunks(64 * 1024) {
            if let Err(error) = terminals::write_output(handle, chunk) {
                log(
                    LogLevel::Warn,
                    &format!("write-output failed: {}", error.message),
                );
                return;
            }
        }
    }

    fn set_status(&self, status: Option<terminals::TerminalStatus>) {
        if let Some(handle) = self.terminal
            && let Err(error) = terminals::set_status(handle, status.as_ref())
        {
            log(
                LogLevel::Warn,
                &format!("set-status failed: {}", error.message),
            );
        }
    }

    fn refresh_status_line(&mut self) -> Result<(), String> {
        if let Some(id) = self.status_line {
            let text = self.status_text();
            self.tree
                .set_props(id, TextProps::new(text).color(ColorToken::TextMuted))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[cfg(feature = "fyx-ui-0-4")]
impl Extension for PseudoTerminalDemo {
    fn new() -> Self {
        Self {
            tree: SurfaceTree::default(),
            shell: Shell::default(),
            terminal: None,
            error: None,
            status_line: None,
            clear: None,
            busy: None,
            idle: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "WASM shell",
            DockPosition::Bottom,
            Icon::Terminal,
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        self.ensure_terminal();
        let status = self.status_text();
        let terminal = self.terminal;
        let mut ids = (None, None, None, None);
        let nodes = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(4).padding(4), |tree| {
                tree.flex(FlexProps::row().gap(8), |tree| {
                    ids.0 = Some(tree.text(TextProps::new(status).color(ColorToken::TextMuted)));
                    ids.1 = Some(tree.button(ButtonProps::new("Clear")));
                    ids.2 = Some(tree.button(ButtonProps::new("Busy")));
                    ids.3 = Some(tree.button(ButtonProps::new("Idle")));
                });
                if let Some(handle) = terminal {
                    tree.terminal(handle);
                }
            });
        })?;
        (self.status_line, self.clear, self.busy, self.idle) = ids;
        Ok(nodes)
    }

    fn handle_event(&mut self, _surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if let UiEvent::Click(id) = event {
            let id = Some(id);
            if id == self.clear {
                let bytes = self.shell.clear();
                self.write(&bytes);
            } else if id == self.busy {
                self.set_status(Some(
                    terminals::TerminalStatus::new(terminals::StatusState::Working, "Busy")
                        .detail("set by the Busy button"),
                ));
            } else if id == self.idle {
                self.set_status(None);
            }
        }
        Ok(self.tree.take_update())
    }

    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::TerminalInput((handle, bytes)) if Some(handle) == self.terminal => {
                let out = self.shell.input(&bytes);
                self.write(&out);
            }
            IoEvent::TerminalResized((handle, cols, rows)) if Some(handle) == self.terminal => {
                self.shell.resize(cols, rows);
                self.refresh_status_line()?;
                self.tree.push(PANEL).map_err(|error| error.message)?;
            }
            // Settings the user changed (none are declared by this demo itself; a package that
            // adds `[[fyx_ui.settings]]` sees them here).
            IoEvent::SettingsChanged(keys) => {
                for key in keys {
                    let value = settings::get(&key).map_err(|error| error.message)?;
                    log(LogLevel::Info, &format!("setting {key} = {value:?}"));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// What a build for an older world shows: pseudo-terminals need 0.4.
#[cfg(not(feature = "fyx-ui-0-4"))]
pub struct NeedsTerminals;

#[cfg(not(feature = "fyx-ui-0-4"))]
pub const NEEDS_0_4: &str = "This demo needs fyx:ui 0.4. Build it with --features \
                             fyx_demo_pseudo_terminal/fyx-ui-0-4.";

#[cfg(not(feature = "fyx-ui-0-4"))]
impl Extension for NeedsTerminals {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "WASM shell",
            DockPosition::Bottom,
            Icon::Terminal,
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let mut ids = IdAllocator::new();
        let mut tree = TreeBuilder::new(&mut ids);
        tree.flex(FlexProps::column().gap(8).padding(8), |tree| {
            tree.text(TextProps::new(NEEDS_0_4).color(ColorToken::TextMuted));
        });
        Ok(tree.finish())
    }
}

#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3", feature = "fyx-ui-0-4")))]
register_extension!(NeedsTerminals, v0_1);
#[cfg(all(
    feature = "fyx-ui-0-2",
    not(any(feature = "fyx-ui-0-3", feature = "fyx-ui-0-4"))
))]
register_extension!(NeedsTerminals, v0_2);
#[cfg(all(feature = "fyx-ui-0-3", not(feature = "fyx-ui-0-4")))]
register_extension!(NeedsTerminals);
#[cfg(feature = "fyx-ui-0-4")]
register_extension!(PseudoTerminalDemo);

#[cfg(all(test, feature = "fyx-ui-0-4"))]
mod tests {
    use super::shell::PROMPT;
    use super::*;

    fn typed(shell: &mut Shell, keys: &str) -> String {
        String::from_utf8(shell.input(keys.as_bytes())).unwrap()
    }

    #[test]
    fn the_shell_echoes_edits_and_runs_commands() {
        let mut shell = Shell::default();
        assert!(String::from_utf8(shell.start()).unwrap().ends_with(PROMPT));
        assert_eq!(typed(&mut shell, "ecx"), "ecx");
        assert_eq!(typed(&mut shell, "\x7f"), "\x08 \x08");
        assert_eq!(
            typed(&mut shell, "ho hi\r"),
            format!("ho hi\r\nhi\r\n{PROMPT}")
        );
        assert_eq!(
            typed(&mut shell, "nope\r"),
            format!("nope\r\nnope: command not found\r\n{PROMPT}")
        );
        // Arrow keys are escape sequences: ignored.
        assert_eq!(typed(&mut shell, "\x1b[A\x1b[D"), "");
        shell.resize(80, 24);
        assert!(typed(&mut shell, "size\r").contains("80 columns, 24 rows"));
        assert!(typed(&mut shell, "count\r").contains("4 commands so far"));
        assert_eq!(typed(&mut shell, "abc\x03"), format!("abc^C\r\n{PROMPT}"));
    }

    #[test]
    fn mount_opens_one_pseudo_terminal_in_the_panel() {
        take_host_effects();
        let mut demo = PseudoTerminalDemo::new();
        let nodes = demo.mount(PANEL).expect("mount");
        let handle = demo.terminal.expect("opened");
        assert!(
            nodes
                .iter()
                .any(|node| node.props == Props::Terminal(TerminalProps { handle }))
        );
        demo.mount(PANEL).expect("remount");
        let opened = take_host_effects()
            .into_iter()
            .filter(|effect| {
                matches!(
                    effect,
                    HostEffect::Terminal(terminals::TerminalEffect::OpenPseudo { .. })
                )
            })
            .count();
        assert_eq!(opened, 1);
    }
}
