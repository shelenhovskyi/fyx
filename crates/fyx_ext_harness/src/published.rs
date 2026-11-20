//! What a 0.4 extension shows in the editor through `context` (plan 6.6 and 6.7, design.md
//! §E.1 and §F.2): its own diagnostics (`diagnostics:publish`) and its project-panel decorations
//! (`workspace:file-decorations`), with the host's checks and limits.
//!
//! The harness keeps what the extension published ([`crate::Harness::published_diagnostics`],
//! [`crate::Harness::file_decorations`]) instead of drawing it. Paths are checked as the host
//! checks them, against the worktrees and private files a test set
//! ([`crate::Harness::set_worktrees`], [`crate::Harness::set_private_file`]); each stored
//! `FileRef` names its worktree. `source` is dropped, where the host puts the extension's name.
//! Published diagnostics do not feed `context.diagnostics`, which answers from
//! [`crate::Harness::set_diagnostics`] alone.
//!
//! A badge is counted in `char`s where the host counts graphemes, so a badge of one grapheme
//! made of several `char`s (a flag, a family emoji) fails here and passes in Fyx.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ui_ext::ColorToken;
use ui_ext::context::{self as model, Diagnostic, FileDiagnostics};
use ui_ext::process::FileRef;
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::context::FakeContext;

/// Entries one `publish-diagnostics` may hold for its file.
pub const MAX_PUBLISHED_PER_FILE: usize = 500;
/// Entries one extension may have published over all files.
pub const MAX_PUBLISHED: usize = 5_000;
/// `publish-diagnostics` calls per second; more is `unavailable`.
pub const MAX_PUBLISHES_PER_SECOND: usize = 20;
/// Decorations one `decorate-files` may hold.
pub const MAX_DECORATIONS_PER_CALL: usize = 500;
/// Paths one extension may have decorated.
pub const MAX_DECORATED_PATHS: usize = 10_000;
/// Characters of a badge.
pub const MAX_BADGE_CHARS: usize = 2;
/// Bytes of a decoration's tooltip line.
pub const MAX_DECORATION_TOOLTIP_BYTES: usize = 256;

/// The colours a decoration may tint a file name with.
pub const DECORATION_COLORS: [ColorToken; 5] = [
    ColorToken::TextAccent,
    ColorToken::Warning,
    ColorToken::Error,
    ColorToken::Success,
    ColorToken::TextMuted,
];

/// `context.file-decoration`, as the host keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDecoration {
    /// Names its worktree; an empty path is the worktree's root.
    pub path: FileRef,
    pub badge: Option<String>,
    pub color: Option<ColorToken>,
    pub tooltip: Option<String>,
}

fn invalid(message: impl Into<String>) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message: message.into(),
    }
}

