//! `fyx:ui` 0.4: editor context and opening files (design.md B.11), with the
//! `fyx-ui-0-4` feature.
//!
//! - [`watch`] (`context:editor`) starts the editor feed: `IoEvent::ContextChanged` arrives at
//!   once and after each change (debounced 300 ms), with the active file, the open files and the
//!   selection. Only worktree files appear; a private file's selection has no text.
//! - [`to_file_ref`] and [`absolute_path`] (`context:editor`) map a tool's absolute paths and
//!   `file://` URIs to worktree files and back. A private file is `not-permitted`.
//! - [`open`] (`workspace:open`) opens a worktree file; `focus` needs a gesture or a recent
//!   listener request while one of the extension's terminals has focus.
//! - [`diagnostics`] (`workspace:diagnostics`) reads the language servers' diagnostics of one
//!   file or of every worktree file (errors first, at most 500 entries, private files left out);
//!   [`watch_diagnostics`] sends `IoEvent::DiagnosticsChanged` with the files that changed
//!   (debounced 300 ms; empty when more than 200 did), with no entries: read them again.
//!
//! - [`publish_diagnostics`] and [`clear_diagnostics`] (`diagnostics:publish`, plan 6.6) show the
//!   extension's own problems in a file, like a language server's; the host names the extension
//!   as their source. They last while the extension runs.
//! - [`decorate_files`] and [`clear_file_decorations`] (`workspace:file-decorations`, plan 6.7)
//!   put a badge, a colour or a tooltip line on project-panel entries.
//!
//! Exported as 0.2 or 0.1, every function here fails with `unavailable`. Off WASM, [`watch`],
//! [`open`] and [`watch_diagnostics`] record a [`HostEffect::Context`](crate::HostEffect::Context),
//! the path mapping uses the roots [`set_worktree_roots`] gave (none: everything is outside), and
//! [`diagnostics`] answers from [`set_diagnostics`]. The publishing and decorating calls record
//! their arguments as they are; the checks (limits, worktree, private files) are the host's and
//! the harness's.

pub use crate::bindings::fyx::ui::context::{
    Diagnostic, DiagnosticsSnapshot, EditorContext, FileDecoration, FileDiagnostics, Severity,
};
use crate::{FileRef, HostError, TextRange};

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_context_watch(on: bool) -> Result<(), HostError>;
    safe fn __fyx_ui_context_open(
        path: &FileRef,
        range: Option<TextRange>,
        focus: bool,
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_context_to_file_ref(absolute: &str) -> Result<FileRef, HostError>;
    safe fn __fyx_ui_context_absolute_path(path: &FileRef) -> Result<String, HostError>;
    safe fn __fyx_ui_context_diagnostics(
        path: Option<&FileRef>,
    ) -> Result<DiagnosticsSnapshot, HostError>;
    safe fn __fyx_ui_context_watch_diagnostics(on: bool) -> Result<(), HostError>;
    safe fn __fyx_ui_context_publish_diagnostics(
        path: &FileRef,
        diagnostics: &[Diagnostic],
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_context_clear_diagnostics(path: Option<&FileRef>) -> Result<(), HostError>;
    safe fn __fyx_ui_context_decorate_files(
        decorations: &[FileDecoration],
    ) -> Result<(), HostError>;
    safe fn __fyx_ui_context_clear_file_decorations(
        paths: Option<&[FileRef]>,
    ) -> Result<(), HostError>;
}

/// What a context function did, recorded off WASM in
/// [`HostEffect::Context`](crate::HostEffect::Context).
#[derive(Debug, Clone, PartialEq)]
pub enum ContextEffect {
    Watch(bool),
    Open {
        path: FileRef,
        range: Option<TextRange>,
        focus: bool,
    },
    WatchDiagnostics(bool),
    PublishDiagnostics {
        path: FileRef,
        diagnostics: Vec<Diagnostic>,
    },
    /// `None` clears every file.
    ClearDiagnostics(Option<FileRef>),
    DecorateFiles(Vec<FileDecoration>),
    /// `None` clears every decoration.
    ClearFileDecorations(Option<Vec<FileRef>>),
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static DIAGNOSTICS: std::cell::RefCell<Vec<FileDiagnostics>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static ROOTS: std::cell::RefCell<Vec<(u64, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Off WASM, the worktrees [`to_file_ref`] and [`absolute_path`] map against on this thread:
/// (id, root as the host spells it, with `\` or `/`). For unit tests of an extension.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_worktree_roots(roots: &[(u64, &str)]) {
    ROOTS.with_borrow_mut(|known| {
        *known = roots
            .iter()
            .map(|(id, root)| (*id, (*root).to_owned()))
            .collect();
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn fake_error(code: crate::HostErrorCode, message: &str) -> HostError {
    HostError {
        code,
        message: message.to_owned(),
    }
}

/// `C:\a\b`, `file:///C:/a/b`, `file://C:\a\b` and `/a/b` as `/`-separated text, with the
/// drive in upper case.
#[cfg(not(target_arch = "wasm32"))]
fn fake_normalize(path: &str) -> String {
    let path = path
        .strip_prefix("file://")
        .unwrap_or(path)
        .replace('\\', "/");
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_owned(),
        _ => path,
    };
    let mut path = path.trim_end_matches('/').to_owned();
    if path.as_bytes().get(1) == Some(&b':') {
        path[..1].make_ascii_uppercase();
    }
    path
}

/// `context:editor`: starts or stops the editor feed.
pub fn watch(on: bool) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_watch(on);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(ContextEffect::Watch(on)))
}

/// `workspace:open`: opens an existing worktree file, selecting `range` if given.
pub fn open(path: &FileRef, range: Option<TextRange>, focus: bool) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_open(path, range, focus);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(ContextEffect::Open {
        path: path.clone(),
        range,
        focus,
    }))
}

