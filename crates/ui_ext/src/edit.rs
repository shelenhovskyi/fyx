//! The pure part of `workspace:edit` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §10, plan 4.8):
//! the edit model, the limits, the import's checks and the checks before an edit is applied
//! (overlaps, protected paths). Resolving targets in the project, versions, symlinks, the
//! `private_files` setting and the undoable transaction are the host's (`ui_ext_host`).

use std::collections::BTreeSet;

use crate::process::FileRef;
use crate::v0_2::{HostError, HostErrorCode};

pub const CAPABILITY_EDIT: &str = "workspace:edit";
pub const MAX_FILES: usize = 64;
/// Text edits across all the files of one edit.
pub const MAX_TEXT_EDITS: usize = 4096;
/// Bytes of new text across one edit.
pub const MAX_NEW_TEXT_BYTES: usize = 4 << 20;
pub const MAX_LABEL_BYTES: usize = 128;
pub const MAX_EDITS_IN_FLIGHT: usize = 4;
/// Files larger than this are refused (by the host, which knows their size).
pub const MAX_FILE_BYTES: u64 = 16 << 20;

/// Directories no extension may write into, at any depth (design.md §10, integration-5 S2):
/// repository internals and hooks, Fyx's agent review rules, editor settings and tasks (code
/// execution). Agent configs ([`crate::protected`]) are refused as well, whatever the shield mode.
pub const PROTECTED_DIRECTORIES: &[&str] = &[".git", ".fyx", ".zed", ".vscode", ".idea"];

/// Zero-based line, and a column in UTF-8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Half-open: `start` is included, `end` is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextRange {
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub new_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// A `documents` id.
    Document(u64),
    File(FileRef),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    pub target: EditTarget,
    /// The `document-info.version` the edit was computed against; a newer one is `conflict`.
    pub expected_version: Option<u64>,
    /// Create the file (empty) first if it does not exist; its directory must.
    pub create: bool,
    pub edits: Vec<TextEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceEdit {
    /// The undo step's name and the notification's title.
    pub label: String,
    pub files: Vec<FileEdit>,
}

/// The WIT `edit-error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    Denied,
    Conflict(String),
    NotAllowed(String),
    NotFound(String),
    Invalid(String),
    Cancelled,
    /// A file is in a worktree that is not trusted (restricted mode, integration-5 A5).
    UntrustedWorktree,
    Other(String),
}

fn invalid(message: impl Into<String>) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message: message.into(),
    }
}

/// The import's checks (design.md §10, "Limits"): the label, the file, edit and text caps, and
/// the path rule of every `file` target.
pub fn check_edit(edit: &WorkspaceEdit) -> Result<(), HostError> {
    if edit.label.is_empty() || edit.label.len() > MAX_LABEL_BYTES {
        return Err(invalid(format!(
            "the label must be 1 to {MAX_LABEL_BYTES} bytes"
        )));
    }
    if edit.label.chars().any(char::is_control) {
        return Err(invalid("the label is one line of text"));
    }
    if edit.files.is_empty() || edit.files.len() > MAX_FILES {
        return Err(invalid(format!("an edit names 1 to {MAX_FILES} files")));
    }
    let edits: usize = edit.files.iter().map(|file| file.edits.len()).sum();
    if edits > MAX_TEXT_EDITS {
        return Err(HostError {
            code: HostErrorCode::LimitExceeded,
            message: format!("at most {MAX_TEXT_EDITS} text edits per edit"),
        });
    }
    let text: usize = edit
        .files
        .iter()
        .flat_map(|file| &file.edits)
        .map(|edit| edit.new_text.len())
        .sum();
    if text > MAX_NEW_TEXT_BYTES {
        return Err(HostError {
            code: HostErrorCode::LimitExceeded,
            message: format!("at most {MAX_NEW_TEXT_BYTES} bytes of new text per edit"),
        });
    }
    for file in &edit.files {
        if let EditTarget::File(file_ref) = &file.target {
            if file_ref.path.is_empty() {
                return Err(invalid("a file target needs a path"));
            }
            file_ref.check()?;
        }
        if file.create && !matches!(file.target, EditTarget::File(_)) {
            return Err(invalid("create needs a file target"));
        }
        for text_edit in &file.edits {
            if text_edit.range.start > text_edit.range.end {
                return Err(invalid("a range ends before it starts"));
            }
        }
    }
    Ok(())
}

