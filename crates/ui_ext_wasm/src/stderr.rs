//! A guest's stderr (triage-3, SEC T5): Rust guests print panics there. Instead of the host's
//! raw stderr, each line goes through `log` (escaped, like `log` calls) and into the Extension
//! Inspector, cut at [`MAX_STDERR_LINE`] bytes, and an instance writes at most
//! [`MAX_STDERR_BYTES`] in all; past that the rest is dropped with one notice. Writes never fail,
//! so a chatty guest is not trapped for printing.

use std::{
    io,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    task::{Context, Poll},
};

use bytes::Bytes;
use tokio::io::AsyncWrite;
use wasmtime_wasi::{
    cli::{IsTerminal, StdoutStream},
    p2::{OutputStream, Pollable, StreamResult},
};

use crate::diagnostics::DiagnosticsBuffer;

/// The longest line logged; the rest of a longer line is dropped.
pub const MAX_STDERR_LINE: usize = 1024;
/// What one instance may write to stderr in all.
pub const MAX_STDERR_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct GuestStderr(Arc<Mutex<Lines>>);

struct Lines {
    extension_id: Arc<str>,
    diagnostics: DiagnosticsBuffer,
    line: Vec<u8>,
    line_cut: bool,
    written: usize,
    exhausted: bool,
}

impl GuestStderr {
    pub(crate) fn new(extension_id: Arc<str>, diagnostics: DiagnosticsBuffer) -> Self {
        Self(Arc::new(Mutex::new(Lines {
            extension_id,
            diagnostics,
            line: Vec::new(),
            line_cut: false,
            written: 0,
            exhausted: false,
        })))
    }

    fn write_bytes(&self, bytes: &[u8]) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(bytes);
    }
}

impl Lines {
    fn write(&mut self, bytes: &[u8]) {
        if self.exhausted {
            return;
        }
        let room = MAX_STDERR_BYTES.saturating_sub(self.written);
        let (taken, over) = if bytes.len() > room {
            (&bytes[..room], true)
        } else {
            (bytes, false)
        };
        self.written += taken.len();
        for &byte in taken {
            if byte == b'\n' {
                self.flush_line();
            } else if self.line.len() < MAX_STDERR_LINE {
                self.line.push(byte);
            } else {
                self.line_cut = true;
            }
        }
        if over {
            self.flush_line();
            self.exhausted = true;
            self.emit(&format!(
                "more than {MAX_STDERR_BYTES} bytes on stderr; the rest is dropped"
            ));
        }
    }

    fn flush_line(&mut self) {
        if self.line.is_empty() && !self.line_cut {
            return;
        }
        let mut text = String::from_utf8_lossy(&self.line).into_owned();
        if self.line_cut {
            text.push_str("… (cut)");
        }
        self.line.clear();
        self.line_cut = false;
        self.emit(&text);
    }

    fn emit(&self, text: &str) {
        log::warn!(
            target: "fyx_ui",
            "{} stderr: {}",
            self.extension_id,
            ui_ext::sanitize_for_log(text)
        );
        self.diagnostics
            .log(log::Level::Warn, &format!("stderr: {text}"));
    }
}

impl Drop for Lines {
    /// A last line without a newline (a panic message as the instance dies) still shows.
    fn drop(&mut self) {
        self.flush_line();
    }
}

impl IsTerminal for GuestStderr {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for GuestStderr {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for GuestStderr {
    async fn ready(&mut self) {}
}

impl OutputStream for GuestStderr {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.write_bytes(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(64 * 1024)
    }
}

impl AsyncWrite for GuestStderr {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.write_bytes(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logged(stderr: &GuestStderr) -> Vec<String> {
        stderr
            .0
            .lock()
            .unwrap()
            .diagnostics
            .snapshot()
            .logs
            .iter()
            .map(|entry| entry.message.clone())
            .collect()
    }

    #[test]
    fn test_stderr_is_split_cut_and_capped() {
        let stderr = GuestStderr::new("fake".into(), DiagnosticsBuffer::default());
        stderr.write_bytes(b"panicked at src/lib.rs:3:\nboom");
        assert_eq!(logged(&stderr), ["stderr: panicked at src/lib.rs:3:"]);
        stderr.write_bytes(b"\n");
        assert_eq!(logged(&stderr)[1], "stderr: boom");

        let long = vec![b'x'; MAX_STDERR_LINE + 10];
        stderr.write_bytes(&long);
        stderr.write_bytes(b"\n");
        let cut = &logged(&stderr)[2];
        assert!(cut.ends_with("… (cut)"));
        assert_eq!(
            cut.len(),
            "stderr: ".len() + MAX_STDERR_LINE + "… (cut)".len()
        );

        stderr.write_bytes(&vec![b'y'; MAX_STDERR_BYTES]);
        let count = logged(&stderr).len();
        assert!(logged(&stderr)[count - 1].contains("the rest is dropped"));
        stderr.write_bytes(b"more\n");
        assert_eq!(logged(&stderr).len(), count, "nothing after the cap");
    }
}