/// `context:editor`: the worktree file at a tool's absolute path or `file://` URI. Outside
/// every open worktree is `invalid-argument`.
pub fn to_file_ref(absolute: &str) -> Result<FileRef, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_to_file_ref(absolute);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = fake_normalize(absolute);
        ROOTS
            .with_borrow(|roots| {
                roots.iter().find_map(|(id, root)| {
                    let root = fake_normalize(root);
                    let relative = path.strip_prefix(&root)?.strip_prefix('/')?;
                    (!relative.is_empty()).then(|| FileRef {
                        worktree: Some(*id),
                        path: relative.to_owned(),
                    })
                })
            })
            .ok_or_else(|| {
                fake_error(
                    crate::HostErrorCode::InvalidArgument,
                    "the path is outside every open worktree",
                )
            })
    }
}

/// `context:editor`: the absolute path of a worktree file, for a tool's reply.
pub fn absolute_path(path: &FileRef) -> Result<String, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_absolute_path(path);
    #[cfg(not(target_arch = "wasm32"))]
    ROOTS.with_borrow(|roots| {
        let root = match path.worktree {
            Some(id) => roots.iter().find(|(root, _)| *root == id),
            None if roots.len() == 1 => roots.first(),
            None => None,
        };
        let (_, root) = root.ok_or_else(|| {
            fake_error(crate::HostErrorCode::InvalidArgument, "an unknown worktree")
        })?;
        let separator = if root.contains('\\') { '\\' } else { '/' };
        let mut absolute = root.trim_end_matches(['/', '\\']).to_owned();
        for part in path.path.split('/').filter(|part| !part.is_empty()) {
            absolute.push(separator);
            absolute.push_str(part);
        }
        Ok(absolute)
    })
}

/// Off WASM, what [`diagnostics`] answers on this thread: these files, as given. For unit tests
/// of an extension.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_diagnostics(files: Vec<FileDiagnostics>) {
    DIAGNOSTICS.set(files);
}

