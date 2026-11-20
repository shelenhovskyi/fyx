//! `fyx:ui` 0.4: proposed edits through agent review (design.md B.10), with the
//! `fyx-ui-0-4` feature and the `review:propose` capability.
//!
//! - [`propose`] opens a review of one worktree file and returns its id. The user decides in
//!   agent review; the outcome arrives once in [`Extension::handle_io`](crate::Extension) as
//!   `IoEvent::ReviewDecided((id, outcome))`: `Ok` with the decision and the text after the
//!   user's choices, or `Err` with why the review ended undecided (the file went away). With
//!   `apply`, the host applies the accepted hunks to the buffer as one undo step and never saves.
//! - [`withdraw`] closes an undecided review (the tool closed its diff) and returns its current
//!   text; no outcome follows. A decided or unknown id is `invalid-argument`.
//! - [`begin_turn`] and [`end_turn`] bracket a tool session's turn, so files it changed without
//!   a proposal show as "changed without review".
//!
//! Protected files (ones that can make a program run) and private ones are `not-permitted`; at
//! most 64 proposals are open, each at most 8 MiB. Exported as 0.2 or 0.1, every function here
//! fails with `unavailable`. Off WASM each one records a
//! [`HostEffect::Review`](crate::HostEffect::Review); [`propose`] returns ids from 1 and
//! [`withdraw`] returns the proposed text of a proposal this thread has not withdrawn.

pub use crate::bindings::fyx::ui::review::{Decision, Proposal, Proposed, ReviewOutcome};
use crate::{FileRef, HostError};

#[cfg(target_arch = "wasm32")]
unsafe extern "Rust" {
    safe fn __fyx_ui_review_propose(request: &Proposal) -> Result<u64, HostError>;
    safe fn __fyx_ui_review_withdraw(id: u64) -> Result<String, HostError>;
    safe fn __fyx_ui_review_begin_turn(session: &str) -> Result<(), HostError>;
    safe fn __fyx_ui_review_end_turn(session: &str, changed: &[FileRef]) -> Result<(), HostError>;
}

/// What a review function did, recorded off WASM in
/// [`HostEffect::Review`](crate::HostEffect::Review).
#[derive(Debug, Clone, PartialEq)]
pub enum ReviewEffect {
    Propose {
        id: u64,
        proposal: Proposal,
    },
    Withdraw(u64),
    BeginTurn(String),
    EndTurn {
        session: String,
        changed: Vec<FileRef>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The proposals this thread has open, with their text; the next id.
    static OPEN: std::cell::RefCell<(u64, std::collections::BTreeMap<u64, String>)> =
        const { std::cell::RefCell::new((1, std::collections::BTreeMap::new())) };
}

#[cfg(not(target_arch = "wasm32"))]
fn record(effect: ReviewEffect) -> Result<(), HostError> {
    crate::host::record(crate::HostEffect::Review(effect))
}

impl Proposal {
    /// A proposal of `new_content` for `path`, which the host does not apply.
    pub fn new_content(
        path: FileRef,
        new_content: impl Into<String>,
        title: impl Into<String>,
        session: impl Into<String>,
    ) -> Self {
        Self {
            path,
            change: Proposed::NewContent(new_content.into()),
            title: title.into(),
            session: session.into(),
            apply: false,
        }
    }
}

/// `review:propose`: opens a review and returns its id; see the module docs.
pub fn propose(request: &Proposal) -> Result<u64, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_review_propose(request);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let text = match &request.change {
            Proposed::NewContent(text) | Proposed::UnifiedDiff(text) => text.clone(),
        };
        let id = OPEN.with_borrow_mut(|(next, open)| {
            let id = *next;
            *next += 1;
            open.insert(id, text);
            id
        });
        record(ReviewEffect::Propose {
            id,
            proposal: request.clone(),
        })?;
        Ok(id)
    }
}

/// Closes the undecided review `id` and returns its current text, the user's edits included.
pub fn withdraw(id: u64) -> Result<String, HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_review_withdraw(id);
    #[cfg(not(target_arch = "wasm32"))]
    {
        record(ReviewEffect::Withdraw(id))?;
        OPEN.with_borrow_mut(|(_, open)| open.remove(&id))
            .ok_or_else(|| HostError {
                code: crate::HostErrorCode::InvalidArgument,
                message: format!("{id} is not an undecided proposal of this extension"),
            })
    }
}