/// The rule a worktree-relative path falls under, if any: a [`PROTECTED_DIRECTORIES`] entry at
/// any depth, or an agent config ([`crate::protected::protected_config_in`]: `.claude/`, `.codex/`,
/// `.mcp.json`, `AGENTS.md`, ...). ASCII case does not matter, so `.GIT/config` is protected on
/// case-insensitive file systems too.
pub fn protected_rule(path: &str) -> Option<String> {
    let names: Vec<&str> = path.split('/').collect();
    if let Some(dir) = names.iter().find_map(|name| {
        PROTECTED_DIRECTORIES
            .iter()
            .find(|protected| name.eq_ignore_ascii_case(protected))
    }) {
        return Some(format!("{dir}/"));
    }
    crate::protected::protected_config_in(&names)
}

pub fn is_protected_path(path: &str) -> bool {
    protected_rule(path).is_some()
}

/// The checks before the host resolves anything, when the edit is applied: overlapping edits in
/// a file are `invalid`, a protected path or two edits of the same file are refused. One failing
/// file fails the whole edit, so nothing is applied.
pub fn check_targets(edit: &WorkspaceEdit) -> Result<(), EditError> {
    let mut targets = BTreeSet::new();
    for file in &edit.files {
        let key = match &file.target {
            EditTarget::Document(id) => format!("document:{id}"),
            EditTarget::File(file_ref) => {
                if let Some(rule) = protected_rule(&file_ref.path) {
                    return Err(EditError::NotAllowed(format!(
                        "{} is protected ({rule}): it can make a program run",
                        file_ref.path
                    )));
                }
                format!(
                    "file:{}:{}",
                    file_ref.worktree.map_or(String::new(), |id| id.to_string()),
                    file_ref.path
                )
            }
        };
        if !targets.insert(key) {
            return Err(EditError::Invalid(
                "a file appears twice in one edit; put its edits together".into(),
            ));
        }
        let mut ranges: Vec<TextRange> = file.edits.iter().map(|edit| edit.range).collect();
        ranges.sort_by_key(|range| (range.start, range.end));
        for pair in ranges.windows(2) {
            // Half-open ranges: touching is fine, two inserts at one point are not.
            if pair[1].start < pair[0].end
                || (pair[0].start == pair[0].end && pair[1].start == pair[0].start)
            {
                return Err(EditError::Invalid(format!(
                    "edits overlap at line {}, column {}",
                    pair[1].start.line, pair[1].start.column
                )));
            }
        }
    }
    Ok(())
}

/// The in-flight cap ([`MAX_EDITS_IN_FLIGHT`]) and the ids.
#[derive(Debug, Default)]
pub struct EditCounters {
    next_id: u64,
    in_flight: BTreeSet<u64>,
}

impl EditCounters {
    pub fn start(&mut self) -> Result<u64, HostError> {
        if self.in_flight.len() >= MAX_EDITS_IN_FLIGHT {
            return Err(HostError {
                code: HostErrorCode::LimitExceeded,
                message: format!("at most {MAX_EDITS_IN_FLIGHT} edits in flight"),
            });
        }
        self.next_id += 1;
        self.in_flight.insert(self.next_id);
        Ok(self.next_id)
    }

    /// Its `edit-done` was delivered, or the call that made it failed.
    pub fn finished(&mut self, id: u64) {
        self.in_flight.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, column: u32) -> Position {
        Position { line, column }
    }

    fn text_edit(start: (u32, u32), end: (u32, u32), text: &str) -> TextEdit {
        TextEdit {
            range: TextRange {
                start: at(start.0, start.1),
                end: at(end.0, end.1),
            },
            new_text: text.into(),
        }
    }

    fn file(path: &str, edits: Vec<TextEdit>) -> FileEdit {
        FileEdit {
            target: EditTarget::File(FileRef {
                worktree: None,
                path: path.into(),
            }),
            expected_version: None,
            create: false,
            edits,
        }
    }

    fn edit(files: Vec<FileEdit>) -> WorkspaceEdit {
        WorkspaceEdit {
            label: "Rename".into(),
            files,
        }
    }

