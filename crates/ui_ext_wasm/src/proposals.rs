//! The fyx:ui 0.4 `review` import over [`ReviewHost`] (design wit-0.4 §B.10, plan 5.13):
//! `review:propose`. This side checks the capability, the paths (a worktree file, never a
//! protected or default-private one), the sizes and the open-proposal limit, hands out ids and
//! makes sure each proposal's outcome reaches `handle-io` at most once, never after `withdraw`
//! or the instance's end. The bridge in `ui_ext_host` opens the reviews over `agent_review`.

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use futures::{StreamExt as _, channel::mpsc};
use ui_ext::{
    edit::is_protected_path,
    process::FileRef,
    protected::is_default_private,
    review::{
        MAX_OPEN_PROPOSALS, MAX_PROPOSAL_BYTES, MAX_PROPOSAL_LABEL_BYTES, MAX_TURN_PATHS, Proposal,
        Proposed, ReviewHost, ReviewOutcome,
    },
    v0_2::{HostError, HostErrorCode},
};

use crate::{
    editor_context::{absolute, roots},
    network::{IoEvent, Running},
    process::Workspace,
};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

/// Proposal ids, unique in the process: a reloaded instance never reuses one.
static NEXT_PROPOSAL: AtomicU64 = AtomicU64::new(1);

type Decided = (u64, Result<ReviewOutcome, String>);

/// One instance's proposals.
pub(crate) struct ReviewHandle {
    extension_id: Arc<str>,
    host: Option<Arc<dyn ReviewHost>>,
    workspace: Option<Arc<dyn Workspace>>,
    /// The undecided proposals: an outcome passes only while its id is here.
    open: Arc<Mutex<BTreeSet<u64>>>,
    sender: mpsc::UnboundedSender<Decided>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Decided>>>,
}

impl ReviewHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        host: Option<Arc<dyn ReviewHost>>,
        workspace: Option<Arc<dyn Workspace>>,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded();
        Self {
            extension_id,
            host,
            workspace,
            open: Arc::default(),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.host.is_some()
    }

    fn host(&self) -> Result<&Arc<dyn ReviewHost>, HostError> {
        self.host.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "agent review is not available in this window",
            )
        })
    }

    fn open(&self) -> std::sync::MutexGuard<'_, BTreeSet<u64>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Moves the outcomes onto the io channel, on the current tokio runtime. Once.
    pub(crate) fn forward(&self, running: &Arc<Running>) {
        let Some(mut receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let running = Arc::downgrade(running);
        tokio::spawn(async move {
            while let Some((id, result)) = receiver.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::ReviewDecided(id, result)).await;
            }
        });
    }

    /// `review.propose`.
    pub(crate) fn propose(&self, proposal: Proposal) -> Result<u64, HostError> {
        let host = self.host()?;
        check_label("title", &proposal.title)?;
        check_label("session", &proposal.session)?;
        let size = match &proposal.change {
            Proposed::NewContent(text) | Proposed::UnifiedDiff(text) => text.len(),
        };
        if size > MAX_PROPOSAL_BYTES {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("a proposal is at most {MAX_PROPOSAL_BYTES} bytes"),
            ));
        }
        let full = self.file(&proposal.path)?;
        if matches!(proposal.change, Proposed::UnifiedDiff(_)) && !full.is_file() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!(
                    "a diff needs an existing file; {:?} is not one",
                    proposal.path.path
                ),
            ));
        }
        if full.is_dir() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{:?} is a folder", proposal.path.path),
            ));
        }
        let id = {
            let mut open = self.open();
            if open.len() >= MAX_OPEN_PROPOSALS {
                return Err(error(
                    HostErrorCode::LimitExceeded,
                    format!("at most {MAX_OPEN_PROPOSALS} proposals may be open"),
                ));
            }
            let id = NEXT_PROPOSAL.fetch_add(1, Ordering::Relaxed);
            open.insert(id);
            id
        };
        let open = Arc::downgrade(&self.open);
        let sender = self.sender.clone();
        let sink = Arc::new(move |id: u64, result: Result<ReviewOutcome, String>| {
            if take(&open, id) {
                sender.unbounded_send((id, result)).ok();
            }
        });
        if let Err(why) = host.propose(&self.extension_id, id, proposal, sink) {
            self.open().remove(&id);
            return Err(error(HostErrorCode::Unavailable, why));
        }
        Ok(id)
    }

    /// `review.withdraw`: the proposal's current text; its outcome never arrives.
    pub(crate) fn withdraw(&self, id: u64) -> Result<String, HostError> {
        let host = self.host()?;
        if !self.open().remove(&id) {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{id} is not an undecided proposal of this extension"),
            ));
        }
        host.withdraw(&self.extension_id, id)
            .map_err(|why| error(HostErrorCode::Unavailable, why))
    }

    /// `review.begin-turn`.
    pub(crate) fn begin_turn(&self, session: &str) -> Result<(), HostError> {
        let host = self.host()?;
        check_label("session", session)?;
        host.begin_turn(&self.extension_id, session);
        Ok(())
    }

    /// `review.end-turn`: the files the tool changed in the turn, worktree files only.
    pub(crate) fn end_turn(&self, session: &str, changed: Vec<FileRef>) -> Result<(), HostError> {
        let host = self.host()?;
        check_label("session", session)?;
        if changed.len() > MAX_TURN_PATHS {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!("a turn names at most {MAX_TURN_PATHS} files"),
            ));
        }
        let roots = roots(self.workspace.as_deref())?;
        for file in &changed {
            file.check()?;
            absolute(&roots, file)?;
        }
        host.end_turn(&self.extension_id, session, changed);
        Ok(())
    }

    /// The instance is going or `review:propose` was revoked: its open reviews close, and no
    /// outcome of theirs is delivered.
    pub(crate) fn close_all(&self) {
        let ids: Vec<u64> = std::mem::take(&mut *self.open()).into_iter().collect();
        if let Some(host) = &self.host
            && !ids.is_empty()
        {
            host.close(&self.extension_id, &ids);
        }
    }

    /// A proposal's file: checked, in a worktree, neither protected nor private by default.
    fn file(&self, path: &FileRef) -> Result<PathBuf, HostError> {
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
        if is_default_private(&path.path) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        // A new file needs its worktree named; an existing one is found in the only worktree
        // that has it.
        absolute(&roots(self.workspace.as_deref())?, path)
    }
}

impl Drop for ReviewHandle {
    fn drop(&mut self) {
        self.close_all();
    }
}

/// Removes `id` from the open set if the handle still lives; whether it was there.
fn take(open: &Weak<Mutex<BTreeSet<u64>>>, id: u64) -> bool {
    open.upgrade().is_some_and(|open| {
        open.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id)
    })
}

fn check_label(what: &str, text: &str) -> Result<(), HostError> {
    if text.len() > MAX_PROPOSAL_LABEL_BYTES || text.chars().any(char::is_control) {
        return Err(error(
            HostErrorCode::InvalidArgument,
            format!(
                "{what} is at most {MAX_PROPOSAL_LABEL_BYTES} bytes, without control characters"
            ),
        ));
    }
    Ok(())
}