/// `workspace:diagnostics`: the diagnostics of `path`, or of every worktree file with any
/// (`None`). A private file is `not-permitted`; a file outside every worktree is
/// `invalid-argument`; a file without diagnostics answers no files.
pub fn diagnostics(path: Option<&FileRef>) -> Result<DiagnosticsSnapshot, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_diagnostics(path);
    #[cfg(not(target_arch = "wasm32"))]
    Ok(DIAGNOSTICS.with_borrow(|files| DiagnosticsSnapshot {
        files: files
            .iter()
            .filter(|file| path.is_none_or(|path| *path == file.path))
            .cloned()
            .collect(),
        truncated: false,
    }))
}

/// `workspace:diagnostics`: starts or stops `IoEvent::DiagnosticsChanged`.
pub fn watch_diagnostics(on: bool) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_watch_diagnostics(on);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(ContextEffect::WatchDiagnostics(
        on,
    )))
}

/// `diagnostics:publish`: replaces this extension's diagnostics of `path`; an empty list clears
/// them. `source` is ignored: the host names the extension. Ranges are 0-based, columns in UTF-8
/// bytes. At most 500 per file and 5,000 in all (`invalid-argument`); over 20 calls a second is
/// `unavailable`.
pub fn publish_diagnostics(path: &FileRef, diagnostics: &[Diagnostic]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_publish_diagnostics(path, diagnostics);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(
        ContextEffect::PublishDiagnostics {
            path: path.clone(),
            diagnostics: diagnostics.to_vec(),
        },
    ))
}

/// `diagnostics:publish`: clears this extension's diagnostics of `path`, or of every file.
pub fn clear_diagnostics(path: Option<&FileRef>) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_clear_diagnostics(path);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(ContextEffect::ClearDiagnostics(
        path.cloned(),
    )))
}

/// A diagnostic for [`publish_diagnostics`] on one 0-based line, over `columns` (UTF-8 bytes).
pub fn line_diagnostic(
    line: u32,
    columns: std::ops::Range<u32>,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        range: TextRange::new(
            crate::Position::new(line, columns.start),
            crate::Position::new(line, columns.end),
        ),
        severity,
        message: message.into(),
        source: None,
        code: None,
    }
}

/// `workspace:file-decorations`: sets or replaces this extension's decoration of each path. A
/// badge has at most 2 characters; the colour is `text-accent`, `warning`, `error`, `success` or
/// `text-muted`; a tooltip at most 256 bytes. At most 500 per call and 10,000 in all.
pub fn decorate_files(decorations: &[FileDecoration]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_decorate_files(decorations);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(ContextEffect::DecorateFiles(
        decorations.to_vec(),
    )))
}

/// `workspace:file-decorations`: removes this extension's decorations of `paths`, or all.
pub fn clear_file_decorations(paths: Option<&[FileRef]>) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_context_clear_file_decorations(paths);
    #[cfg(not(target_arch = "wasm32"))]
    crate::host::record(crate::HostEffect::Context(
        ContextEffect::ClearFileDecorations(paths.map(<[FileRef]>::to_vec)),
    ))
}

/// What `register_extension!`'s context shims call, per exported world.
#[doc(hidden)]
pub mod glue {
    /// The 0.4 imports.
    pub mod newest {
        use crate::bindings::fyx::ui::context as wit;
        use crate::{FileRef, HostError, TextRange};

        pub fn watch(on: bool) -> Result<(), HostError> {
            wit::watch(on)
        }

        pub fn open(
            path: &FileRef,
            range: Option<TextRange>,
            focus: bool,
        ) -> Result<(), HostError> {
            wit::open(path, range, focus)
        }

        pub fn to_file_ref(absolute: &str) -> Result<FileRef, HostError> {
            wit::to_file_ref(absolute)
        }

        pub fn absolute_path(path: &FileRef) -> Result<String, HostError> {
            wit::absolute_path(path)
        }

        pub fn diagnostics(path: Option<&FileRef>) -> Result<wit::DiagnosticsSnapshot, HostError> {
            wit::diagnostics(path)
        }