    #[test]
    fn the_import_checks() {
        let ok = edit(vec![file(
            "src/main.rs",
            vec![text_edit((0, 0), (0, 3), "fn")],
        )]);
        assert_eq!(check_edit(&ok), Ok(()));
        let mut no_label = ok.clone();
        no_label.label.clear();
        assert!(check_edit(&no_label).is_err());
        let mut long_label = ok;
        long_label.label = "a".repeat(MAX_LABEL_BYTES + 1);
        assert!(check_edit(&long_label).is_err());
        assert!(check_edit(&edit(Vec::new())).is_err());
        assert!(check_edit(&edit(vec![file("a.rs", Vec::new()); MAX_FILES + 1])).is_err());
        let many = edit(vec![file(
            "a.rs",
            (0..=MAX_TEXT_EDITS as u32)
                .map(|line| text_edit((line, 0), (line, 0), "x"))
                .collect(),
        )]);
        assert_eq!(
            check_edit(&many).unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        let big = edit(vec![file(
            "a.rs",
            vec![text_edit(
                (0, 0),
                (0, 0),
                &"x".repeat(MAX_NEW_TEXT_BYTES + 1),
            )],
        )]);
        assert_eq!(
            check_edit(&big).unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        for path in ["../outside.rs", "/etc/passwd", "C:x", "a\\b", ""] {
            assert!(
                check_edit(&edit(vec![file(path, Vec::new())])).is_err(),
                "{path:?}"
            );
        }
        let backwards = edit(vec![file("a.rs", vec![text_edit((2, 0), (1, 0), "")])]);
        assert!(check_edit(&backwards).is_err());
    }

    #[test]
    fn protected_paths_and_overlaps() {
        for path in [
            ".git/hooks/pre-commit",
            ".GIT/config",
            ".fyx/review.toml",
            ".zed/tasks.json",
            ".vscode/tasks.json",
            ".idea/workspace.xml",
        ] {
            assert!(matches!(
                check_targets(&edit(vec![
                    file("src/ok.rs", Vec::new()),
                    file(path, Vec::new())
                ])),
                Err(EditError::NotAllowed(_))
            ));
        }
        assert!(!is_protected_path("src/.git-like/x"));
        assert!(!is_protected_path("docs/.zed.md"));
        // integration-5 S2: at any depth, and agent configs, whatever the shield mode.
        for path in [
            "sub/.zed/settings.json",
            "pkg/.vscode/tasks.json",
            "a/.idea/workspace.xml",
            ".claude/settings.json",
            "tools/.codex/config.toml",
            ".cursor/rules/x.mdc",
            ".mcp.json",
            "sub/.mcp.json",
            "AGENTS.md",
            "docs/CLAUDE.md",
            "vendor/lib/.git/config",
        ] {
            assert!(is_protected_path(path), "{path}");
        }
        assert!(!is_protected_path("notes/mcp.json"));
        assert!(!is_protected_path("src/agents.rs"));
        let overlapping = edit(vec![file(
            "a.rs",
            vec![
                text_edit((0, 0), (0, 5), "a"),
                text_edit((0, 4), (0, 6), "b"),
            ],
        )]);
        assert!(matches!(
            check_targets(&overlapping),
            Err(EditError::Invalid(_))
        ));
        let two_inserts = edit(vec![file(
            "a.rs",
            vec![
                text_edit((1, 0), (1, 0), "a"),
                text_edit((1, 0), (1, 0), "b"),
            ],
        )]);
        assert!(matches!(
            check_targets(&two_inserts),
            Err(EditError::Invalid(_))
        ));
        let touching = edit(vec![file(
            "a.rs",
            vec![
                text_edit((0, 0), (0, 5), "a"),
                text_edit((0, 5), (0, 6), "b"),
            ],
        )]);
        assert_eq!(check_targets(&touching), Ok(()));
        let twice = edit(vec![file("a.rs", Vec::new()), file("a.rs", Vec::new())]);
        assert!(matches!(check_targets(&twice), Err(EditError::Invalid(_))));
    }

    #[test]
    fn counters() {
        let mut counters = EditCounters::default();
        for id in 1..=MAX_EDITS_IN_FLIGHT as u64 {
            assert_eq!(counters.start(), Ok(id));
        }
        assert!(counters.start().is_err());
        counters.finished(1);
        assert_eq!(counters.start(), Ok(MAX_EDITS_IN_FLIGHT as u64 + 1));
    }
}
