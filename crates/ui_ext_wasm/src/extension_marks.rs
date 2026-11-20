//! fyx:ui 0.4 marks an extension puts on the user's files (design wit-0.4 §E, §F.2, plans 6.6
//! and 6.7): `context.publish-diagnostics`/`clear-diagnostics` (`diagnostics:publish`) and
//! `context.decorate-files`/`clear-file-decorations` (`workspace:file-decorations`), checked here
//! and handed to the host's [`DiagnosticsPublisher`] and [`FileDecorationsHost`].
//!
//! The import checks the capability first. Every path must be in a worktree (`invalid-argument`)
//! and not private (`not-permitted`); diagnostics need a file, decorations a file or directory.
//! What an instance published goes away with it: [`MarksHandle`]'s drop, and a revocation
//! ([`MarksHandle::revoke_diagnostics`], [`MarksHandle::revoke_decorations`]), clear it at the
//! host.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

use ui_ext::{
    context::Diagnostic,
    file_decorations::{DecoratedFiles, FileDecoration, FileDecorationsHost},
    process::FileRef,
    published_diagnostics::{DiagnosticsPublisher, PublishedDiagnostics},
    v0_2::{HostError, HostErrorCode},
};

use crate::{
    accounts::in_account_dir,
    editor_context::{resolve, roots},
    process::Workspace,
};

/// One instance's published diagnostics and file decorations.
pub(crate) struct MarksHandle {
    extension_id: Arc<str>,
    workspace: Option<Arc<dyn Workspace>>,
    publisher: Option<Arc<dyn DiagnosticsPublisher>>,
    decorations: Option<Arc<dyn FileDecorationsHost>>,
    published: Mutex<PublishedDiagnostics>,
    decorated: Mutex<DecoratedFiles>,
}

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MarksHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        workspace: Option<Arc<dyn Workspace>>,
        publisher: Option<Arc<dyn DiagnosticsPublisher>>,
        decorations: Option<Arc<dyn FileDecorationsHost>>,
    ) -> Self {
        Self {
            extension_id,
            workspace,
            publisher,
            decorations,
            published: Mutex::default(),
            decorated: Mutex::default(),
        }
    }

    /// `path` checked, in a worktree, not private: with its worktree named.
    fn resolve(
        &self,
        path: FileRef,
        is_private: &dyn Fn(&Path) -> bool,
        want_file: bool,
    ) -> Result<FileRef, HostError> {
        path.check()?;
        let roots = roots(self.workspace.as_deref())?;
        let (worktree, full) = resolve(&roots, &path)?;
        if is_private(&full) || in_account_dir(&full) {
            return Err(error(
                HostErrorCode::NotPermitted,
                format!("{:?} is a private file", path.path),
            ));
        }
        let exists = if want_file {
            full.is_file()
        } else {
            full.exists()
        };
        if !exists {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!(
                    "{:?} is not a {} in the worktree",
                    path.path,
                    if want_file {
                        "file"
                    } else {
                        "file or directory"
                    }
                ),
            ));
        }
        Ok(FileRef {
            worktree: Some(worktree),
            path: path.path,
        })
    }

    fn publisher(&self) -> Result<&Arc<dyn DiagnosticsPublisher>, HostError> {
        self.publisher.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "this window shows no diagnostics from extensions",
            )
        })
    }

    fn decorations_host(&self) -> Result<&Arc<dyn FileDecorationsHost>, HostError> {
        self.decorations.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "this window has no project panel to decorate",
            )
        })
    }

    /// `context.publish-diagnostics` (§E.1): replaces this extension's diagnostics of `path`.
    pub(crate) fn publish_diagnostics(
        &self,
        path: FileRef,
        mut diagnostics: Vec<Diagnostic>,
    ) -> Result<(), HostError> {
        let publisher = self.publisher()?;
        let path = self.resolve(path, &|full| publisher.is_private(full), true)?;
        lock(&self.published).publish(&path, &mut diagnostics, Instant::now())?;
        publisher
            .publish(&self.extension_id, path, diagnostics)
            .map_err(|why| error(HostErrorCode::Unavailable, why))
    }

    /// `context.clear-diagnostics`: one file (an empty publish), or every file with `None`.
    pub(crate) fn clear_diagnostics(&self, path: Option<FileRef>) -> Result<(), HostError> {
        match path {
            Some(path) => self.publish_diagnostics(path, Vec::new()),
            None => {
                let publisher = self.publisher()?;
                lock(&self.published).clear_all(Instant::now())?;
                publisher.clear_all(&self.extension_id);
                Ok(())
            }
        }
    }

    /// `context.decorate-files` (§F.2): sets or replaces this extension's decoration of each
    /// path. One bad path or record refuses the whole call.
    pub(crate) fn decorate_files(&self, decorations: Vec<FileDecoration>) -> Result<(), HostError> {
        let host = self.decorations_host()?;
        if decorations.len() > ui_ext::file_decorations::MAX_DECORATIONS_PER_CALL {
            // Before resolving 10,000 paths.
            return lock(&self.decorated).decorate(&decorations);
        }
        let decorations = decorations
            .into_iter()
            .map(|decoration| {
                Ok(FileDecoration {
                    path: self.resolve(decoration.path, &|full| host.is_private(full), false)?,
                    ..decoration
                })
            })
            .collect::<Result<Vec<_>, HostError>>()?;
        lock(&self.decorated).decorate(&decorations)?;
        host.decorate(&self.extension_id, decorations)
            .map_err(|why| error(HostErrorCode::Unavailable, why))
    }

    /// `context.clear-file-decorations`: these paths, or all with `None`.
    pub(crate) fn clear_file_decorations(
        &self,
        paths: Option<Vec<FileRef>>,
    ) -> Result<(), HostError> {
        let host = self.decorations_host()?;
        let paths = match paths {
            None => None,
            Some(paths) => {
                let roots = roots(self.workspace.as_deref())?;
                let resolved = paths
                    .into_iter()
                    .map(|path| {
                        path.check()?;
                        let (worktree, _) = resolve(&roots, &path)?;
                        Ok(FileRef {
                            worktree: Some(worktree),
                            path: path.path,
                        })
                    })
                    .collect::<Result<Vec<_>, HostError>>()?;
                Some(resolved)
            }
        };
        lock(&self.decorated).clear(paths.as_deref());
        host.clear(&self.extension_id, paths);
        Ok(())
    }

    /// `diagnostics:publish` was revoked: everything published goes.
    pub(crate) fn revoke_diagnostics(&self) {
        let mut published = lock(&self.published);
        if published.is_empty() {
            return;
        }
        published.forget();
        if let Some(publisher) = &self.publisher {
            publisher.clear_all(&self.extension_id);
        }
    }

    /// `workspace:file-decorations` was revoked: every decoration goes.
    pub(crate) fn revoke_decorations(&self) {
        let mut decorated = lock(&self.decorated);
        if decorated.is_empty() {
            return;
        }
        decorated.clear(None);
        if let Some(host) = &self.decorations {
            host.clear(&self.extension_id, None);
        }
    }
}

