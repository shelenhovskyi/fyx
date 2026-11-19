//! Fyx's terminal foundation (layer A of `docs/fyx/terminal-api/research.md`).
//!
//! Alacritty reads and parses the pty output on its own IO thread and drops the OSC sequences it does
//! not know (7, 9, 133, 633, 777, 1337, 7501, ...). Instead of patching the alacritty fork, Fyx wraps
//! the pty in [`TeePty`]: alacritty's `EventLoop` is generic over `EventedPty`, so the wrapper sees
//! every byte the event loop reads, scans it for OSC payloads and hands the bytes on unchanged.
//!
//! The only bytes the tap ever writes back to the pty are constant feature-detection replies (the
//! OSC 7501 support query); nothing in a reply is built from program output.

pub mod inbox;
pub mod osc;
mod osc_scan;
mod tee_pty;
mod terminal_id;

pub use osc::OscEvent;
pub use osc_scan::{MAX_OSC_PAYLOAD, OscScanner};
pub use tee_pty::{OscObserver, OscReplies, TeePty, tee, tee_with};
pub use terminal_id::{
    BLANKED_INHERITED_ENV, TERMINAL_ID_ENV, TERMINAL_TOKEN_ENV, TerminalExt, TerminalId,
    TerminalToken, terminal_for_token,
};

/// OSC 7501 "Program Status" (rev 0.4, superlogical.com/rex/docs/build/program-status).
pub mod program_status {
    /// Payload of the support query, `OSC 7501 ; ? ST`.
    pub const QUERY: &[u8] = b"7501;?";

    /// The reply to [`QUERY`]: support, with the default state and kind sets spelled out.
    pub const REPLY: &[u8] =
        b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

    /// Whether an OSC payload (without `ESC ]` and the terminator) is the support query.
    pub fn is_query(payload: &[u8]) -> bool {
        payload == QUERY
    }
}
