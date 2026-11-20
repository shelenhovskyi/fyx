//! Command blocks from shell integration (plan 5.17, research layer A item 3): the command line,
//! whether it carried the terminal's nonce, the exit code, and the output as plain text.
//!
//! The tap feeds the bytes between an OSC 633/133 `C` (executed) and `D` (finished) mark through a
//! `vte` parser of its own, keeping printed characters, tabs and line breaks and dropping every
//! escape sequence, so a T3 read of the last block (§C.3) is plain text without ANSI. Only the
//! output of a running command is parsed twice; the rest goes to alacritty alone.
//!
//! Any program can print the marks. Only Fyx's script knows the nonce (it takes it out of the
//! environment before the user's first command), so a block is [`CommandBlock::trusted`] only when
//! its command line came with the nonce: an untrusted command line is shown as the program's
//! claim, never as what the user typed.

use std::sync::atomic::{AtomicUsize, Ordering};

use alacritty_terminal::vte::{Parser, Perform};

use crate::osc::{OscEvent, ShellMark};

/// How many [`CommandBlockReader`]s are alive, in the whole process.
static READERS: AtomicUsize = AtomicUsize::new(0);

/// While one is alive, terminals record command blocks. Recording parses every running command's
/// output a second time on the IO thread, so with no reader (no loaded extension declares
/// `terminal:read`) the marks are still tracked but no output is kept. A block is recorded from
/// the next command that starts after the first reader appears.
#[derive(Debug)]
pub struct CommandBlockReader(());

impl CommandBlockReader {
    pub fn acquire() -> Self {
        READERS.fetch_add(1, Ordering::Relaxed);
        Self(())
    }
}

impl Drop for CommandBlockReader {
    fn drop(&mut self) {
        READERS.fetch_sub(1, Ordering::Relaxed);
    }
}

fn readers_alive() -> bool {
    READERS.load(Ordering::Relaxed) > 0
}

/// A block keeps the last this many bytes of its output.
pub const MAX_BLOCK_OUTPUT_BYTES: usize = 256 * 1024;

