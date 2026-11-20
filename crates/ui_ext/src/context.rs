//! fyx:ui 0.4 editor context, opening files and diagnostics (`context:editor`, `workspace:open`,
//! `workspace:diagnostics`, design wit-0.4 §B.11, plan 5.13): the model the runtime and the
//! editor host share, and the pure conversion between a tool's absolute paths and `file-ref`s.
//!
//! A tool's IDE protocol speaks absolute paths and `file://` URIs; the WIT speaks `file-ref`.
//! [`parse_tool_path`] accepts what tools send on every platform (Claude Code on Windows sends
//! both `file:///C:/…` and `file://C:\…`), and [`file_ref_in`] finds the deepest worktree that
//! holds the path, so the extension never learns a path outside the worktrees.

use std::{path::Path, sync::Arc};

use crate::{edit::TextRange, process::FileRef};

pub use crate::integration::{
    CAPABILITY_CONTEXT_EDITOR, CAPABILITY_WORKSPACE_DIAGNOSTICS, CAPABILITY_WORKSPACE_OPEN,
};

/// The selection text `context-changed` carries at most; longer text is cut at a character
/// boundary.
pub const MAX_SELECTION_BYTES: usize = 64 << 10;
/// What a tool may send as one absolute path or URI.
pub const MAX_TOOL_PATH_BYTES: usize = 4096;
/// §B.11: `focus = true` without a gesture needs a listener request at most this old, while one
/// of the extension's terminals has focus.
pub const FOCUS_REQUEST_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

/// `context.editor-context`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorContext {
    pub active: Option<FileRef>,
    /// Most recently focused first, `active` included.
    pub open: Vec<FileRef>,
    /// The text is absent for a private file, and at most [`MAX_SELECTION_BYTES`].
    pub selection: Option<(TextRange, Option<String>)>,
    /// Whether `active`'s worktree is trusted; `None` without an active file.
    pub trusted: Option<bool>,
}

/// Where the host sends an extension's context: at once when watching starts, then each change,
/// debounced 300 ms. Never blocks.
pub type ContextSink = Arc<dyn Fn(EditorContext) + Send + Sync>;

/// The editor side of `context` (implemented in `ui_ext_host`). Only worktree files appear in
/// the context; the runtime checks the capabilities, the paths and the focus rule first.
pub trait ContextHost: Send + Sync + 'static {
    /// Starts (`Some`) or stops (`None`) the extension's feed; a new sink replaces the old.
    fn watch(&self, extension_id: &str, sink: Option<ContextSink>);
    /// Opens a worktree file at `range`, in a background tab unless `focus`. The file exists
    /// in a worktree of the window; the error is shown to the extension as `unavailable`.
    fn open(
        &self,
        extension_id: &str,
        path: FileRef,
        range: Option<TextRange>,
        focus: bool,
    ) -> Result<(), String>;
    /// Zed's `private_files` (`is_private_path`): such a file never shows its text and never
    /// becomes or comes from a tool's absolute path.
    fn is_private(&self, absolute: &Path) -> bool;
    /// Whether one of the extension's terminals has focus now (the §B.11 focus rule).
    fn terminal_focused(&self, _extension_id: &str) -> bool {
        false
    }
}

