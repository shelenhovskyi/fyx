//! A fake editor for the staged `fyx:ui` 0.4 `context` (design.md B.11), with the host's path
//! rules from `ui_ext::context` (W2c's 5.13), so an extension's path mapping, opening and
//! selection feed run headless.
//!
//! A test names the open worktrees ([`crate::Harness::set_worktrees`]; their roots need not
//! exist), marks private files ([`crate::Harness::set_private_file`]) and sets what the editor
//! shows ([`crate::Harness::set_editor_context`]). While the extension watches,
//! [`crate::Harness::deliver_io`] delivers the context once when watching starts and again after
//! each change, filtered as the host filters it: worktree files only, no text of a private
//! active file, the selection capped. `open` is recorded ([`crate::Harness::opened_files`]);
//! unlike the host, the harness does not check that the file exists, and it records the focus
//! the extension asked for.

use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use ui_ext::context::{self as model, EditorContext};
use ui_ext::edit::TextRange;
use ui_ext::process::FileRef;
use ui_ext::v0_2::{HostError, HostErrorCode};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

/// A `context.open` the extension made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedFile {
    pub file: FileRef,
    pub range: Option<TextRange>,
    /// What the extension asked for; the host grants it only after a gesture or a recent
    /// listener request while a terminal has focus.
    pub focus: bool,
}

#[derive(Debug, Default)]
pub(crate) struct FakeContext {
    /// (worktree id, root as the host spells it).
    roots: Vec<(u64, String)>,
    private: BTreeSet<(u64, String)>,
    watching: bool,
    current: EditorContext,
    pub(crate) events: VecDeque<EditorContext>,
    pub(crate) opened: Vec<OpenedFile>,
}

impl FakeContext {
    pub(crate) fn set_roots(&mut self, roots: Vec<(u64, String)>) {
        self.roots = roots;
    }

    pub(crate) fn set_private(&mut self, worktree: u64, path: &str) {
        self.private.insert((worktree, path.to_owned()));
    }

    pub(crate) fn is_watching(&self) -> bool {
        self.watching
    }

    pub(crate) fn is_private(&self, file: &FileRef) -> bool {
        file.worktree
            .is_some_and(|id| self.private.contains(&(id, file.path.clone())))
    }

    fn in_worktree(&self, file: &FileRef) -> bool {
        file.check().is_ok()
            && !file.path.is_empty()
            && file
                .worktree
                .is_some_and(|id| self.roots.iter().any(|(root, _)| *root == id))
    }

    /// The host's privacy filter.
    fn filtered(&self, mut context: EditorContext) -> EditorContext {
        context.open.retain(|file| self.in_worktree(file));
        if context
            .active
            .as_ref()
            .is_some_and(|file| !self.in_worktree(file))
        {
            context.active = None;
            context.selection = None;
            context.trusted = None;
        }
        match &context.active {
            None => context.selection = None,
            Some(active) if self.is_private(active) => {
                if let Some((_, text)) = &mut context.selection {
                    *text = None;
                }
            }
            Some(_) => {}
        }
        if let Some((_, Some(text))) = &mut context.selection {
            model::cap_selection(text);
        }
        context
    }

    pub(crate) fn set_context(&mut self, context: EditorContext) {
        self.current = self.filtered(context);
        if self.watching {
            self.events.push_back(self.current.clone());
        }
    }

    pub(crate) fn watch(&mut self, on: bool) {
        if on && !self.watching {
            self.events.push_back(self.current.clone());
        }
        self.watching = on;
    }

