//! A streaming scanner that finds `ESC ] payload (BEL | ESC \)` in pty output.
//!
//! It mirrors how `vte` ends an OSC string (BEL, any ESC, CAN, SUB) so the tap and alacritty agree on
//! where a sequence ends, keeps state across reads, and never buffers more than [`MAX_OSC_PAYLOAD`]
//! bytes: longer sequences (OSC 52 clipboard, 1337 images) are skipped whole.

/// Longest OSC payload the tap looks at. OSC 7501 caps a whole sequence at 4096 bytes.
pub const MAX_OSC_PAYLOAD: usize = 8192;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    Osc,
    /// Inside an OSC that went over the size limit; discard until it ends.
    Overflow,
}

#[derive(Debug)]
pub struct OscScanner {
    state: State,
    payload: Vec<u8>,
    max_payload: usize,
    /// OSC sequences skipped because they were longer than the limit.
    pub overflowed: u64,
}

impl Default for OscScanner {
    fn default() -> Self {
        Self::new(MAX_OSC_PAYLOAD)
    }
}

impl OscScanner {
    pub fn new(max_payload: usize) -> Self {
        Self {
            state: State::Ground,
            payload: Vec::new(),
            max_payload,
            overflowed: 0,
        }
    }

    /// Scans `bytes`, calling `on_osc` with each complete payload (without `ESC ]` and terminator).
    pub fn feed(&mut self, bytes: &[u8], mut on_osc: impl FnMut(&[u8])) {
        self.feed_at(bytes, |payload, _| on_osc(payload));
    }

    /// Like [`OscScanner::feed`], with the offset in `bytes` just past each payload's terminator
    /// (for `ESC \`, past the `ESC`; the `\` that follows is not part of any output).
    pub fn feed_at(&mut self, bytes: &[u8], mut on_osc: impl FnMut(&[u8], usize)) {
        for (index, &byte) in bytes.iter().enumerate() {
            match self.state {
                State::Ground => {
                    if byte == ESC {
                        self.state = State::Escape;
                    }
                }
                State::Escape => {
                    self.state = match byte {
                        b']' => {
                            self.payload.clear();
                            State::Osc
                        }
                        ESC => State::Escape,
                        _ => State::Ground,
                    };
                }
                State::Osc => match byte {
                    BEL => {
                        on_osc(&self.payload, index + 1);
                        self.state = State::Ground;
                    }
                    // vte ends the OSC string on any ESC (`ESC \` is ST) and dispatches it.
                    ESC => {
                        on_osc(&self.payload, index + 1);
                        self.state = State::Escape;
                    }
                    CAN | SUB => self.state = State::Ground,
                    // vte ignores other C0 controls inside an OSC string.
                    0x00..=0x1f => {}
                    _ => {
                        if self.payload.len() >= self.max_payload {
                            self.payload.clear();
                            self.overflowed += 1;
                            self.state = State::Overflow;
                        } else {
                            self.payload.push(byte);
                        }
                    }
                },
                State::Overflow => match byte {
                    BEL | CAN | SUB => self.state = State::Ground,
                    ESC => self.state = State::Escape,
                    _ => {}
                },
            }
        }
        if self.state != State::Osc && self.payload.capacity() > self.max_payload {
            self.payload = Vec::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<String> {
        let mut scanner = OscScanner::new(16);
        let mut found = Vec::new();
        for chunk in chunks {
            scanner.feed(chunk, |payload| {
                found.push(String::from_utf8_lossy(payload).into_owned())
            });
        }
        found
    }

    #[test]
    fn finds_bel_and_st_terminated_sequences() {
        assert_eq!(
            scan(&[b"a\x1b]7501;?\x07b\x1b]7;file://h/x\x1b\\c"]),
            ["7501;?", "7;file://h/x"]
        );
    }

    #[test]
    fn keeps_state_across_reads() {
        assert_eq!(
            scan(&[b"\x1b", b"]75", b"01;?", b"\x1b", b"\\"]),
            ["7501;?"]
        );
    }

    #[test]
    fn skips_csi_and_cancelled_sequences() {
        assert_eq!(scan(&[b"\x1b[31m\x1b]9;x\x18\x1b]9;y\x07"]), ["9;y"]);
    }

    #[test]
    fn skips_sequences_over_the_limit() {
        let mut scanner = OscScanner::new(16);
        let mut found = Vec::new();
        scanner.feed(
            b"\x1b]52;c;AAAAAAAAAAAAAAAAAAAAAAAA\x07\x1b]9;ok\x07",
            |payload| found.push(payload.to_vec()),
        );
        assert_eq!(found, [b"9;ok".to_vec()]);
        assert_eq!(scanner.overflowed, 1);
    }

    #[test]
    fn escape_inside_osc_ends_it_like_vte() {
        assert_eq!(
            scan(&[b"\x1b]2;title\x1b]7501;?\x07"]),
            ["2;title", "7501;?"]
        );
    }
}
