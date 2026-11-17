use std::{fmt::Write as _, ops::Range, path::Path, path::PathBuf};

use anyhow::{Context as _, Result, anyhow};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HunkDecision {
    #[default]
    Pending,
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// Rows of the old text that this hunk replaces.
    pub old_rows: Range<u32>,
    /// Rows of the new text that this hunk inserts.
    pub new_rows: Range<u32>,
    pub old_text: String,
    pub new_text: String,
    pub decision: HunkDecision,
    pub comment: Option<String>,
}

impl Hunk {
    /// Fragments sent with a permission request often omit the final newline that the same lines have
    /// in the file, so decisions are matched on text with that difference ignored.
    fn same_change(&self, other: &Hunk) -> bool {
        fn with_newline(text: &str) -> std::borrow::Cow<'_, str> {
            if text.is_empty() || text.ends_with('\n') {
                text.into()
            } else {
                format!("{text}\n").into()
            }
        }
        with_newline(&self.old_text) == with_newline(&other.old_text)
            && with_newline(&self.new_text) == with_newline(&other.new_text)
    }

    fn label(&self) -> String {
        if self.old_rows.is_empty() {
            format!("insertion after line {}", self.old_rows.start)
        } else if self.old_rows.len() == 1 {
            format!("line {}", self.old_rows.start + 1)
        } else {
            format!("lines {}-{}", self.old_rows.start + 1, self.old_rows.end)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The new text equals the old text.
    Unchanged,
    Pending {
        undecided: usize,
    },
    AcceptedAll,
    RejectedAll,
    Partial {
        accepted: usize,
        rejected: usize,
    },
}

impl Resolution {
    pub fn is_pending(&self) -> bool {
        matches!(self, Resolution::Pending { .. })
    }
}

/// An agent's proposed change to one file, split into line hunks that are decided one by one.
#[derive(Clone, Debug)]
pub struct ProposedEdit {
    abs_path: PathBuf,
    old_text: String,
    new_text: String,
    hunks: Vec<Hunk>,
}

impl ProposedEdit {
    pub fn new(abs_path: PathBuf, old_text: String, new_text: String) -> Self {
        let hunks = compute_hunks(&old_text, &new_text);
        Self {
            abs_path,
            old_text,
            new_text,
            hunks,
        }
    }

    pub fn abs_path(&self) -> &Path {
        &self.abs_path
    }

    pub fn old_text(&self) -> &str {
        &self.old_text
    }

    pub fn new_text(&self) -> &str {
        &self.new_text
    }

    pub fn hunks(&self) -> &[Hunk] {
        &self.hunks
    }

    pub fn set_decision(&mut self, hunk_index: usize, decision: HunkDecision) -> Result<()> {
        self.hunk_mut(hunk_index)?.decision = decision;
        Ok(())
    }

    pub fn decide_all(&mut self, decision: HunkDecision) {
        for hunk in &mut self.hunks {
            hunk.decision = decision;
        }
    }

    pub fn set_comment(&mut self, hunk_index: usize, comment: Option<String>) -> Result<()> {
        let comment = comment
            .map(|comment| comment.trim().to_string())
            .filter(|comment| !comment.is_empty());
        self.hunk_mut(hunk_index)?.comment = comment;
        Ok(())
    }

    fn hunk_mut(&mut self, hunk_index: usize) -> Result<&mut Hunk> {
        let hunk_count = self.hunks.len();
        self.hunks
            .get_mut(hunk_index)
            .with_context(|| format!("hunk {hunk_index} out of range ({hunk_count} hunks)"))
    }

    pub fn resolution(&self) -> Resolution {
        if self.hunks.is_empty() {
            return Resolution::Unchanged;
        }
        let mut accepted = 0;
        let mut rejected = 0;
        let mut undecided = 0;
        for hunk in &self.hunks {
            match hunk.decision {
                HunkDecision::Pending => undecided += 1,
                HunkDecision::Accepted => accepted += 1,
                HunkDecision::Rejected => rejected += 1,
            }
        }
        if undecided > 0 {
            Resolution::Pending { undecided }
        } else if rejected == 0 {
            Resolution::AcceptedAll
        } else if accepted == 0 {
            Resolution::RejectedAll
        } else {
            Resolution::Partial { accepted, rejected }
        }
    }

    /// The old text with only the accepted hunks applied.
    pub fn apply_accepted(&self) -> Result<String> {
        if let Resolution::Pending { undecided } = self.resolution() {
            return Err(anyhow!("{undecided} hunks are still undecided"));
        }
        apply_hunks(&self.old_text, &self.hunks, |hunk| {
            hunk.decision == HunkDecision::Accepted
        })
    }

    /// Copies decisions and comments from an earlier review of the same change, such as the preview
    /// shown with a permission request. Returns how many hunks were decided this way.
    pub fn adopt_decisions(&mut self, earlier: &ProposedEdit) -> usize {
        let mut used = vec![false; earlier.hunks.len()];
        let mut adopted = 0;
        for hunk in &mut self.hunks {
            if hunk.decision != HunkDecision::Pending {
                continue;
            }
            let matching = earlier
                .hunks
                .iter()
                .enumerate()
                .find(|(index, earlier_hunk)| {
                    !used[*index]
                        && earlier_hunk.decision != HunkDecision::Pending
                        && earlier_hunk.same_change(hunk)
                });
            if let Some((index, earlier_hunk)) = matching {
                used[index] = true;
                hunk.decision = earlier_hunk.decision;
                hunk.comment = earlier_hunk.comment.clone();
                adopted += 1;
            }
        }
        adopted
    }

    /// What the agent is told after the review, or `None` when everything landed without comments.
    pub fn feedback(&self) -> Option<String> {
        let resolution = self.resolution();
        let has_comments = self.hunks.iter().any(|hunk| hunk.comment.is_some());
        let path = self.abs_path.display();
        let mut feedback = match resolution {
            Resolution::Unchanged | Resolution::AcceptedAll if !has_comments => return None,
            Resolution::Unchanged | Resolution::AcceptedAll => {
                format!("The user applied your edit to {path} and left comments.")
            }
            Resolution::RejectedAll => {
                format!("The user rejected your edit to {path}; the file is unchanged.")
            }
            Resolution::Partial { accepted, .. } => format!(
                "The user applied {accepted} of {} hunks of your edit to {path}; the rejected hunks \
                 were not written. Re-read the file before editing it again.",
                self.hunks.len()
            ),
            Resolution::Pending { undecided } => {
                format!("{undecided} hunks of your edit to {path} are still under review.")
            }
        };
        for (index, hunk) in self.hunks.iter().enumerate() {
            let verdict = match hunk.decision {
                HunkDecision::Rejected => "rejected",
                HunkDecision::Accepted if hunk.comment.is_some() => "applied",
                _ => continue,
            };
            write!(
                feedback,
                "\n- Hunk {} ({}) {verdict}",
                index + 1,
                hunk.label()
            )
            .ok();
            if let Some(comment) = &hunk.comment {
                write!(feedback, ": {comment}").ok();
            }
        }
        Some(feedback)
    }
}

fn compute_hunks(old_text: &str, new_text: &str) -> Vec<Hunk> {
    if old_text == new_text {
        return Vec::new();
    }
    line_hunks(old_text, new_text).unwrap_or_else(|| vec![whole_text_hunk(old_text, new_text)])
}

/// Line hunks from Zed's line diff, or `None` when they would not reproduce the new text exactly
/// (`line_diff` compares lines without their terminators, so newline-only changes are invisible to it).
fn line_hunks(old_text: &str, new_text: &str) -> Option<Vec<Hunk>> {
    let old_lines: Vec<&str> = old_text.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new_text.split_inclusive('\n').collect();
    let mut hunks = Vec::new();
    for (old_rows, new_rows) in language::line_diff(old_text, new_text) {
        let old_hunk_text = old_lines
            .get(old_rows.start as usize..old_rows.end as usize)?
            .concat();
        let new_hunk_text = new_lines
            .get(new_rows.start as usize..new_rows.end as usize)?
            .concat();
        hunks.push(Hunk {
            old_rows,
            new_rows,
            old_text: old_hunk_text,
            new_text: new_hunk_text,
            decision: HunkDecision::Pending,
            comment: None,
        });
    }
    let reproduced = apply_hunks(old_text, &hunks, |_| true).ok()?;
    (!hunks.is_empty() && reproduced == new_text).then_some(hunks)
}

fn whole_text_hunk(old_text: &str, new_text: &str) -> Hunk {
    let row_count = |text: &str| text.split_inclusive('\n').count() as u32;
    Hunk {
        old_rows: 0..row_count(old_text),
        new_rows: 0..row_count(new_text),
        old_text: old_text.to_string(),
        new_text: new_text.to_string(),
        decision: HunkDecision::Pending,
        comment: None,
    }
}

fn apply_hunks(old_text: &str, hunks: &[Hunk], include: impl Fn(&Hunk) -> bool) -> Result<String> {
    let old_lines: Vec<&str> = old_text.split_inclusive('\n').collect();
    let mut result = String::with_capacity(old_text.len());
    let mut row = 0;
    for hunk in hunks {
        let start = hunk.old_rows.start as usize;
        let unchanged = old_lines
            .get(row..start)
            .context("hunks overlap or are out of order")?;
        result.extend(unchanged.iter().copied());
        if include(hunk) {
            result.push_str(&hunk.new_text);
        } else {
            result.push_str(&hunk.old_text);
        }
        row = hunk.old_rows.end as usize;
    }
    let rest = old_lines
        .get(row..)
        .context("hunk past the end of the text")?;
    result.extend(rest.iter().copied());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "fn main() {\n    one();\n    two();\n    three();\n    four();\n}\n";
    const NEW: &str =
        "fn main() {\n    ONE();\n    two();\n    three();\n    four();\n    five();\n}\n";

    fn edit(old: &str, new: &str) -> ProposedEdit {
        ProposedEdit::new(
            PathBuf::from("/project/src/main.rs"),
            old.into(),
            new.into(),
        )
    }

    #[test]
    fn splits_separate_changes_into_hunks() {
        let edit = edit(OLD, NEW);
        let hunks = edit.hunks();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_text, "    one();\n");
        assert_eq!(hunks[0].new_text, "    ONE();\n");
        assert_eq!(hunks[1].old_rows, 5..5);
        assert_eq!(hunks[1].new_text, "    five();\n");
        assert_eq!(edit.resolution(), Resolution::Pending { undecided: 2 });
        assert!(edit.apply_accepted().is_err());
    }

    #[test]
    fn applies_only_accepted_hunks() {
        let mut edit = edit(OLD, NEW);
        edit.set_decision(0, HunkDecision::Accepted).unwrap();
        edit.set_decision(1, HunkDecision::Rejected).unwrap();
        edit.set_comment(1, Some("  not needed ".into())).unwrap();
        assert_eq!(
            edit.resolution(),
            Resolution::Partial {
                accepted: 1,
                rejected: 1
            }
        );
        assert_eq!(
            edit.apply_accepted().unwrap(),
            "fn main() {\n    ONE();\n    two();\n    three();\n    four();\n}\n"
        );
        let feedback = edit.feedback().unwrap();
        assert!(feedback.contains("applied 1 of 2 hunks"), "{feedback}");
        assert!(
            feedback.contains("Hunk 2 (insertion after line 5) rejected: not needed"),
            "{feedback}"
        );

        edit.set_decision(0, HunkDecision::Rejected).unwrap();
        assert_eq!(edit.resolution(), Resolution::RejectedAll);
        assert_eq!(edit.apply_accepted().unwrap(), OLD);

        edit.decide_all(HunkDecision::Accepted);
        edit.set_comment(1, None).unwrap();
        assert_eq!(edit.resolution(), Resolution::AcceptedAll);
        assert_eq!(edit.apply_accepted().unwrap(), NEW);
        assert_eq!(edit.feedback(), None);
    }

    #[test]
    fn out_of_range_hunk_is_an_error() {
        let mut edit = edit(OLD, NEW);
        assert!(edit.set_decision(2, HunkDecision::Accepted).is_err());
        assert!(edit.set_comment(7, Some("x".into())).is_err());
    }

    #[test]
    fn unchanged_new_and_newline_only_edits() {
        let unchanged = edit(OLD, OLD);
        assert_eq!(unchanged.resolution(), Resolution::Unchanged);
        assert_eq!(unchanged.apply_accepted().unwrap(), OLD);

        let mut created = edit("", "hello\nworld\n");
        assert_eq!(created.hunks().len(), 1);
        created.decide_all(HunkDecision::Accepted);
        assert_eq!(created.apply_accepted().unwrap(), "hello\nworld\n");

        for (old, new) in [("a\nb", "a\nb\n"), ("a\nb\n", "a\r\nb\r\n")] {
            let mut edit = edit(old, new);
            assert!(!edit.hunks().is_empty(), "{old:?} -> {new:?}");
            edit.decide_all(HunkDecision::Accepted);
            assert_eq!(edit.apply_accepted().unwrap(), new);
            edit.decide_all(HunkDecision::Rejected);
            assert_eq!(edit.apply_accepted().unwrap(), old);
        }
    }

    #[test]
    fn adopts_decisions_from_a_fragment_preview() {
        // Agents send the edited fragment with the permission request, without its final newline.
        let mut preview = edit("    one();\n    two();", "    ONE();\n    two();");
        assert_eq!(preview.hunks().len(), 1);
        preview.set_decision(0, HunkDecision::Rejected).unwrap();
        preview
            .set_comment(0, Some("keep lowercase".into()))
            .unwrap();

        let mut write = edit(OLD, NEW);
        assert_eq!(write.adopt_decisions(&preview), 1);
        assert_eq!(write.hunks()[0].decision, HunkDecision::Rejected);
        assert_eq!(write.hunks()[0].comment.as_deref(), Some("keep lowercase"));
        assert_eq!(write.hunks()[1].decision, HunkDecision::Pending);
        assert_eq!(write.resolution(), Resolution::Pending { undecided: 1 });
    }
}