    /// The worktree of `file`: its own, or the only one when it names none.
    fn root(&self, file: &FileRef) -> Result<&str, HostError> {
        file.check()?;
        let found = match file.worktree {
            Some(id) => self.roots.iter().find(|(root, _)| *root == id),
            None if self.roots.len() == 1 => self.roots.first(),
            None => None,
        };
        found.map(|(_, root)| root.as_str()).ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                format!("no worktree for {:?}", file.path),
            )
        })
    }

    pub(crate) fn open(
        &mut self,
        file: FileRef,
        range: Option<TextRange>,
        focus: bool,
    ) -> Result<(), HostError> {
        self.root(&file)?;
        if file.path.is_empty() {
            return Err(error(HostErrorCode::InvalidArgument, "not a file"));
        }
        self.opened.push(OpenedFile { file, range, focus });
        Ok(())
    }

    pub(crate) fn to_file_ref(&self, absolute: &str) -> Result<FileRef, HostError> {
        let parsed = model::parse_tool_path(absolute).ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                "not an absolute path or file URI",
            )
        })?;
        let file = model::file_ref_in(
            &parsed,
            self.roots
                .iter()
                .map(|(id, root)| (*id, Path::new(root.as_str()))),
        )
        .ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                "the path is outside every open worktree",
            )
        })?;
        if self.is_private(&file) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        Ok(file)
    }

    /// The root joined with the path, in the root's own separator.
    pub(crate) fn absolute_path(&self, file: &FileRef) -> Result<String, HostError> {
        let root = self.root(file)?;
        if self.is_private(file) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        let separator = if root.contains('\\') { "\\" } else { "/" };
        let mut absolute = root.trim_end_matches(['/', '\\']).to_owned();
        for part in file.path.split('/').filter(|part| !part.is_empty()) {
            absolute.push_str(separator);
            absolute.push_str(part);
        }
        Ok(absolute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::edit::Position;

    fn file(worktree: u64, path: &str) -> FileRef {
        FileRef {
            worktree: Some(worktree),
            path: path.into(),
        }
    }

    fn fake() -> FakeContext {
        let mut context = FakeContext::default();
        context.set_roots(vec![(1, "/work/app".into()), (2, r"C:\work\lib".into())]);
        context
    }

    #[test]
    fn paths_map_both_ways() {
        let mut context = fake();
        assert_eq!(
            context.to_file_ref("/work/app/src/a.rs").unwrap(),
            file(1, "src/a.rs")
        );
        assert_eq!(
            context.to_file_ref("file:///C:/work/lib/b.rs").unwrap(),
            file(2, "b.rs")
        );
        assert_eq!(
            context.to_file_ref(r"c:\work\lib\b.rs").unwrap(),
            file(2, "b.rs")
        );
        assert_eq!(
            context.absolute_path(&file(2, "x/b.rs")).unwrap(),
            r"C:\work\lib\x\b.rs"
        );
        assert_eq!(
            context.absolute_path(&file(1, "src/a.rs")).unwrap(),
            "/work/app/src/a.rs"
        );
        let code = |result: Result<FileRef, HostError>| result.unwrap_err().code;
        assert_eq!(
            code(context.to_file_ref("/elsewhere/a")),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(context.to_file_ref("src/a.rs")),
            HostErrorCode::InvalidArgument
        );
        context.set_private(1, ".env");
        assert_eq!(
            code(context.to_file_ref("/work/app/.env")),
            HostErrorCode::NotPermitted
        );
        assert!(context.absolute_path(&file(3, "a")).is_err());
        assert!(context.open(file(1, "../a"), None, false).is_err());
        context.open(file(1, "src/a.rs"), None, true).unwrap();
        assert_eq!(context.opened.len(), 1);
    }

    #[test]
    fn the_feed_is_filtered() {
        let mut context = fake();
        context.set_private(1, ".env");
        let range = TextRange {
            start: Position { line: 0, column: 0 },
            end: Position { line: 0, column: 3 },
        };
        context.set_context(EditorContext {
            active: Some(file(1, ".env")),
            open: vec![file(1, ".env"), file(9, "gone.rs")],
            selection: Some((range, Some("KEY".into()))),
            trusted: Some(true),
        });
        assert!(context.events.is_empty(), "not watching yet");
        context.watch(true);
        let sent = context.events.pop_front().unwrap();
        assert_eq!(sent.open, [file(1, ".env")]);
        assert_eq!(sent.selection, Some((range, None)));
        context.set_context(EditorContext {
            active: Some(file(9, "gone.rs")),
            ..EditorContext::default()
        });
        assert_eq!(
            context.events.pop_front().unwrap(),
            EditorContext::default()
        );
        context.watch(false);
        context.set_context(EditorContext::default());
        assert!(context.events.is_empty());
    }
}