/// An absolute path in a platform-neutral form: a drive (`C:`, upper case) or `None` for a
/// POSIX root, and the normal components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolPath {
    pub drive: Option<char>,
    pub components: Vec<String>,
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// What a tool sends as a file: `file:///C:/x`, `file://C:\x`, `file:///home/x`,
/// `file://localhost/home/x`, `C:\x`, `C:/x` or `/home/x`. `None` for anything relative, a
/// UNC or remote host, a `..` component, a control character or more than
/// [`MAX_TOOL_PATH_BYTES`].
pub fn parse_tool_path(input: &str) -> Option<ToolPath> {
    if input.len() > MAX_TOOL_PATH_BYTES || input.chars().any(char::is_control) {
        return None;
    }
    let scheme = input.get(..7).filter(|s| s.eq_ignore_ascii_case("file://"));
    let path = match scheme {
        Some(_) => {
            let rest = percent_decode(&input[7..])?;
            let rest = match rest.get(..10) {
                Some(host) if host.eq_ignore_ascii_case("localhost/") => rest[9..].to_string(),
                _ => rest,
            };
            // `file:///C:/x` has the drive after the root; `file://C:\x` in the host position.
            match rest.as_bytes() {
                [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => rest[1..].to_string(),
                [drive, b':', ..] if drive.is_ascii_alphabetic() => rest,
                [b'/', ..] => rest,
                _ => return None,
            }
        }
        None => input.to_string(),
    };
    let path = path.replace('\\', "/");
    let (drive, rest) = match path.as_bytes() {
        [drive, b':', b'/', ..] if drive.is_ascii_alphabetic() => {
            (Some(drive.to_ascii_uppercase() as char), &path[3..])
        }
        // `//server/share` is a UNC path.
        [b'/', b'/', ..] => return None,
        [b'/', ..] => (None, &path[1..]),
        _ => return None,
    };
    let mut components = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part if part.contains(':') => return None,
            part => components.push(part.to_string()),
        }
    }
    Some(ToolPath { drive, components })
}

/// A host path (a worktree root) as a [`ToolPath`]; Windows' `\\?\` verbatim prefix is
/// dropped.
pub fn host_tool_path(path: &Path) -> Option<ToolPath> {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\")
        .or_else(|| text.strip_prefix("//?/"))
        .unwrap_or(&text);
    parse_tool_path(text)
}

/// The `file-ref` of `path` in the deepest of `roots` (worktree id, root) that holds it. A drive
/// path compares without ASCII case, as Windows does.
pub fn file_ref_in<'a>(
    path: &ToolPath,
    roots: impl IntoIterator<Item = (u64, &'a Path)>,
) -> Option<FileRef> {
    let mut best: Option<(usize, u64)> = None;
    for (id, root) in roots {
        let Some(root) = host_tool_path(root) else {
            continue;
        };
        if root.drive != path.drive || root.components.len() > path.components.len() {
            continue;
        }
        let same = |a: &String, b: &String| {
            if path.drive.is_some() {
                a.eq_ignore_ascii_case(b)
            } else {
                a == b
            }
        };
        let inside = root
            .components
            .iter()
            .zip(&path.components)
            .all(|(a, b)| same(a, b));
        if inside && best.is_none_or(|(depth, _)| root.components.len() > depth) {
            best = Some((root.components.len(), id));
        }
    }
    let (depth, id) = best?;
    Some(FileRef {
        worktree: Some(id),
        path: path.components[depth..].join("/"),
    })
}

/// Cuts selection text to [`MAX_SELECTION_BYTES`] at a character boundary.
pub fn cap_selection(text: &mut String) {
    cut_to(text, MAX_SELECTION_BYTES);
}

/// Cuts `text` to at most `max` bytes at a character boundary.
pub(crate) fn cut_to(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// Entries one `context.diagnostics` answer holds at most (§B.11); errors are kept first.
pub const MAX_DIAGNOSTICS: usize = 500;
/// A diagnostic's message is cut at a character boundary at this length.
pub const MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 2 << 10;
/// A diagnostic's `source` and `code`, each. Not in the design: a bound on what a language
/// server may pass through.
pub const MAX_DIAGNOSTIC_LABEL_BYTES: usize = 256;
/// Files one `diagnostics-changed` names at most; a larger change sends an empty list
/// ("anything may have changed").
pub const MAX_CHANGED_DIAGNOSTIC_FILES: usize = 200;
/// `diagnostics-changed` is sent this long after the first change it reports.
pub const DIAGNOSTICS_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);

