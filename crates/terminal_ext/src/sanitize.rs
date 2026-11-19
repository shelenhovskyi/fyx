//! Pseudo-terminal output sanitising (`docs/fyx/wit-0.4/design.md` §B.2).
//!
//! An extension writes a pseudo-terminal's output, and the extension also receives the
//! terminal's input. A terminal answers some sequences by writing to its input (device
//! attributes, cursor position, mode and colour reports, the clipboard), so an extension could
//! read state back or make the terminal "type" for it (the CVE class of research §4). Upstream
//! never writes those answers for a pseudo-terminal (a `fyx:` hook in `crates/terminal`), and this
//! filter removes the questions before alacritty sees them.
//!
//! The filter parses with the same `vte` parser alacritty uses, so both see the same sequences,
//! and writes back only what it allows, re-encoded from the parsed form. Anything it does not
//! know how to re-encode is dropped: DCS, SOS, PM and APC strings, OSC commands outside a short
//! allow list, sequences vte marked as ignored.

use alacritty_terminal::vte::{Params, Parser, Perform};

/// OSC commands that pass: titles and colours (queries removed). Everything else (8 hyperlinks,
/// 52 clipboard, 1337, 7501, 133, ...) is dropped.
const ALLOWED_OSC: &[&[u8]] = &[
    b"0", b"1", b"2", b"4", b"10", b"11", b"12", b"104", b"110", b"111", b"112",
];

/// A streaming filter for one pseudo-terminal; keeps parser state between writes.
#[derive(Default)]
pub struct OutputSanitizer {
    parser: Parser,
}

impl OutputSanitizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The allowed part of `bytes`. A sequence split across calls comes out whole with the call
    /// that completes it.
    pub fn sanitize(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut out = Encoder {
            out: Vec::with_capacity(bytes.len()),
        };
        self.parser.advance(&mut out, bytes);
        out.out
    }
}

struct Encoder {
    out: Vec<u8>,
}

/// CSI sequences that make the terminal answer: DA1-3 (`c`), DSR and CPR (`n`), DECRQM (`$p`),
/// window reports (`t`), the kitty keyboard query (`?u`), the modifyOtherKeys query (`?m`),
/// XTVERSION (`>q`), DECREQTPARM (`x`) and XTSMGRAPHICS (`?S`).
fn csi_answers(intermediates: &[u8], action: char) -> bool {
    let private = intermediates.first().copied();
    match action {
        'c' | 'n' | 't' | 'x' => true,
        'p' => intermediates.contains(&b'$'),
        'u' | 'm' | 'S' => private == Some(b'?'),
        'q' => private == Some(b'>'),
        _ => false,
    }
}

fn osc_allowed(params: &[&[u8]]) -> bool {
    let Some(command) = params.first() else {
        return false;
    };
    // `?` anywhere asks the terminal to report a colour.
    ALLOWED_OSC.contains(command) && !params[1..].iter().any(|param| *param == b"?")
}

impl Encoder {
    fn push_char(&mut self, c: char) {
        let mut buffer = [0; 4];
        self.out
            .extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
    }

    fn push_number(&mut self, value: u16) {
        self.out.extend_from_slice(value.to_string().as_bytes());
    }
}

impl Perform for Encoder {
    fn print(&mut self, c: char) {
        self.push_char(c);
    }

    fn execute(&mut self, byte: u8) {
        // C0 as is; C1 (reached as UTF-8 or as a lone byte) as UTF-8, which vte executes the same.
        if byte < 0x80 {
            self.out.push(byte);
        } else {
            self.push_char(char::from(byte));
        }
    }

    // DCS strings (DECRQSS, XTGETTCAP, sixel, tmux passthrough) are dropped whole.
    fn hook(&mut self, _: &Params, _: &[u8], _: bool, _: char) {}
    fn put(&mut self, _: u8) {}
    fn unhook(&mut self) {}

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if !osc_allowed(params) {
            return;
        }
        self.out.extend_from_slice(b"\x1b]");
        for (index, param) in params.iter().enumerate() {
            if index > 0 {
                self.out.push(b';');
            }
            self.out.extend_from_slice(param);
        }
        self.out.extend_from_slice(b"\x1b\\");
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore || csi_answers(intermediates, action) {
            return;
        }
        // vte collects a private marker (`<=>?`) before the parameters and other intermediates
        // (` !"#$%&'()*+,-./`) after them, in one list.
        let (private, trailing): (Vec<u8>, Vec<u8>) = intermediates
            .iter()
            .partition(|byte| (0x3C..=0x3F).contains(*byte));
        self.out.extend_from_slice(b"\x1b[");
        self.out.extend_from_slice(&private);
        for (index, group) in params.iter().enumerate() {
            if index > 0 {
                self.out.push(b';');
            }
            for (sub, value) in group.iter().enumerate() {
                if sub > 0 {
                    self.out.push(b':');
                }
                self.push_number(*value);
            }
        }
        self.out.extend_from_slice(&trailing);
        self.push_char(action);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        // DECID (`ESC Z`) answers like DA1. A string terminator (`ESC \`) arrives here after
        // the string it ended; the re-encoded OSC carries its own.
        if ignore || (intermediates.is_empty() && matches!(byte, b'Z' | b'\\')) {
            return;
        }
        self.out.push(0x1b);
        self.out.extend_from_slice(intermediates);
        self.out.push(byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(input: &[u8]) -> Vec<u8> {
        OutputSanitizer::new().sanitize(input)
    }

    #[test]
    fn text_and_ordinary_sequences_pass() {
        let input =
            "héllo \x1b[1;31mred\x1b[0m\r\n\x1b[?25l\x1b[2 q\x1b[38:2::1:2:3m\x1b(B\x1b[?1049h";
        let output = clean(input.as_bytes());
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "héllo \x1b[1;31mred\x1b[0m\r\n\x1b[?25l\x1b[2 q\x1b[38:2:0:1:2:3m\x1b(B\x1b[?1049h"
        );
        assert_eq!(clean(b"\x1b]0;my title\x07"), b"\x1b]0;my title\x1b\\");
        assert_eq!(
            clean(b"\x1b]11;rgb:00/00/00\x1b\\"),
            b"\x1b]11;rgb:00/00/00\x1b\\"
        );
    }