/// A tool session's turn begins.
pub fn begin_turn(session: &str) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_review_begin_turn(session);
    #[cfg(not(target_arch = "wasm32"))]
    record(ReviewEffect::BeginTurn(session.to_owned()))
}

/// A tool session's turn ends, with the worktree files it changed (at most 1024).
pub fn end_turn(session: &str, changed: &[FileRef]) -> Result<(), HostError> {
    #[cfg(target_arch = "wasm32")]
    return __fyx_ui_review_end_turn(session, changed);
    #[cfg(not(target_arch = "wasm32"))]
    record(ReviewEffect::EndTurn {
        session: session.to_owned(),
        changed: changed.to_vec(),
    })
}

/// What `register_extension!`'s review shims call, per exported world.
#[doc(hidden)]
pub mod glue {
    /// The 0.4 imports.
    pub mod newest {
        use super::super::Proposal;
        use crate::bindings::fyx::ui::review as wit;
        use crate::{FileRef, HostError};

        pub fn propose(request: &Proposal) -> Result<u64, HostError> {
            wit::propose(request)
        }

        pub fn withdraw(id: u64) -> Result<String, HostError> {
            wit::withdraw(id)
        }

        pub fn begin_turn(session: &str) -> Result<(), HostError> {
            wit::begin_turn(session)
        }

        pub fn end_turn(session: &str, changed: &[FileRef]) -> Result<(), HostError> {
            wit::end_turn(session, changed)
        }
    }

    /// Worlds without review: every call is `unavailable`.
    macro_rules! older {
        ($world:ident, $version:literal) => {
            pub mod $world {
                use super::super::Proposal;
                use crate::{FileRef, HostError, HostErrorCode};

                fn unavailable(what: &str) -> HostError {
                    HostError {
                        code: HostErrorCode::Unavailable,
                        message: format!(
                            "review::{what} needs fyx:ui 0.4; this extension is exported as {}",
                            $version
                        ),
                    }
                }

                pub fn propose(_request: &Proposal) -> Result<u64, HostError> {
                    Err(unavailable("propose"))
                }

                pub fn withdraw(_id: u64) -> Result<String, HostError> {
                    Err(unavailable("withdraw"))
                }

                pub fn begin_turn(_session: &str) -> Result<(), HostError> {
                    Err(unavailable("begin_turn"))
                }

                pub fn end_turn(_session: &str, _changed: &[FileRef]) -> Result<(), HostError> {
                    Err(unavailable("end_turn"))
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
    fn review_calls_off_wasm() {
        take_host_effects();
        let file = FileRef {
            worktree: Some(1),
            path: "src/a.rs".into(),
        };
        let proposal = Proposal::new_content(file.clone(), "fn a() {}\n", "Edit a.rs", "s1");
        let id = propose(&proposal).unwrap();
        assert_eq!(withdraw(id).unwrap(), "fn a() {}\n");
        assert_eq!(
            withdraw(id).unwrap_err().code,
            crate::HostErrorCode::InvalidArgument
        );
        begin_turn("s1").unwrap();
        end_turn("s1", std::slice::from_ref(&file)).unwrap();
        assert_eq!(
            take_host_effects(),
            [
                HostEffect::Review(ReviewEffect::Propose { id, proposal }),
                HostEffect::Review(ReviewEffect::Withdraw(id)),
                HostEffect::Review(ReviewEffect::Withdraw(id)),
                HostEffect::Review(ReviewEffect::BeginTurn("s1".into())),
                HostEffect::Review(ReviewEffect::EndTurn {
                    session: "s1".into(),
                    changed: vec![file],
                }),
            ]
        );
    }

    #[test]
    fn older_worlds_have_no_review() {
        let error = glue::v0_2::begin_turn("s").unwrap_err();
        assert_eq!(error.code, crate::HostErrorCode::Unavailable);
        assert!(
            error.message.contains("begin_turn needs fyx:ui 0.4"),
            "{error:?}"
        );
        assert!(glue::v0_1::withdraw(1).is_err());
    }
}