/// `context.severity`, most severe first: the order of a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// `context.diagnostic`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Zero-based lines; columns in UTF-8 bytes of the line, like every `text-range`.
    pub range: TextRange,
    pub severity: DiagnosticSeverity,
    /// At most [`MAX_DIAGNOSTIC_MESSAGE_BYTES`] in a snapshot.
    pub message: String,
    /// The language server, such as `rust-analyzer`.
    pub source: Option<String>,
    /// Numeric codes as text.
    pub code: Option<String>,
}

/// `context.file-diagnostics`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiagnostics {
    pub path: FileRef,
    pub diagnostics: Vec<Diagnostic>,
}

/// `context.diagnostics-snapshot`: only files with at least one entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagnosticsSnapshot {
    pub files: Vec<FileDiagnostics>,
    /// More than [`MAX_DIAGNOSTICS`] entries were found; the least severe were left out.
    pub truncated: bool,
}

/// Where the host says which files' diagnostics changed, while an extension watches. It may be
/// called on every change, undebounced, from any thread; the runtime debounces, drops files that
/// are private or in no worktree, and caps the list. An empty list means "anything may have
/// changed" (a language server restarted). Never blocks.
pub type DiagnosticsSink = Arc<dyn Fn(Vec<FileRef>) + Send + Sync>;

/// The language-server side of `context.diagnostics` and `watch-diagnostics` (implemented in
/// `ui_ext_host` over the project's diagnostics). The runtime checks `workspace:diagnostics`,
/// resolves and checks the path, leaves private files out, sorts, caps and cuts.
pub trait DiagnosticsSource: Send + Sync + 'static {
    /// What the window's language servers report now, open buffers or not: for `Some(path)`
    /// (its `worktree` always named) that one file, for `None` every file of the window's
    /// worktrees that has diagnostics. Each returned `FileRef` names its worktree. Ranges are
    /// zero-based with UTF-8 byte columns (converted from the servers' UTF-16). Any order, uncut.
    fn diagnostics(&self, extension_id: &str, path: Option<&FileRef>) -> Vec<FileDiagnostics>;
    /// Starts (`Some`) or stops (`None`) the extension's change feed; a new sink replaces the
    /// old.
    fn watch(&self, extension_id: &str, sink: Option<DiagnosticsSink>);
    /// Zed's `private_files` (`is_private_path`), as [`ContextHost::is_private`].
    fn is_private(&self, absolute: &Path) -> bool;
}