    #[test]
    fn questions_and_clipboard_are_removed() {
        for question in [
            &b"\x1b[c"[..],
            b"\x1b[0c",
            b"\x1b[>c",
            b"\x1b[=c",
            b"\x1b[5n",
            b"\x1b[6n",
            b"\x1b[?6n",
            b"\x1b[4$p",
            b"\x1b[?2004$p",
            b"\x1b[14t",
            b"\x1b[18t",
            b"\x1b[?u",
            b"\x1b[?4m",
            b"\x1b[>q",
            b"\x1bZ",
            b"\x1bP$qm\x1b\\",
            b"\x1bP+q544e\x1b\\",
            b"\x1b]10;?\x07",
            b"\x1b]4;1;?\x1b\\",
            b"\x1b]52;c;aGVsbG8=\x07",
            b"\x1b]52;c;?\x07",
            b"\x1b]8;;https://example.com\x1b\\",
            b"\x1b]1337;SetUserVar=a=Yg==\x07",
            b"\x1b]7501;?\x1b\\",
            b"\x1b_Gf=24;AAAA\x1b\\",
        ] {
            let mut input = b"a".to_vec();
            input.extend_from_slice(question);
            input.push(b'b');
            assert_eq!(
                clean(&input),
                b"ab",
                "{:?} passed",
                String::from_utf8_lossy(question)
            );
        }
    }

    #[test]
    fn sequences_split_across_writes() {
        let mut sanitizer = OutputSanitizer::new();
        let mut output = Vec::new();
        for byte in b"x\x1b[6ny\x1b]52;c;aGk=\x07z\x1b[1mw".iter() {
            output.extend(sanitizer.sanitize(&[*byte]));
        }
        output.extend(sanitizer.sanitize("é".as_bytes().split_at(1).0));
        output.extend(sanitizer.sanitize("é".as_bytes().split_at(1).1));
        assert_eq!(String::from_utf8(output).unwrap(), "xyz\x1b[1mwé");
    }

    #[test]
    fn c1_controls_cannot_open_a_sequence() {
        // vte executes C1 bytes; they never start a CSI, so `6n` stays text.
        let output = clean(b"\xc2\x9b6n");
        assert_eq!(output, "\u{9b}6n".as_bytes());
        assert_eq!(clean(b"\x9b6n"), "\u{9b}6n".as_bytes());
    }

    /// What alacritty answers when fed the sanitised output: nothing.
    #[test]
    fn alacritty_never_answers_sanitised_output() {
        use alacritty_terminal::event::{Event, EventListener};
        use alacritty_terminal::term::{Config, Term, test::TermSize};
        use alacritty_terminal::vte::ansi::Processor;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Replies(Arc<Mutex<Vec<String>>>);
        impl EventListener for Replies {
            fn send_event(&self, event: Event) {
                let reply = match event {
                    Event::PtyWrite(text) => text,
                    Event::ClipboardStore(..) => "clipboard store".into(),
                    Event::ClipboardLoad(..) => "clipboard load".into(),
                    Event::ColorRequest(..) => "colour".into(),
                    Event::TextAreaSizeRequest(..) => "size".into(),
                    _ => return,
                };
                self.0.lock().unwrap().push(reply);
            }
        }

        let questions: &[u8] =
            b"\x1b[c\x1b[>c\x1b[6n\x1b[5n\x1b[?6n\x1b[?2004$p\x1b[4$p\x1b[14t\x1b[18t\
            \x1b[?u\x1b[?4m\x1bZ\x1b]10;?\x07\x1b]11;?\x07\x1b]4;1;?\x07\x1b]52;c;?\x07\
            \x1b]52;c;aGk=\x07\x1bP$qm\x1b\\";
        let feed = |bytes: &[u8]| {
            let replies = Replies::default();
            let config = Config {
                osc52: alacritty_terminal::term::Osc52::CopyPaste,
                ..Config::default()
            };
            let mut term = Term::new(config, &TermSize::new(80, 24), replies.clone());
            let mut processor: Processor = Processor::new();
            processor.advance(&mut term, bytes);
            let replies = replies.0.lock().unwrap().clone();
            replies
        };
        // The questions do get answered when not sanitised, so the test can fail.
        assert!(feed(questions).len() >= 10);
        assert_eq!(feed(&clean(questions)), Vec::<String>::new());
    }
}