        pub fn watch_diagnostics(on: bool) -> Result<(), HostError> {
            wit::watch_diagnostics(on)
        }

        pub fn publish_diagnostics(
            path: &FileRef,
            diagnostics: &[wit::Diagnostic],
        ) -> Result<(), HostError> {
            wit::publish_diagnostics(path, diagnostics)
        }

        pub fn clear_diagnostics(path: Option<&FileRef>) -> Result<(), HostError> {
            wit::clear_diagnostics(path)
        }

        pub fn decorate_files(decorations: &[wit::FileDecoration]) -> Result<(), HostError> {
            wit::decorate_files(decorations)
        }

        pub fn clear_file_decorations(paths: Option<&[FileRef]>) -> Result<(), HostError> {
            wit::clear_file_decorations(paths)
        }
    }

    /// Worlds without context: every call is `unavailable`.
    macro_rules! older {
        ($world:ident, $version:literal) => {
            pub mod $world {
                use crate::{FileRef, HostError, HostErrorCode, TextRange};

                fn unavailable(what: &str) -> HostError {
                    HostError {
                        code: HostErrorCode::Unavailable,
                        message: format!(
                            "context::{what} needs fyx:ui 0.4; this extension is exported as {}",
                            $version
                        ),
                    }
                }

                pub fn watch(_on: bool) -> Result<(), HostError> {
                    Err(unavailable("watch"))
                }

                pub fn open(
                    _path: &FileRef,
                    _range: Option<TextRange>,
                    _focus: bool,
                ) -> Result<(), HostError> {
                    Err(unavailable("open"))
                }

                pub fn to_file_ref(_absolute: &str) -> Result<FileRef, HostError> {
                    Err(unavailable("to_file_ref"))
                }

                pub fn absolute_path(_path: &FileRef) -> Result<String, HostError> {
                    Err(unavailable("absolute_path"))
                }

                pub fn diagnostics(
                    _path: Option<&FileRef>,
                ) -> Result<super::super::DiagnosticsSnapshot, HostError> {
                    Err(unavailable("diagnostics"))
                }

                pub fn watch_diagnostics(_on: bool) -> Result<(), HostError> {
                    Err(unavailable("watch_diagnostics"))
                }

                pub fn publish_diagnostics(
                    _path: &FileRef,
                    _diagnostics: &[super::super::Diagnostic],
                ) -> Result<(), HostError> {
                    Err(unavailable("publish_diagnostics"))
                }

                pub fn clear_diagnostics(_path: Option<&FileRef>) -> Result<(), HostError> {
                    Err(unavailable("clear_diagnostics"))
                }

                pub fn decorate_files(
                    _decorations: &[super::super::FileDecoration],
                ) -> Result<(), HostError> {
                    Err(unavailable("decorate_files"))
                }

                pub fn clear_file_decorations(_paths: Option<&[FileRef]>) -> Result<(), HostError> {
                    Err(unavailable("clear_file_decorations"))
                }
            }
        };
    }

    older!(v0_2, "0.2");
    older!(v0_1, "0.1");
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::{HostEffect, take_host_effects};

    #[test]
    fn context_calls_off_wasm() {
        take_host_effects();
        set_worktree_roots(&[(1, "/work/app"), (2, r"C:\work\lib")]);
        let app = FileRef {
            worktree: Some(1),
            path: "src/a.rs".into(),
        };
        let lib = FileRef {
            worktree: Some(2),
            path: "b.rs".into(),
        };
        assert_eq!(to_file_ref("/work/app/src/a.rs").unwrap(), app);
        assert_eq!(to_file_ref("file:///c:/work/lib/b.rs").unwrap(), lib);
        assert_eq!(to_file_ref(r"C:\work\lib\b.rs").unwrap(), lib);
        assert!(to_file_ref("/elsewhere/a").is_err());
        assert_eq!(absolute_path(&lib).unwrap(), r"C:\work\lib\b.rs");
        assert_eq!(absolute_path(&app).unwrap(), "/work/app/src/a.rs");
        watch(true).unwrap();
        open(&app, None, true).unwrap();
        assert_eq!(
            take_host_effects(),
            [
                HostEffect::Context(ContextEffect::Watch(true)),
                HostEffect::Context(ContextEffect::Open {
                    path: app,
                    range: None,
                    focus: true,
                }),
            ]
        );
        set_worktree_roots(&[]);
    }