/// Where output starts again after an OSC that ended at `end` ([`crate::OscScanner::feed_at`]):
/// past the `\` of an `ESC \` terminator.
pub(crate) fn after_terminator(bytes: &[u8], end: usize) -> usize {
    if end > 0 && bytes[end - 1] == 0x1b && bytes.get(end) == Some(&b'\\') {
        end + 1
    } else {
        end
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandBlock {
    /// From OSC 633;E, unescaped.
    pub command_line: Option<String>,
    /// The command line came with this terminal's nonce.
    pub trusted: bool,
    pub exit_code: Option<i32>,
    /// Plain text: no escape sequences, no carriage-return overwrites.
    pub output: String,
    /// The start of the output was dropped to stay within [`MAX_BLOCK_OUTPUT_BYTES`].
    pub truncated: bool,
}

#[derive(Default)]
pub(crate) struct Blocks {
    /// The last command line and whether it had the nonce, until its `C`.
    pending_line: Option<(String, bool)>,
    running: Option<Running>,
    last: Option<CommandBlock>,
}

impl std::fmt::Debug for Blocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blocks")
            .field("recording", &self.running.is_some())
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

struct Running {
    block: CommandBlock,
    parser: Parser,
    text: PlainText,
}

impl Blocks {
    /// Whether output bytes matter now (a command is running).
    pub(crate) fn recording(&self) -> bool {
        self.running.is_some()
    }

    /// Output between two marks, in order.
    pub(crate) fn output(&mut self, bytes: &[u8]) {
        if let Some(running) = &mut self.running {
            running.parser.advance(&mut running.text, bytes);
        }
    }

    pub(crate) fn mark(&mut self, event: &OscEvent, nonce: &str) {
        match event {
            OscEvent::CommandLine { text, nonce: given } => {
                let trusted = !nonce.is_empty() && given.as_deref() == Some(nonce);
                self.pending_line = Some((text.clone(), trusted));
            }
            OscEvent::Shell { mark, .. } => match mark {
                ShellMark::CommandExecuted => {
                    self.finish(None);
                    let (command_line, trusted) = match self.pending_line.take() {
                        Some((line, trusted)) => (Some(line), trusted),
                        None => (None, false),
                    };
                    if !readers_alive() {
                        return;
                    }
                    self.running = Some(Running {
                        block: CommandBlock {
                            command_line,
                            trusted,
                            ..CommandBlock::default()
                        },
                        parser: Parser::new(),
                        text: PlainText::default(),
                    });
                }
                ShellMark::CommandFinished { exit_code } => self.finish(*exit_code),
                // A prompt without a `D` (the shell was interrupted): the block ends unknown.
                ShellMark::PromptStart => {
                    self.finish(None);
                    self.pending_line = None;
                }
                ShellMark::CommandStart => {}
            },
            _ => {}
        }
    }

    fn finish(&mut self, exit_code: Option<i32>) {
        let Some(Running {
            mut block, text, ..
        }) = self.running.take()
        else {
            return;
        };
        block.exit_code = exit_code;
        block.truncated = text.truncated;
        block.output = text.text.trim_end().trim_start_matches('\n').to_owned();
        self.last = Some(block);
    }

    pub(crate) fn last(&self) -> Option<CommandBlock> {
        self.last.clone()
    }
}

#[derive(Default)]
struct PlainText {
    text: String,
    line_start: usize,
    /// A carriage return without a line feed: the next character starts the line again.
    carriage: bool,
    truncated: bool,
}

impl PlainText {
    fn push(&mut self, char: char) {
        if self.carriage {
            self.text.truncate(self.line_start);
            self.carriage = false;
        }
        if self.text.len() + char.len_utf8() > MAX_BLOCK_OUTPUT_BYTES {
            self.drop_oldest();
        }
        self.text.push(char);
    }

    /// Drops the oldest quarter, at a line break when there is one.
    fn drop_oldest(&mut self) {
        let mut cut = MAX_BLOCK_OUTPUT_BYTES / 4;
        while !self.text.is_char_boundary(cut) {
            cut += 1;
        }
        let cut = self.text[cut..]
            .find('\n')
            .map_or(cut, |newline| cut + newline + 1);
        self.text.drain(..cut);
        self.line_start = self.line_start.saturating_sub(cut);
        self.truncated = true;
    }
}

impl Perform for PlainText {
    fn print(&mut self, char: char) {
        self.push(char);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.carriage = false;
                self.push('\n');
                self.line_start = self.text.len();
            }
            b'\r' => self.carriage = true,
            b'\t' => self.push('\t'),
            // Backspace moves back over one character of the current line.
            0x08 => {
                if self.text.len() > self.line_start {
                    self.text.pop();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc;

    fn feed(blocks: &mut Blocks, bytes: &[u8], nonce: &str) {
        let _reader = CommandBlockReader::acquire();
        // As the tap does: output up to each OSC's end, then the mark.
        let mut scanner = crate::OscScanner::new(crate::MAX_OSC_PAYLOAD);
        let mut last = 0;
        let mut marks = Vec::new();
        scanner.feed_at(bytes, |payload, end| marks.push((payload.to_vec(), end)));
        for (payload, end) in marks {
            blocks.output(&bytes[last..end]);
            last = after_terminator(bytes, end);
            if let Some(event) = osc::parse(&payload) {
                blocks.mark(&event, nonce);
            }
        }
        blocks.output(&bytes[last..]);
    }

    #[test]
    fn a_block_is_the_plain_output_between_executed_and_finished() {
        let mut blocks = Blocks::default();
        feed(
            &mut blocks,
            b"\x1b]633;A\x07$ \x1b]633;B\x07\x1b]633;E;ls\\x3b pwd;n0nce\x07\x1b]633;C\x1b\\\r\n\
              \x1b[1;34msrc\x1b[0m\tCargo.toml\r\nprogress 10%\rprogress 100%\r\n\
              \x1b]633;D;0\x07\x1b]633;A\x07$ ",
            "n0nce",
        );
        assert_eq!(
            blocks.last(),
            Some(CommandBlock {
                command_line: Some("ls; pwd".into()),
                trusted: true,
                exit_code: Some(0),
                output: "src\tCargo.toml\nprogress 100%".into(),
                truncated: false,
            })
        );
    }

    #[test]
    fn a_command_line_without_the_nonce_is_untrusted() {
        for line in [
            &b"\x1b]633;E;rm -rf ~\x07"[..],
            b"\x1b]633;E;rm -rf ~;guess\x07",
        ] {
            let mut blocks = Blocks::default();
            let mut bytes = line.to_vec();
            bytes.extend_from_slice(b"\x1b]133;C\x07gone\x1b]133;D;1\x07");
            feed(&mut blocks, &bytes, "n0nce");
            let block = blocks.last().unwrap();
            assert!(!block.trusted);
            assert_eq!(block.command_line.as_deref(), Some("rm -rf ~"));
            assert_eq!((block.output.as_str(), block.exit_code), ("gone", Some(1)));
        }
        // No nonce set: nothing is trusted, even a matching empty one.
        let mut blocks = Blocks::default();
        feed(
            &mut blocks,
            b"\x1b]633;E;ls;\x07\x1b]633;C\x07\x1b]633;D;0\x07",
            "",
        );
        assert!(!blocks.last().unwrap().trusted);
    }

    #[test]
    fn an_interrupted_block_ends_at_the_next_prompt_and_output_is_capped() {
        let mut blocks = Blocks::default();
        feed(&mut blocks, b"\x1b]633;C\x07", "n");
        assert!(blocks.recording());
        let line = "x".repeat(1023) + "\n";
        for _ in 0..300 {
            blocks.output(line.as_bytes());
        }
        feed(&mut blocks, b"^C\r\n\x1b]633;A\x07", "n");
        assert!(!blocks.recording());
        let block = blocks.last().unwrap();
        assert_eq!(block.exit_code, None);
        assert!(block.truncated);
        assert!(block.output.len() <= MAX_BLOCK_OUTPUT_BYTES);
        assert!(block.output.starts_with('x') && block.output.ends_with("^C"));
    }
}
