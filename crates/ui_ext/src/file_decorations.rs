//! fyx:ui 0.4 file decorations (`workspace:file-decorations`, T1, design wit-0.4 §F.2, plan
//! 6.7): badges, colours and tooltips on project-panel entries. The record, its checks and the
//! limits of `context.decorate-files` and `clear-file-decorations`, and the host side the runtime
//! hands the checked decorations to.
//!
//! The runtime resolves and checks every path (a worktree entry, not private) first, then
//! [`DecoratedFiles`] checks the records and the counts. The host (`ui_ext_host`, W3) writes
//! them into `project`'s `FileDecorations` global, which the project panel reads.

use std::collections::BTreeSet;

use crate::{
    process::FileRef,
    types::ColorToken,
    v0_2::{HostError, HostErrorCode},
};

pub use crate::integration::CAPABILITY_FILE_DECORATIONS;
/// §F.2: decorations one `decorate-files` sets at most.
pub const MAX_DECORATIONS_PER_CALL: usize = 500;
/// §F.2: paths one extension decorates at most.
pub const MAX_DECORATED_PATHS: usize = 10_000;
/// §F.2: a badge's characters. Counted as `char`s: a symbol with a variation selector (`✔️`) is
/// two, a flag is two, so the bound also keeps a badge to about two glyph widths.
pub const MAX_BADGE_CHARS: usize = 2;
/// §F.2.
pub const MAX_DECORATION_TOOLTIP_BYTES: usize = 256;
/// §F.2: the tokens a decoration may tint a name with; the others are `invalid-argument`.
pub const DECORATION_COLORS: [ColorToken; 5] = [
    ColorToken::TextAccent,
    ColorToken::Warning,
    ColorToken::Error,
    ColorToken::Success,
    ColorToken::TextMuted,
];

/// `context.file-decoration`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDecoration {
    pub path: FileRef,
    /// At most [`MAX_BADGE_CHARS`], drawn after the name like the git status letter.
    pub badge: Option<String>,
    /// One of [`DECORATION_COLORS`]; git status and diagnostics colours win over it.
    pub color: Option<ColorToken>,
    /// At most [`MAX_DECORATION_TOOLTIP_BYTES`], under the extension's name in the tooltip.
    pub tooltip: Option<String>,
}

impl FileDecoration {
    /// The record's own rules (§F.2); the path is the runtime's to resolve.
    pub fn check(&self) -> Result<(), String> {
        if let Some(badge) = &self.badge {
            let chars = badge.chars().count();
            if chars == 0 || chars > MAX_BADGE_CHARS {
                return Err(format!(
                    "a badge has 1 to {MAX_BADGE_CHARS} characters, not {chars}"
                ));
            }
            if badge.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err("a badge has no spaces or control characters".into());
            }
        }
        if let Some(color) = self.color
            && !DECORATION_COLORS.contains(&color)
        {
            return Err(format!(
                "a decoration's colour is text-accent, warning, error, success or text-muted, \
                 not {color:?}"
            ));
        }
        if let Some(tooltip) = &self.tooltip {
            if tooltip.len() > MAX_DECORATION_TOOLTIP_BYTES {
                return Err(format!(
                    "a decoration's tooltip is at most {MAX_DECORATION_TOOLTIP_BYTES} bytes"
                ));
            }
            if tooltip.chars().any(|c| c.is_control() && c != '\n') {
                return Err("a decoration's tooltip has no control characters".into());
            }
        }
        Ok(())
    }
}

/// The editor side of `workspace:file-decorations` (implemented in `ui_ext_host` over
/// `project::FileDecorations`). Every path passed the runtime's checks and names its worktree.
pub trait FileDecorationsHost: Send + Sync + 'static {
    /// Sets or replaces the extension's decoration of each listed path. The error is shown to
    /// the extension as `unavailable`.
    fn decorate(&self, extension_id: &str, decorations: Vec<FileDecoration>) -> Result<(), String>;
    /// Removes the extension's decorations of `paths`, or all of them with `None`: also when
    /// the instance stops, crashes, reloads or loses the capability.
    fn clear(&self, extension_id: &str, paths: Option<Vec<FileRef>>);
    /// Zed's `private_files` (`is_private_path`): such an entry is never decorated.
    fn is_private(&self, absolute: &std::path::Path) -> bool;
}

