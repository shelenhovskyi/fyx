//! Host side of `workspace:edit` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §10, plan 4.8).
//!
//! The import checks the edit ([`check_edit`]) and the in-flight cap and returns an id; the edit
//! starts after the call returns (a trap drops it). Then, outside any budget: the overlap and
//! protected-path checks ([`check_targets`]), the broker (`ask` prompts), and the host's
//! [`WorkspaceEdits`], which resolves the targets and applies every file in one undoable step.
//! The outcome comes back through `handle-io` as `edit-done`.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
};

use futures::future::{AbortHandle, Abortable, BoxFuture};
pub use ui_ext::edit::{
    CAPABILITY_EDIT, EditCounters, EditError, EditTarget, FileEdit, MAX_EDITS_IN_FLIGHT,
    MAX_FILE_BYTES, MAX_FILES, MAX_LABEL_BYTES, MAX_NEW_TEXT_BYTES, MAX_TEXT_EDITS,
    PROTECTED_DIRECTORIES, Position, TextEdit, TextRange, WorkspaceEdit, check_edit, check_targets,
    is_protected_path,
};
use ui_ext::v0_2::{HostError, HostErrorCode};

use crate::{
    network::{IoEvent, Running},
    permissions::{CapabilityCheck, consent},
};

/// The editor side of `workspace:edit`, implemented by the host (`ui_ext_host`) over the open
/// project. `edit` has passed [`check_edit`] and [`check_targets`] and the broker.
///
/// The implementation resolves every target (a `file` target against the worktrees, a `document`
/// target against the extension's document ids) and refuses the whole edit if any file fails:
/// a restricted worktree is `denied`; a file outside the worktrees, a symlink out of them, a
/// `private_files` match, a config file `fyx_guard` refuses, a collaboration guest or a file over
/// [`MAX_FILE_BYTES`] is `not-allowed`; a missing file without `create` is `not-found`; a newer
/// version than `expected-version` is `conflict`; a range past the end is `invalid`. Then it
/// applies all files in one synchronous step, as one transaction per buffer grouped under the
/// edit's label, so one undo reverts it, and does not save.
///
/// Dropping the future before it resolves (reload, revocation) must leave nothing applied, which
/// holds when the resolution is the only part that waits.
pub trait WorkspaceEdits: Send + Sync + 'static {
    fn apply(
        &self,
        extension_id: &str,
        edit: WorkspaceEdit,
    ) -> BoxFuture<'static, Result<(), EditError>>;
}

/// The edits of one instance that started and have not delivered, so a revocation or a reload
/// can stop them.
#[derive(Default)]
pub(crate) struct EditSet {
    handles: Mutex<HashMap<u64, AbortHandle>>,
    /// Aborted because the capability was revoked: they deliver `denied`, not `cancelled`.
    revoked: Mutex<HashSet<u64>>,
}

impl EditSet {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, AbortHandle>> {
        self.handles.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `workspace:edit` was revoked: every edit not yet applied ends with `denied`.
    pub(crate) fn revoke_all(&self) {
        let handles: Vec<_> = self.lock().drain().collect();
        let mut revoked = self.revoked.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, handle) in handles {
            revoked.insert(id);
            handle.abort();
        }
    }

    fn abort_all(&self) {
        for (_, handle) in self.lock().drain() {
            handle.abort();
        }
    }

    fn aborted(&self, id: u64) -> EditError {
        if self
            .revoked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id)
        {
            EditError::Denied
        } else {
            EditError::Cancelled
        }
    }
}

/// An edit the import accepted, started once the call returns successfully.
pub(crate) struct PendingEdit {
    id: u64,
    edit: WorkspaceEdit,
}

/// The `workspace-edit` state of one instance, in its store. Edits count as in flight from the
/// import until their `edit-done` is delivered.
pub(crate) struct EditState {
    /// Whether the host gave a [`WorkspaceEdits`]; without one, `apply-edit` is `unavailable`.
    available: bool,
    counters: EditCounters,
    pending: Vec<PendingEdit>,
}

impl EditState {
    pub(crate) fn new(available: bool) -> Self {
        Self {
            available,
            counters: EditCounters::default(),
            pending: Vec::new(),
        }
    }

    /// `workspace-edit.apply-edit`, after the caller checked the manifest and the broker.
    pub(crate) fn apply(&mut self, edit: WorkspaceEdit) -> Result<u64, HostError> {
        if !self.available {
            return Err(HostError {
                code: HostErrorCode::Unavailable,
                message: "no workspace is open".into(),
            });
        }
        check_edit(&edit)?;
        let id = self.counters.start()?;
        self.pending.push(PendingEdit { id, edit });
        Ok(id)
    }

    /// After a call: the edits to start if it succeeded; a failed call drops them.
    pub(crate) fn finish_call(&mut self, succeeded: bool) -> Vec<PendingEdit> {
        let pending = std::mem::take(&mut self.pending);
        if succeeded {
            return pending;
        }
        for edit in &pending {
            self.counters.finished(edit.id);
        }
        Vec::new()
    }

    /// Before `handle-io` delivers `edit-done`.
    pub(crate) fn delivered(&mut self, id: u64) {
        self.counters.finished(id);
    }
}

/// Starts accepted edits, once per instance. Dropping it stops every edit not yet applied, and
/// none delivers (the events channel is closed with the instance).
pub(crate) struct EditHandle {
    extension_id: Arc<str>,
    broker: Option<Arc<dyn CapabilityCheck>>,
    edits: Option<Arc<dyn WorkspaceEdits>>,
    set: Arc<EditSet>,
    running: Arc<Running>,
}

impl EditHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        broker: Option<Arc<dyn CapabilityCheck>>,
        edits: Option<Arc<dyn WorkspaceEdits>>,
        set: Arc<EditSet>,
        running: Arc<Running>,
    ) -> Self {
        Self {
            extension_id,
            broker,
            edits,
            set,
            running,
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.edits.is_some()
    }

    /// Starts `pending` on the current tokio runtime, in call order.
    pub(crate) fn start(&self, pending: Vec<PendingEdit>) {
        for PendingEdit { id, edit } in pending {
            self.running.diagnostics.io_started(
                CAPABILITY_EDIT,
                id,
                format!("{:?} ({} files)", edit.label, edit.files.len()),
            );
            let (handle, registration) = AbortHandle::new_pair();
            self.set.lock().insert(id, handle);
            let set = self.set.clone();
            let running = self.running.clone();
            let broker = self.broker.clone();
            let edits = self.edits.clone();
            let extension_id = self.extension_id.clone();
            tokio::spawn(async move {
                let work = async move {
                    check_targets(&edit)?;
                    if !consent(broker.as_ref(), &extension_id, CAPABILITY_EDIT).await {
                        return Err(EditError::Denied);
                    }
                    match edits {
                        Some(edits) => edits.apply(&extension_id, edit).await,
                        None => Err(EditError::Cancelled),
                    }
                };
                let result = match Abortable::new(work, registration).await {
                    Ok(result) => result,
                    Err(_) => Err(set.aborted(id)),
                };
                set.lock().remove(&id);
                running.deliver(IoEvent::EditDone(id, result)).await;
            });
        }
    }
}

impl Drop for EditHandle {
    fn drop(&mut self) {
        self.set.abort_all();
    }
}
