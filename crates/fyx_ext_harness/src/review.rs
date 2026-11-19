//! A fake agent review for the staged `fyx:ui` 0.4 `review` (design.md B.10), with the checks of
//! W2c's runtime (`ui_ext_wasm::proposals`, plan 5.13): the labels and sizes, a worktree file
//! that is neither protected nor private, at most [`MAX_OPEN_PROPOSALS`] open, ids that are
//! never reused, and each outcome delivered at most once, never after `withdraw`.
//!
//! A test reads the open proposals ([`crate::Harness::proposals`]) and decides them as the user
//! would ([`crate::Harness::decide_review`], [`crate::Harness::fail_review`]);
//! [`crate::Harness::deliver_io`] then delivers `review-decided`. `withdraw` returns the
//! proposal's current text: what was proposed, or what [`crate::Harness::edit_review`] set.
//! Unlike the host, the harness does not check that a diff's file exists or that the diff
//! applies.

use std::collections::{BTreeMap, VecDeque};

use ui_ext::edit::is_protected_path;
use ui_ext::process::FileRef;
use ui_ext::protected::is_default_private;
use ui_ext::review::{
    MAX_OPEN_PROPOSALS, MAX_PROPOSAL_BYTES, MAX_PROPOSAL_LABEL_BYTES, MAX_TURN_PATHS, Proposal,
    Proposed, ReviewOutcome,
};
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::context::FakeContext;

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

fn check_label(what: &str, label: &str) -> Result<(), HostError> {
    if label.is_empty() || label.len() > MAX_PROPOSAL_LABEL_BYTES {
        return Err(error(
            HostErrorCode::InvalidArgument,
            format!("{what} must be 1 to {MAX_PROPOSAL_LABEL_BYTES} bytes"),
        ));
    }
    Ok(())
}

/// A `begin-turn` or `end-turn` the extension made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewTurn {
    Begin(String),
    End(String, Vec<FileRef>),
}

#[derive(Debug)]
struct Open {
    proposal: Proposal,
    current: String,
}

#[derive(Debug)]
pub(crate) struct FakeReview {
    next_id: u64,
    open: BTreeMap<u64, Open>,
    pub(crate) withdrawn: Vec<u64>,
    pub(crate) turns: Vec<ReviewTurn>,
    pub(crate) events: VecDeque<(u64, Result<ReviewOutcome, String>)>,
}

impl Default for FakeReview {
    fn default() -> Self {
        Self {
            next_id: 1,
            open: BTreeMap::new(),
            withdrawn: Vec::new(),
            turns: Vec::new(),
            events: VecDeque::new(),
        }
    }
}