/// One instance's decorated paths.
#[derive(Debug, Default)]
pub struct DecoratedFiles {
    paths: BTreeSet<(u64, String)>,
}

impl DecoratedFiles {
    /// `decorate-files` (paths resolved: each `worktree` named): checks the call's size, each
    /// record and the extension's total, then records the paths. Nothing is recorded on an error.
    pub fn decorate(&mut self, decorations: &[FileDecoration]) -> Result<(), HostError> {
        if decorations.len() > MAX_DECORATIONS_PER_CALL {
            return Err(invalid(format!(
                "at most {MAX_DECORATIONS_PER_CALL} decorations per call"
            )));
        }
        for decoration in decorations {
            decoration
                .check()
                .map_err(|why| invalid(format!("{:?}: {why}", decoration.path.path)))?;
        }
        let new = decorations
            .iter()
            .map(|decoration| key(&decoration.path))
            .filter(|path| !self.paths.contains(path))
            .collect::<BTreeSet<_>>();
        if self.paths.len() + new.len() > MAX_DECORATED_PATHS {
            return Err(invalid(format!(
                "at most {MAX_DECORATED_PATHS} decorated paths per extension"
            )));
        }
        self.paths.extend(new);
        Ok(())
    }

    /// `clear-file-decorations`: these paths, or all with `None`.
    pub fn clear(&mut self, paths: Option<&[FileRef]>) {
        match paths {
            Some(paths) => {
                for path in paths {
                    self.paths.remove(&key(path));
                }
            }
            None => self.paths.clear(),
        }
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

fn key(path: &FileRef) -> (u64, String) {
    (path.worktree.unwrap_or_default(), path.path.clone())
}

fn invalid(message: String) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoration(path: &str) -> FileDecoration {
        FileDecoration {
            path: FileRef {
                worktree: Some(1),
                path: path.into(),
            },
            badge: Some("★".into()),
            color: Some(ColorToken::Warning),
            tooltip: Some("Bookmarked".into()),
        }
    }

    /// §F.5: a badge of 3 characters and the colour `surface` are refused.
    #[test]
    fn records_are_checked() {
        decoration("a").check().unwrap();
        let refused = |change: fn(&mut FileDecoration)| {
            let mut bad = decoration("a");
            change(&mut bad);
            bad.check().unwrap_err()
        };
        assert!(refused(|d| d.badge = Some("abc".into())).contains("not 3"));
        assert!(refused(|d| d.badge = Some(String::new())).contains("not 0"));
        assert!(refused(|d| d.badge = Some("a\u{7}".into())).contains("control"));
        assert!(refused(|d| d.color = Some(ColorToken::Surface)).contains("Surface"));
        assert!(
            refused(|d| d.tooltip = Some("x".repeat(MAX_DECORATION_TOOLTIP_BYTES + 1)))
                .contains("256")
        );
        let mut two = decoration("a");
        two.badge = Some("✔️".into());
        two.check().unwrap();
    }

    /// The per-call and per-extension caps change nothing; replacing a path does not count it
    /// twice; clearing frees the slots.
    #[test]
    fn counts_are_capped() {
        let mut decorated = DecoratedFiles::default();
        let many: Vec<_> = (0..=MAX_DECORATIONS_PER_CALL)
            .map(|index| decoration(&index.to_string()))
            .collect();
        assert_eq!(
            decorated.decorate(&many).unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
        assert!(decorated.is_empty());

        for batch in 0..(MAX_DECORATED_PATHS / MAX_DECORATIONS_PER_CALL) {
            let calls: Vec<_> = (0..MAX_DECORATIONS_PER_CALL)
                .map(|index| decoration(&format!("{batch}/{index}")))
                .collect();
            decorated.decorate(&calls).unwrap();
        }
        assert_eq!(decorated.len(), MAX_DECORATED_PATHS);
        decorated.decorate(&[decoration("0/0")]).unwrap();
        assert_eq!(
            decorated.decorate(&[decoration("new")]).unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
        decorated.clear(Some(&[decoration("0/0").path]));
        decorated.decorate(&[decoration("new")]).unwrap();
        decorated.clear(None);
        assert!(decorated.is_empty());

        let mut bad = decoration("bad");
        bad.badge = Some("abc".into());
        assert!(
            decorated
                .decorate(&[decoration("ok"), bad])
                .unwrap_err()
                .message
                .contains("\"bad\"")
        );
        assert!(decorated.is_empty());
    }
}
