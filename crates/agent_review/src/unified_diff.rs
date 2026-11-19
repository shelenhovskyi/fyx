//! Proposals sent as a unified diff (`fyx:ui` 0.4 `proposed::unified-diff`): the patch is applied to
//! the buffer's current text, and refused when it does not apply there.

use std::fmt;

use anyhow::{Context as _, Result, ensure};
use gpui::{App, Entity, SharedString};
use language::Buffer;
use std::path::PathBuf;

use acp_thread::fyx_review::AgentWrite;
use agent_client_protocol::v2::schema as acp_v2;

use crate::ExtensionProposal;

/// Why a unified diff cannot become a proposal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    /// The text is not a unified diff of one file.
    Invalid(String),
    /// Hunk `hunk` (1-based) does not match the current text: the file changed since the diff was
    /// made, or the diff was made against another file.
    Conflict { hunk: usize },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Invalid(reason) => write!(f, "invalid unified diff: {reason}"),
            PatchError::Conflict { hunk } => write!(
                f,
                "hunk {hunk} of the diff does not apply to the file's current text"
            ),
        }
    }
}

impl std::error::Error for PatchError {}

/// Applies a unified diff of one file to `old_text`. Every hunk's context and removed lines must
/// match the text exactly; a hunk may sit at other lines than its header says (no fuzz). Headers
/// (`diff --git`, `---`/`+++`) are optional and their paths are ignored. Pure and `Send`, so a
/// caller may run it off the main thread.
pub fn apply_unified_diff(old_text: &str, patch: &str) -> Result<String, PatchError> {
    check_patch(patch).map_err(|error| PatchError::Invalid(format!("{error:#}")))?;
    // `language::apply_diff_patch` is diffy's parse and apply. A parse error carries diffy's
    // message under a context; an apply error is only diffy's "error applying hunk #N".
    language::apply_diff_patch(old_text, patch).map_err(|error| {
        if error.chain().count() > 1 {
            return PatchError::Invalid(format!("{error:#}"));
        }
        let message = error.to_string();
        match message
            .strip_prefix("error applying hunk #")
            .and_then(|number| number.trim().parse().ok())
        {
            Some(hunk) => PatchError::Conflict { hunk },
            None => PatchError::Invalid(message),
        }
    })
}

fn check_patch(patch: &str) -> Result<()> {
    ensure!(
        patch.starts_with("diff --git ") || patch.starts_with("--- ") || patch.starts_with("@@ "),
        "expected `diff --git`, `---` or `@@` at the start"
    );
    // A header line can only start a line in the header: hunk lines start with ' ', '-' or '+'.
    let files = patch
        .lines()
        .filter(|line| line.starts_with("diff --git "))
        .count();
    ensure!(files <= 1, "the diff changes more than one file");
    ensure!(
        patch.lines().any(|line| line.starts_with("@@ ")),
        "the diff has no hunks"
    );
    // diffy adds usize coordinates while checking hunk order: bound them first, so an untrusted
    // header cannot overflow there.
    for line in patch.lines().filter(|line| line.starts_with("@@ ")) {
        for range in line.split_whitespace().skip(1).take(2) {
            let range = range.trim_start_matches(['-', '+']);
            let (start, count) = range.split_once(',').unwrap_or((range, "1"));
            let start = start.parse::<u32>().context("invalid hunk start")?;
            let count = count.parse::<u32>().context("invalid hunk length")?;
            ensure!(
                start > 0 || count == 0,
                "a nonempty hunk starts at line zero"
            );
            start
                .checked_add(count)
                .context("hunk coordinates overflow")?;
        }
    }
    Ok(())
}

impl ExtensionProposal {
    /// A proposal from a unified diff, applied to the buffer's current text (unsaved edits
    /// included), which becomes the proposal's old text. Fails with [`PatchError`] when the diff is
    /// invalid or does not apply.
    pub fn from_unified_diff(
        session_id: acp_v2::SessionId,
        abs_path: PathBuf,
        buffer: Entity<Buffer>,
        patch: &str,
        extension: SharedString,
        title: SharedString,
        cx: &App,
    ) -> Result<Self, PatchError> {
        let old_text = buffer.read(cx).text();
        let new_text = apply_unified_diff(&old_text, patch)?;
        Ok(Self {
            write: AgentWrite {
                session_id,
                abs_path,
                buffer,
                old_text,
                new_text,
            },
            extension,
            title,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "one\ntwo\nthree\nfour\nfive\n";

    #[test]
    fn applies_a_diff_with_and_without_headers() {
        let patch = "--- a/x.txt\n+++ b/x.txt\n@@ -1,2 +1,2 @@\n-one\n+ONE\n two\n";
        assert_eq!(
            apply_unified_diff(OLD, patch).unwrap(),
            "ONE\ntwo\nthree\nfour\nfive\n"
        );
        let patch = "@@ -4,2 +4,2 @@\n four\n-five\n+FIVE\n";
        assert_eq!(
            apply_unified_diff(OLD, patch).unwrap(),
            "one\ntwo\nthree\nfour\nFIVE\n"
        );
    }

    #[test]
    fn a_hunk_may_sit_elsewhere_than_its_header_says() {
        let patch = "@@ -1,2 +1,2 @@\n three\n-four\n+FOUR\n";
        assert_eq!(
            apply_unified_diff(OLD, patch).unwrap(),
            "one\ntwo\nthree\nFOUR\nfive\n"
        );
    }

    #[test]
    fn a_diff_of_other_text_is_a_conflict() {
        let patch = "@@ -1,2 +1,2 @@\n one\n-zwei\n+TWO\n@@ -4,1 +4,1 @@\n-four\n+FOUR\n";
        assert_eq!(
            apply_unified_diff(OLD, patch),
            Err(PatchError::Conflict { hunk: 1 })
        );
        let patch = "@@ -1,1 +1,1 @@\n-one\n+ONE\n@@ -4,1 +4,1 @@\n-vier\n+FOUR\n";
        assert_eq!(
            apply_unified_diff(OLD, patch),
            Err(PatchError::Conflict { hunk: 2 })
        );
    }

    #[test]
    fn refuses_what_is_not_a_diff_of_one_file() {
        for patch in [
            "",
            "hello",
            "--- a/x\n+++ b/x\n",
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-one\n+ONE\n\
             diff --git a/y b/y\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-one\n+ONE\n",
            "@@ -4294967295,2 +1,1 @@\n-one\n+ONE\n",
            "@@ -0,1 +1,1 @@\n-one\n+ONE\n",
            "@@ -x +1 @@\n-one\n+ONE\n",
        ] {
            assert!(
                matches!(apply_unified_diff(OLD, patch), Err(PatchError::Invalid(_))),
                "{patch:?}"
            );
        }
    }
}