impl FakeReview {
    pub(crate) fn propose(
        &mut self,
        context: &FakeContext,
        proposal: Proposal,
    ) -> Result<u64, HostError> {
        check_label("title", &proposal.title)?;
        check_label("session", &proposal.session)?;
        let (Proposed::NewContent(text) | Proposed::UnifiedDiff(text)) = &proposal.change;
        if text.len() > MAX_PROPOSAL_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("a proposal is at most {MAX_PROPOSAL_BYTES} bytes"),
            ));
        }
        let path = &proposal.path;
        path.check()?;
        if path.path.is_empty() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                "a proposal names a file",
            ));
        }
        if is_protected_path(&path.path) {
            return Err(error(
                HostErrorCode::NotPermitted,
                format!("{:?} is protected: it can make a program run", path.path),
            ));
        }
        if is_default_private(&path.path) || context.is_private(path) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        context.absolute_path(path)?;
        if self.open.len() >= MAX_OPEN_PROPOSALS {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("at most {MAX_OPEN_PROPOSALS} proposals may be open"),
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        let current = text.clone();
        self.open.insert(id, Open { proposal, current });
        Ok(id)
    }

    pub(crate) fn withdraw(&mut self, id: u64) -> Result<String, HostError> {
        let open = self.open.remove(&id).ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                format!("{id} is not an undecided proposal of this extension"),
            )
        })?;
        self.withdrawn.push(id);
        Ok(open.current)
    }

    pub(crate) fn begin_turn(&mut self, session: String) -> Result<(), HostError> {
        check_label("session", &session)?;
        self.turns.push(ReviewTurn::Begin(session));
        Ok(())
    }

    pub(crate) fn end_turn(
        &mut self,
        context: &FakeContext,
        session: String,
        changed: Vec<FileRef>,
    ) -> Result<(), HostError> {
        check_label("session", &session)?;
        if changed.len() > MAX_TURN_PATHS {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("a turn names at most {MAX_TURN_PATHS} files"),
            ));
        }
        for file in &changed {
            file.check()?;
            context.absolute_path(file)?;
        }
        self.turns.push(ReviewTurn::End(session, changed));
        Ok(())
    }

    pub(crate) fn proposals(&self) -> Vec<(u64, Proposal)> {
        self.open
            .iter()
            .map(|(id, open)| (*id, open.proposal.clone()))
            .collect()
    }

    pub(crate) fn edit(&mut self, id: u64, text: &str) -> bool {
        self.open
            .get_mut(&id)
            .map(|open| open.current = text.to_owned())
            .is_some()
    }

    /// The review ends; its outcome is queued once. False when `id` is not open.
    pub(crate) fn end(&mut self, id: u64, outcome: Result<ReviewOutcome, String>) -> bool {
        if self.open.remove(&id).is_none() {
            return false;
        }
        self.events.push_back((id, outcome));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext::review::Decision;

    fn file(path: &str) -> FileRef {
        FileRef {
            worktree: Some(1),
            path: path.into(),
        }
    }

    fn proposal(path: &str) -> Proposal {
        Proposal {
            path: file(path),
            change: Proposed::NewContent("new\n".into()),
            title: "Edit".into(),
            session: "s".into(),
            apply: false,
        }
    }

    #[test]
    fn the_runtime_checks_hold() {
        let mut context = FakeContext::default();
        context.set_roots(vec![(1, "/work/app".into())]);
        context.set_private(1, "notes.md");
        let mut review = FakeReview::default();
        let code = |result: Result<u64, HostError>| result.unwrap_err().code;
        assert_eq!(
            code(review.propose(&context, proposal(".env"))),
            HostErrorCode::NotPermitted
        );
        assert_eq!(
            code(review.propose(&context, proposal("notes.md"))),
            HostErrorCode::NotPermitted
        );
        assert_eq!(
            code(review.propose(&context, proposal("../a.rs"))),
            HostErrorCode::InvalidArgument
        );
        let mut untitled = proposal("a.rs");
        untitled.title.clear();
        assert!(review.propose(&context, untitled).is_err());
        let mut elsewhere = proposal("a.rs");
        elsewhere.path.worktree = Some(9);
        assert!(review.propose(&context, elsewhere).is_err());

        let first = review.propose(&context, proposal("a.rs")).unwrap();
        let second = review.propose(&context, proposal("b.rs")).unwrap();
        assert_ne!(first, second);
        assert!(review.edit(first, "edited\n"));
        assert_eq!(review.withdraw(first).unwrap(), "edited\n");
        assert!(review.withdraw(first).is_err(), "withdrawn once");
        assert!(
            !review.end(first, Err("gone".into())),
            "no outcome after withdraw"
        );
        let outcome = ReviewOutcome {
            decision: Decision::Rejected,
            final_content: String::new(),
        };
        assert!(review.end(second, Ok(outcome.clone())));
        assert!(!review.end(second, Ok(outcome)), "an outcome arrives once");
        assert_eq!(review.events.len(), 1);
        assert!(review.withdraw(second).is_err(), "decided");
        assert!(
            review
                .end_turn(&context, "s".into(), vec![file("../x")])
                .is_err()
        );
    }
}