/// Cuts `text` to at most `max` bytes at a character boundary.
fn cut(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

#[derive(Debug, Default)]
pub(crate) struct FakePublished {
    diagnostics: Vec<FileDiagnostics>,
    /// When the publishes of the last second were made.
    publishes: VecDeque<Instant>,
    decorations: Vec<FileDecoration>,
}

impl FakePublished {
    pub(crate) fn diagnostics(&self) -> Vec<FileDiagnostics> {
        self.diagnostics.clone()
    }

    pub(crate) fn decorations(&self) -> Vec<FileDecoration> {
        self.decorations.clone()
    }

    /// `publish-diagnostics`: replaces the file's entries; an empty list clears them.
    pub(crate) fn publish(
        &mut self,
        context: &FakeContext,
        path: &FileRef,
        mut diagnostics: Vec<Diagnostic>,
    ) -> Result<(), HostError> {
        let path = context.resolve_entry(path)?;
        if path.path.is_empty() {
            return Err(invalid("diagnostics belong to a file, not a worktree root"));
        }
        if diagnostics.len() > MAX_PUBLISHED_PER_FILE {
            return Err(invalid(format!(
                "{} diagnostics for one file; at most {MAX_PUBLISHED_PER_FILE}",
                diagnostics.len()
            )));
        }
        let others: usize = self
            .diagnostics
            .iter()
            .filter(|file| file.path != path)
            .map(|file| file.diagnostics.len())
            .sum();
        if others + diagnostics.len() > MAX_PUBLISHED {
            return Err(invalid(format!(
                "{} published diagnostics in all; at most {MAX_PUBLISHED}",
                others + diagnostics.len()
            )));
        }
        let now = Instant::now();
        while self
            .publishes
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(1))
        {
            self.publishes.pop_front();
        }
        if self.publishes.len() >= MAX_PUBLISHES_PER_SECOND {
            return Err(HostError {
                code: HostErrorCode::Unavailable,
                message: format!(
                    "more than {MAX_PUBLISHES_PER_SECOND} publishes a second; retry later"
                ),
            });
        }
        self.publishes.push_back(now);
        for diagnostic in &mut diagnostics {
            diagnostic.source = None;
            cut(&mut diagnostic.message, model::MAX_DIAGNOSTIC_MESSAGE_BYTES);
            if let Some(code) = &mut diagnostic.code {
                cut(code, model::MAX_DIAGNOSTIC_LABEL_BYTES);
            }
        }
        self.diagnostics.retain(|file| file.path != path);
        if !diagnostics.is_empty() {
            self.diagnostics.push(FileDiagnostics { path, diagnostics });
        }
        Ok(())
    }

    /// `clear-diagnostics`: one file, or all of them.
    pub(crate) fn clear_diagnostics(
        &mut self,
        context: &FakeContext,
        path: Option<&FileRef>,
    ) -> Result<(), HostError> {
        match path {
            Some(path) => {
                let path = context.resolve_entry(path)?;
                self.diagnostics.retain(|file| file.path != path);
            }
            None => self.diagnostics.clear(),
        }
        Ok(())
    }

    /// `decorate-files`: every decoration is checked before any is kept.
    pub(crate) fn decorate(
        &mut self,
        context: &FakeContext,
        decorations: Vec<FileDecoration>,
    ) -> Result<(), HostError> {
        if decorations.len() > MAX_DECORATIONS_PER_CALL {
            return Err(invalid(format!(
                "{} decorations in one call; at most {MAX_DECORATIONS_PER_CALL}",
                decorations.len()
            )));
        }
        let mut checked = Vec::with_capacity(decorations.len());
        for mut decoration in decorations {
            decoration.path = context.resolve_entry(&decoration.path)?;
            if let Some(badge) = &decoration.badge
                && badge.chars().count() > MAX_BADGE_CHARS
            {
                return Err(invalid(format!(
                    "the badge {badge:?} has more than {MAX_BADGE_CHARS} characters"
                )));
            }
            if let Some(color) = decoration.color
                && !DECORATION_COLORS.contains(&color)
            {
                return Err(invalid(format!(
                    "{color:?} is not a decoration colour (text-accent, warning, error, success, \
                     text-muted)"
                )));
            }
            if let Some(tooltip) = &decoration.tooltip
                && tooltip.len() > MAX_DECORATION_TOOLTIP_BYTES
            {
                return Err(invalid(format!(
                    "a {}-byte tooltip; at most {MAX_DECORATION_TOOLTIP_BYTES}",
                    tooltip.len()
                )));
            }
            checked.push(decoration);
        }
        let mut decorated = self.decorations.clone();
        for decoration in checked {
            match decorated
                .iter_mut()
                .find(|kept| kept.path == decoration.path)
            {
                Some(kept) => *kept = decoration,
                None => decorated.push(decoration),
            }
        }
        if decorated.len() > MAX_DECORATED_PATHS {
            return Err(invalid(format!(
                "{} decorated paths; at most {MAX_DECORATED_PATHS}",
                decorated.len()
            )));
        }
        self.decorations = decorated;
        Ok(())
    }

    /// `clear-file-decorations`: these paths, or all.
    pub(crate) fn clear_decorations(
        &mut self,
        context: &FakeContext,
        paths: Option<Vec<FileRef>>,
    ) -> Result<(), HostError> {
        match paths {
            Some(paths) => {
                let paths = paths
                    .iter()
                    .map(|path| context.resolve_entry(path))
                    .collect::<Result<Vec<_>, _>>()?;
                self.decorations
                    .retain(|decoration| !paths.contains(&decoration.path));
            }
            None => self.decorations.clear(),
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::context::DiagnosticSeverity;
    use ui_ext::edit::{Position, TextRange};

    fn context() -> FakeContext {
        let mut context = FakeContext::default();
        context.set_roots(vec![(1, "/work".into())]);
        context.set_private(1, ".env");
        context
    }

    fn file(path: &str) -> FileRef {
        FileRef {
            worktree: None,
            path: path.into(),
        }
    }

    fn named(path: &str) -> FileRef {
        FileRef {
            worktree: Some(1),
            path: path.into(),
        }
    }

    fn todo(line: u32) -> Diagnostic {
        Diagnostic {
            range: TextRange {
                start: Position { line, column: 0 },
                end: Position { line, column: 4 },
            },
            severity: DiagnosticSeverity::Information,
            message: "TODO".into(),
            source: Some("rust-analyzer".into()),
            code: None,
        }
    }

    fn code(result: Result<(), HostError>) -> HostErrorCode {
        match result {
            Ok(()) => panic!("expected an error"),
            Err(error) => error.code,
        }
    }

    #[test]
    fn published_diagnostics_replace_per_file_and_name_no_source() {
        let context = context();
        let mut published = FakePublished::default();
        published
            .publish(&context, &file("a.rs"), vec![todo(0), todo(3)])
            .unwrap();
        published
            .publish(&context, &named("a.rs"), vec![todo(5)])
            .unwrap();
        published
            .publish(&context, &file("b.rs"), vec![todo(1)])
            .unwrap();
        let diagnostics = published.diagnostics();
        assert_eq!(
            diagnostics
                .iter()
                .map(|file| (file.path.clone(), file.diagnostics.len()))
                .collect::<Vec<_>>(),
            [(named("a.rs"), 1), (named("b.rs"), 1)]
        );
        assert_eq!(diagnostics[0].diagnostics[0].range.start.line, 5);
        assert_eq!(diagnostics[0].diagnostics[0].source, None);

        published
            .publish(&context, &file("b.rs"), Vec::new())
            .unwrap();
        assert_eq!(published.diagnostics().len(), 1);
        published
            .clear_diagnostics(&context, Some(&file("a.rs")))
            .unwrap();
        assert!(published.diagnostics().is_empty());
    }

    #[test]
    fn publishing_is_checked_like_the_host() {
        let context = context();
        let mut published = FakePublished::default();
        let outside = FileRef {
            worktree: Some(9),
            path: "a.rs".into(),
        };
        assert_eq!(
            code(published.publish(&context, &outside, vec![todo(0)])),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(published.publish(&context, &file(".env"), vec![todo(0)])),
            HostErrorCode::NotPermitted
        );
        assert_eq!(
            code(published.publish(&context, &file(""), vec![todo(0)])),
            HostErrorCode::InvalidArgument
        );
        assert_eq!(
            code(published.publish(
                &context,
                &file("a.rs"),
                vec![todo(0); MAX_PUBLISHED_PER_FILE + 1]
            )),
            HostErrorCode::InvalidArgument
        );
        assert!(published.diagnostics().is_empty());

        let mut long = todo(0);
        long.message = "\u{e9}".repeat(model::MAX_DIAGNOSTIC_MESSAGE_BYTES);
        published
            .publish(&context, &file("a.rs"), vec![long])
            .unwrap();
        let message = &published.diagnostics()[0].diagnostics[0].message;
        assert!(message.len() <= model::MAX_DIAGNOSTIC_MESSAGE_BYTES);
    }

    #[test]
    fn an_extension_has_at_most_5000_diagnostics_and_20_publishes_a_second() {
        let context = context();
        let mut published = FakePublished::default();
        for index in 0..MAX_PUBLISHED / MAX_PUBLISHED_PER_FILE {
            published.publishes.clear();
            published
                .publish(
                    &context,
                    &file(&format!("{index}.rs")),
                    vec![todo(0); MAX_PUBLISHED_PER_FILE],
                )
                .unwrap();
        }
        assert_eq!(
            code(published.publish(&context, &file("more.rs"), vec![todo(0)])),
            HostErrorCode::InvalidArgument
        );
        // Replacing a file's entries counts the new ones only.
        published
            .publish(&context, &file("0.rs"), vec![todo(0)])
            .unwrap();

        let mut fresh = FakePublished::default();
        for _ in 0..MAX_PUBLISHES_PER_SECOND {
            fresh
                .publish(&context, &file("a.rs"), vec![todo(0)])
                .unwrap();
        }
        assert_eq!(
            code(fresh.publish(&context, &file("a.rs"), vec![todo(0)])),
            HostErrorCode::Unavailable
        );
    }

    fn decoration(path: &str) -> FileDecoration {
        FileDecoration {
            path: file(path),
            badge: Some("\u{2605}".into()),
            color: Some(ColorToken::TextAccent),
            tooltip: Some("Bookmarked".into()),
        }
    }

    #[test]
    fn decorations_are_checked_and_replace_per_path() {
        let context = context();
        let mut published = FakePublished::default();
        published
            .decorate(&context, vec![decoration("a.rs"), decoration("")])
            .unwrap();
        let mut plain = decoration("a.rs");
        plain.badge = None;
        published.decorate(&context, vec![plain]).unwrap();
        let kept = published.decorations();
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].path, named("a.rs"));
        assert_eq!(kept[0].badge, None);

        let mut long_badge = decoration("b.rs");
        long_badge.badge = Some("abc".into());
        let mut wrong_color = decoration("b.rs");
        wrong_color.color = Some(ColorToken::Surface);
        let mut long_tooltip = decoration("b.rs");
        long_tooltip.tooltip = Some("x".repeat(MAX_DECORATION_TOOLTIP_BYTES + 1));
        let mut private = decoration("b.rs");
        private.path = file(".env");
        for (bad, expected) in [
            (long_badge, HostErrorCode::InvalidArgument),
            (wrong_color, HostErrorCode::InvalidArgument),
            (long_tooltip, HostErrorCode::InvalidArgument),
            (private, HostErrorCode::NotPermitted),
        ] {
            // The good one in the same call is not kept either.
            assert_eq!(
                code(published.decorate(&context, vec![decoration("c.rs"), bad])),
                expected
            );
        }
        assert_eq!(published.decorations(), kept);

        assert_eq!(
            code(published.decorate(
                &context,
                vec![decoration("a.rs"); MAX_DECORATIONS_PER_CALL + 1]
            )),
            HostErrorCode::InvalidArgument
        );

        published
            .clear_decorations(&context, Some(vec![file("a.rs")]))
            .unwrap();
        assert_eq!(published.decorations().len(), 1);
        published.clear_decorations(&context, None).unwrap();
        assert!(published.decorations().is_empty());
    }
}
