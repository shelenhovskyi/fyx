//! Fyx demo: a linter that publishes its own diagnostics (staged `fyx:ui` 0.4, plan 6.6,
//! `docs/fyx/wit-0.4/design.md` §E).
//!
//! The host cannot read the worktree for extensions, so the demo lints what the user selects:
//! with `context:editor` it watches the editor, and when a selection in a worktree file holds
//! text, every `TODO` in it becomes a warning on that file (`diagnostics:publish`). The editor
//! underlines them, the diagnostics panel lists them with the extension's name as their source,
//! and the status bar counts them. Selecting again in the same file replaces the file's entries;
//! a selection without a `TODO` clears them. Another file keeps its own.
//!
//! Panel `"lint"` (right dock, "TODO lint"): a status line, one row per published entry (a click
//! opens the file at the line, `workspace:open`), and a Clear button that clears every file.

use ui_ext_sdk::*;

#[cfg(feature = "fyx-ui-0-4")]
mod linter;
#[cfg(feature = "fyx-ui-0-4")]
pub use linter::LintDemo;

const MARKER: &str = "TODO";

/// A `TODO` found in a selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// 0-based, in the file.
    pub line: u32,
    /// UTF-8 bytes of the line, like every `text-range`.
    pub columns: std::ops::Range<u32>,
    pub message: String,
}

/// The `TODO`s of a selection that starts at 0-based `start_line` and `start_column` (UTF-8
/// bytes). A marker is a word of its own, so `TODOS` and `TODO_LIST` do not count.
pub fn scan(start_line: u32, start_column: u32, text: &str) -> Vec<Finding> {
    let is_word_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut findings = Vec::new();
    for (index, line_text) in text.split('\n').enumerate() {
        let Ok(offset) = u32::try_from(index) else {
            break;
        };
        let first_column = if index == 0 { start_column } else { 0 };
        let bytes = line_text.as_bytes();
        let found = line_text.match_indices(MARKER).find(|(at, _)| {
            let end = at + MARKER.len();
            !(*at > 0 && is_word_byte(bytes[at - 1]))
                && !bytes.get(end).copied().is_some_and(is_word_byte)
        });
        let Some((at, _)) = found else {
            continue;
        };
        let rest = line_text[at + MARKER.len()..]
            .trim_start_matches(|character: char| character == ':' || character.is_whitespace())
            .trim_end();
        let (Ok(at), Ok(length)) = (u32::try_from(at), u32::try_from(MARKER.len())) else {
            continue;
        };
        let column = first_column.saturating_add(at);
        findings.push(Finding {
            line: start_line.saturating_add(offset),
            columns: column..column.saturating_add(length),
            message: if rest.is_empty() {
                "TODO".to_string()
            } else {
                format!("TODO: {rest}")
            },
        });
    }
    findings
}

/// Exported as 0.3 or older the demo has no `context`, so it shows nothing.
#[cfg(not(feature = "fyx-ui-0-4"))]
pub struct LintDemo;

#[cfg(not(feature = "fyx-ui-0-4"))]
impl Extension for LintDemo {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        Vec::new()
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

// The demo needs the 0.4 `context` imports (`script/fyx-build-demos --fyx-ui-0-4`); the older
// worlds build the empty extension above.
#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(LintDemo, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(LintDemo, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(LintDemo);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_marks_todo_words_with_byte_columns() {
        let text = "fn a() {} // TODO: name it\nlet TODOS = 1;\n    TODO\n\u{e9}\u{e9} TODO_LIST TODO later";
        let findings = scan(10, 3, text);
        assert_eq!(
            findings,
            [
                Finding {
                    line: 10,
                    columns: 3 + 13..3 + 17,
                    message: "TODO: name it".into()
                },
                Finding {
                    line: 12,
                    columns: 4..8,
                    message: "TODO".into()
                },
                // Two 2-byte characters, a space and `TODO_LIST ` come first.
                Finding {
                    line: 13,
                    columns: 15..19,
                    message: "TODO: later".into()
                },
            ]
        );
    }
}