    #[test]
    fn diagnostics_off_wasm() {
        take_host_effects();
        let file = |path: &str| FileRef {
            worktree: Some(1),
            path: path.into(),
        };
        let entry = Diagnostic {
            range: TextRange {
                start: crate::Position { line: 1, column: 0 },
                end: crate::Position { line: 1, column: 3 },
            },
            severity: Severity::Error,
            message: "no".into(),
            source: Some("rustc".into()),
            code: None,
        };
        set_diagnostics(vec![FileDiagnostics {
            path: file("a.rs"),
            diagnostics: vec![entry],
        }]);
        assert_eq!(diagnostics(None).unwrap().files.len(), 1);
        assert_eq!(diagnostics(Some(&file("a.rs"))).unwrap().files.len(), 1);
        assert!(diagnostics(Some(&file("b.rs"))).unwrap().files.is_empty());
        watch_diagnostics(true).unwrap();
        assert_eq!(
            take_host_effects(),
            [HostEffect::Context(ContextEffect::WatchDiagnostics(true))]
        );
        set_diagnostics(Vec::new());
        assert!(glue::v0_2::diagnostics(None).is_err());
    }

    #[test]
    fn older_worlds_have_no_context() {
        let error = glue::v0_2::watch(true).unwrap_err();
        assert_eq!(error.code, crate::HostErrorCode::Unavailable);
        assert!(
            error.message.contains("watch needs fyx:ui 0.4"),
            "{error:?}"
        );
        assert!(glue::v0_1::to_file_ref("/a").is_err());
    }

    #[test]
    fn publishing_and_decorating_are_recorded_off_wasm() {
        take_host_effects();
        let file = FileRef {
            worktree: Some(1),
            path: "notes.md".into(),
        };
        let todo = line_diagnostic(3, 0..4, Severity::Information, "TODO");
        assert_eq!(todo.range.start.line, 3);
        assert_eq!(todo.range.end.column, 4);
        publish_diagnostics(&file, std::slice::from_ref(&todo)).unwrap();
        clear_diagnostics(None).unwrap();
        let star = FileDecoration {
            path: file.clone(),
            badge: Some("★".into()),
            color: None,
            tooltip: Some("Bookmarked".into()),
        };
        decorate_files(std::slice::from_ref(&star)).unwrap();
        clear_file_decorations(Some(std::slice::from_ref(&file))).unwrap();
        assert_eq!(
            take_host_effects(),
            vec![
                HostEffect::Context(ContextEffect::PublishDiagnostics {
                    path: file.clone(),
                    diagnostics: vec![todo],
                }),
                HostEffect::Context(ContextEffect::ClearDiagnostics(None)),
                HostEffect::Context(ContextEffect::DecorateFiles(vec![star])),
                HostEffect::Context(ContextEffect::ClearFileDecorations(Some(vec![file]))),
            ]
        );
    }

    #[test]
    fn older_worlds_cannot_publish_or_decorate() {
        let file = FileRef {
            worktree: None,
            path: "a".into(),
        };
        let unavailable = |result: Result<(), HostError>| {
            assert_eq!(result.unwrap_err().code, crate::HostErrorCode::Unavailable);
        };
        unavailable(glue::v0_2::publish_diagnostics(&file, &[]));
        unavailable(glue::v0_1::clear_diagnostics(None));
        unavailable(glue::v0_2::decorate_files(&[]));
        unavailable(glue::v0_1::clear_file_decorations(None));
    }
}