impl Drop for MarksHandle {
    /// The instance stopped, crashed or was reloaded: its marks go with it (§E.1, §F.2).
    fn drop(&mut self) {
        self.revoke_diagnostics();
        self.revoke_decorations();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ui_ext::ColorToken;
    use ui_ext::context::DiagnosticSeverity;
    use ui_ext::edit::{Position, TextRange};

    use super::*;
    use crate::process::WorktreeRoot;

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
    }

    impl Recorder {
        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *lock(&self.calls))
        }

        fn record(&self, call: String) {
            lock(&self.calls).push(call);
        }
    }

    impl DiagnosticsPublisher for Recorder {
        fn publish(
            &self,
            extension_id: &str,
            path: FileRef,
            diagnostics: Vec<Diagnostic>,
        ) -> Result<(), String> {
            let sources: Vec<_> = diagnostics
                .iter()
                .map(|entry| entry.source.clone())
                .collect();
            self.record(format!(
                "publish {extension_id} {:?}:{} {} {sources:?}",
                path.worktree,
                path.path,
                diagnostics.len()
            ));
            Ok(())
        }

        fn clear_all(&self, extension_id: &str) {
            self.record(format!("clear-all {extension_id}"));
        }

        fn is_private(&self, absolute: &Path) -> bool {
            absolute.ends_with(".env")
        }
    }

    impl FileDecorationsHost for Recorder {
        fn decorate(
            &self,
            extension_id: &str,
            decorations: Vec<FileDecoration>,
        ) -> Result<(), String> {
            let paths: Vec<_> = decorations
                .iter()
                .map(|decoration| {
                    format!("{:?}:{}", decoration.path.worktree, decoration.path.path)
                })
                .collect();
            self.record(format!("decorate {extension_id} {}", paths.join(",")));
            Ok(())
        }

        fn clear(&self, extension_id: &str, paths: Option<Vec<FileRef>>) {
            self.record(format!(
                "clear {extension_id} {:?}",
                paths.map(|paths| paths.len())
            ));
        }

        fn is_private(&self, absolute: &Path) -> bool {
            absolute.ends_with(".env")
        }
    }

    struct Worktrees(Vec<WorktreeRoot>);

    impl Workspace for Worktrees {
        fn worktrees(&self) -> Vec<WorktreeRoot> {
            self.0.clone()
        }
    }

    fn setup() -> (tempfile::TempDir, Arc<Recorder>, MarksHandle) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        for file in ["src/a.rs", ".env"] {
            std::fs::write(dir.path().join(file), "TODO\n").unwrap();
        }
        let recorder = Arc::new(Recorder::default());
        let workspace = Arc::new(Worktrees(vec![WorktreeRoot {
            id: 7,
            root: PathBuf::from(dir.path()),
            trusted: true,
        }]));
        let handle = MarksHandle::new(
            "lint".into(),
            Some(workspace),
            Some(recorder.clone()),
            Some(recorder.clone()),
        );
        (dir, recorder, handle)
    }

    fn file(path: &str) -> FileRef {
        FileRef {
            worktree: None,
            path: path.into(),
        }
    }

    fn entry() -> Diagnostic {
        Diagnostic {
            range: TextRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 4 },
            },
            severity: DiagnosticSeverity::Warning,
            message: "a TODO".into(),
            source: Some("rust-analyzer".into()),
            code: None,
        }
    }

    fn decoration(path: &str, badge: &str) -> FileDecoration {
        FileDecoration {
            path: file(path),
            badge: Some(badge.into()),
            color: Some(ColorToken::Warning),
            tooltip: None,
        }
    }

    /// §E.4: the worktree is named, `source` dropped; a private file and one in no worktree are
    /// refused and publish nothing; dropping the instance clears what it published.
    #[test]
    fn diagnostics_are_checked_and_cleared() {
        let (_dir, recorder, handle) = setup();
        handle
            .publish_diagnostics(file("src/a.rs"), vec![entry(), entry()])
            .unwrap();
        assert_eq!(
            recorder.take(),
            ["publish lint Some(7):src/a.rs 2 [None, None]"]
        );
        let code = |result: Result<(), HostError>| result.unwrap_err().code;
        assert_eq!(
            code(handle.publish_diagnostics(file(".env"), vec![entry()])),
            HostErrorCode::NotPermitted
        );
        assert_eq!(
            code(handle.publish_diagnostics(file("missing.rs"), vec![entry()])),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(handle.publish_diagnostics(file("src"), vec![entry()])),
            HostErrorCode::InvalidArgument
        );
        assert!(recorder.take().is_empty());
        handle.clear_diagnostics(Some(file("src/a.rs"))).unwrap();
        assert_eq!(recorder.take(), ["publish lint Some(7):src/a.rs 0 []"]);
        // Nothing left: dropping clears nothing.
        handle
            .publish_diagnostics(file("src/a.rs"), vec![entry()])
            .unwrap();
        recorder.take();
        drop(handle);
        assert_eq!(recorder.take(), ["clear-all lint"]);
    }

    /// §F.5: a badge of 3 characters and a private file are refused, nothing is decorated;
    /// directories may be decorated; revoking clears.
    #[test]
    fn decorations_are_checked_and_cleared() {
        let (_dir, recorder, handle) = setup();
        handle
            .decorate_files(vec![decoration("src/a.rs", "★"), decoration("src", "B")])
            .unwrap();
        assert_eq!(
            recorder.take(),
            ["decorate lint Some(7):src/a.rs,Some(7):src"]
        );
        let code = |result: Result<(), HostError>| result.unwrap_err().code;
        assert_eq!(
            code(handle.decorate_files(vec![decoration("src/a.rs", "abc")])),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(handle.decorate_files(vec![decoration("src/a.rs", "★"), decoration(".env", "!")])),
            HostErrorCode::NotPermitted
        );
        assert!(recorder.take().is_empty());
        handle
            .clear_file_decorations(Some(vec![file("src")]))
            .unwrap();
        assert_eq!(recorder.take(), ["clear lint Some(1)"]);
        handle.revoke_decorations();
        assert_eq!(recorder.take(), ["clear lint None"]);
        // Already cleared: the drop does not call the host again.
        drop(handle);
        assert!(recorder.take().is_empty());
    }
}