/// A snapshot of entries that passed the path checks: empty files dropped, entries sorted errors
/// first, then by path and position, at most [`MAX_DIAGNOSTICS`], texts cut, and the files in
/// the order of their first entry.
pub fn diagnostics_snapshot(files: Vec<FileDiagnostics>) -> DiagnosticsSnapshot {
    let mut paths = Vec::new();
    let mut entries: Vec<(usize, Diagnostic)> = Vec::new();
    for file in files {
        if file.diagnostics.is_empty() {
            continue;
        }
        let index = paths.len();
        paths.push(file.path);
        entries.extend(file.diagnostics.into_iter().map(|entry| (index, entry)));
    }
    let key = |(file, entry): &(usize, Diagnostic)| {
        let path: &FileRef = &paths[*file];
        (
            entry.severity,
            path.worktree,
            path.path.clone(),
            entry.range.start,
            entry.range.end,
        )
    };
    entries.sort_by_cached_key(key);
    let truncated = entries.len() > MAX_DIAGNOSTICS;
    entries.truncate(MAX_DIAGNOSTICS);
    let mut slots: Vec<Option<usize>> = vec![None; paths.len()];
    let mut out: Vec<FileDiagnostics> = Vec::new();
    for (file, mut entry) in entries {
        cut_to(&mut entry.message, MAX_DIAGNOSTIC_MESSAGE_BYTES);
        for label in [&mut entry.source, &mut entry.code].into_iter().flatten() {
            cut_to(label, MAX_DIAGNOSTIC_LABEL_BYTES);
        }
        let slot = *slots[file].get_or_insert_with(|| {
            out.push(FileDiagnostics {
                path: paths[file].clone(),
                diagnostics: Vec::new(),
            });
            out.len() - 1
        });
        out[slot].diagnostics.push(entry);
    }
    DiagnosticsSnapshot {
        files: out,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(components: &[&str]) -> Option<ToolPath> {
        Some(ToolPath {
            drive: Some('C'),
            components: components.iter().map(|c| c.to_string()).collect(),
        })
    }

    #[test]
    fn tool_paths() {
        // What Claude Code 2.1.326 sends on Windows, both ways, and plain Windows paths.
        assert_eq!(
            parse_tool_path("file:///C:/src/app/main.rs"),
            drive(&["src", "app", "main.rs"])
        );
        assert_eq!(
            parse_tool_path(r"file://C:\src\app\main.rs"),
            drive(&["src", "app", "main.rs"])
        );
        assert_eq!(
            parse_tool_path(r"C:\src\app\main.rs"),
            drive(&["src", "app", "main.rs"])
        );
        assert_eq!(
            parse_tool_path("c:/src/./app//main.rs"),
            drive(&["src", "app", "main.rs"])
        );
        assert_eq!(
            parse_tool_path("FILE:///c%3A/my%20dir/a.rs"),
            drive(&["my dir", "a.rs"])
        );
        let posix = |components: &[&str]| {
            Some(ToolPath {
                drive: None,
                components: components.iter().map(|c| c.to_string()).collect(),
            })
        };
        assert_eq!(
            parse_tool_path("/home/me/a.rs"),
            posix(&["home", "me", "a.rs"])
        );
        assert_eq!(
            parse_tool_path("file:///home/me/a.rs"),
            posix(&["home", "me", "a.rs"])
        );
        assert_eq!(
            parse_tool_path("file://localhost/home/a.rs"),
            posix(&["home", "a.rs"])
        );
        for refused in [
            "a.rs",
            "src/a.rs",
            "C:a.rs",
            r"\\server\share\a.rs",
            "file://server/share/a.rs",
            "/home/../etc/passwd",
            r"C:\src\..\x",
            "/home/a\u{0}b",
            "file:///C:/a%zz",
            "C:/a/b:c",
        ] {
            assert_eq!(parse_tool_path(refused), None, "{refused}");
        }
        assert_eq!(
            parse_tool_path(&format!("/{}", "a".repeat(MAX_TOOL_PATH_BYTES))),
            None
        );
    }

    #[test]
    fn file_refs_in_worktrees() {
        let roots = [
            (1, Path::new(r"C:\Projects\App")),
            (2, Path::new(r"\\?\C:\Projects\App\vendor\lib")),
            (3, Path::new("/home/me/app")),
        ];
        let file_ref = |input: &str| file_ref_in(&parse_tool_path(input).unwrap(), roots);
        assert_eq!(
            file_ref("file:///c:/projects/app/src/main.rs"),
            Some(FileRef {
                worktree: Some(1),
                path: "src/main.rs".into()
            })
        );
        assert_eq!(
            file_ref(r"file://C:\Projects\App\vendor\lib\x.rs"),
            Some(FileRef {
                worktree: Some(2),
                path: "x.rs".into()
            })
        );
        assert_eq!(
            file_ref(r"C:\Projects\App"),
            Some(FileRef {
                worktree: Some(1),
                path: String::new()
            })
        );
        assert_eq!(file_ref(r"C:\Projects\Other\a.rs"), None);
        assert_eq!(file_ref(r"D:\Projects\App\a.rs"), None);
        // POSIX paths keep their case.
        assert_eq!(
            file_ref("/home/me/app/A.rs"),
            Some(FileRef {
                worktree: Some(3),
                path: "A.rs".into()
            })
        );
        assert_eq!(file_ref("/home/me/App/A.rs"), None);
        assert_eq!(file_ref("/home/me/application/a.rs"), None);
    }

    #[test]
    fn selections_are_capped_at_a_character() {
        let mut text = "é".repeat(MAX_SELECTION_BYTES);
        cap_selection(&mut text);
        assert_eq!(text.len(), MAX_SELECTION_BYTES);
        let mut text = format!("a{}", "é".repeat(MAX_SELECTION_BYTES / 2));
        cap_selection(&mut text);
        assert_eq!(text.len(), MAX_SELECTION_BYTES - 1);
    }

    fn entry(severity: DiagnosticSeverity, line: u32, message: &str) -> Diagnostic {
        let at = crate::edit::Position { line, column: 0 };
        Diagnostic {
            range: TextRange { start: at, end: at },
            severity,
            message: message.into(),
            source: Some("rust-analyzer".into()),
            code: None,
        }
    }

    fn file(path: &str, diagnostics: Vec<Diagnostic>) -> FileDiagnostics {
        FileDiagnostics {
            path: FileRef {
                worktree: Some(1),
                path: path.into(),
            },
            diagnostics,
        }
    }

    #[test]
    fn snapshots_put_errors_first_then_path_and_line() {
        use DiagnosticSeverity::*;
        let snapshot = diagnostics_snapshot(vec![
            file(
                "b.rs",
                vec![entry(Warning, 3, "w b3"), entry(Error, 9, "e b9")],
            ),
            file("empty.rs", Vec::new()),
            file(
                "a.rs",
                vec![entry(Hint, 1, "h a1"), entry(Error, 5, "e a5")],
            ),
            file("c.rs", vec![entry(Information, 0, "i c0")]),
        ]);
        assert!(!snapshot.truncated);
        let order: Vec<(&str, Vec<&str>)> = snapshot
            .files
            .iter()
            .map(|file| {
                (
                    file.path.path.as_str(),
                    file.diagnostics
                        .iter()
                        .map(|entry| entry.message.as_str())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                ("a.rs", vec!["e a5", "h a1"]),
                ("b.rs", vec!["e b9", "w b3"]),
                ("c.rs", vec!["i c0"]),
            ]
        );
    }

    #[test]
    fn snapshots_keep_500_errors_first_and_cut_messages() {
        use DiagnosticSeverity::*;
        let warnings = (0..400).map(|line| entry(Warning, line, "w")).collect();
        let errors = (0..300)
            .map(|line| entry(Error, line, &"é".repeat(MAX_DIAGNOSTIC_MESSAGE_BYTES)))
            .collect();
        let snapshot = diagnostics_snapshot(vec![file("w.rs", warnings), file("z.rs", errors)]);
        assert!(snapshot.truncated);
        assert_eq!(snapshot.files[0].path.path, "z.rs");
        assert_eq!(snapshot.files[0].diagnostics.len(), 300);
        assert_eq!(snapshot.files[1].diagnostics.len(), 200);
        assert_eq!(snapshot.files[1].diagnostics[199].range.start.line, 199);
        let message = &snapshot.files[0].diagnostics[0].message;
        assert_eq!(message.len(), MAX_DIAGNOSTIC_MESSAGE_BYTES);
        let exact = diagnostics_snapshot(vec![file(
            "x.rs",
            (0..500).map(|line| entry(Hint, line, "h")).collect(),
        )]);
        assert!(!exact.truncated);
        let mut long = entry(Error, 0, "e");
        long.code = Some("x".repeat(1000));
        let cut = diagnostics_snapshot(vec![file("x.rs", vec![long])]);
        assert_eq!(
            cut.files[0].diagnostics[0].code.as_deref().map(str::len),
            Some(MAX_DIAGNOSTIC_LABEL_BYTES)
        );
    }
}
