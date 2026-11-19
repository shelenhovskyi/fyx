//! The fyx:ui 0.4 `context` import over [`ContextHost`] (design wit-0.4 §B.11, plan 5.13):
//! `context:editor` (the feed and the path conversions) and `workspace:open`. This side checks
//! the capabilities, the paths and the focus rule, and keeps private files' text out of the
//! feed whatever the host sends; the host debounces and knows the editor.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use futures::{StreamExt as _, channel::mpsc};
use ui_ext::{
    context::{self, ContextHost, EditorContext},
    edit::TextRange,
    process::FileRef,
    v0_2::{HostError, HostErrorCode},
};

use crate::{
    listeners::ListenersHandle,
    network::{IoEvent, Running},
    process::Workspace,
};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

/// Which instance of an extension watches the context, by host and extension id: a reloaded
/// instance's `watch` replaces the old one's sink, and the old one, dropped later, leaves it.
static WATCHING: LazyLock<Mutex<HashMap<(usize, Arc<str>), u64>>> = LazyLock::new(Default::default);
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// One instance's tie to the editor host.
pub(crate) struct ContextHandle {
    extension_id: Arc<str>,
    host: Option<Arc<dyn ContextHost>>,
    workspace: Option<Arc<dyn Workspace>>,
    listeners: Arc<ListenersHandle>,
    sender: mpsc::UnboundedSender<EditorContext>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<EditorContext>>>,
    instance: u64,
    watching: AtomicBool,
}

impl ContextHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        host: Option<Arc<dyn ContextHost>>,
        workspace: Option<Arc<dyn Workspace>>,
        listeners: Arc<ListenersHandle>,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded();
        Self {
            extension_id,
            host,
            workspace,
            listeners,
            sender,
            receiver: Mutex::new(Some(receiver)),
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            watching: AtomicBool::new(false),
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.host.is_some()
    }

    fn host(&self) -> Result<&Arc<dyn ContextHost>, HostError> {
        self.host.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "the editor context is not available in this window",
            )
        })
    }

    fn key(&self, host: &Arc<dyn ContextHost>) -> (usize, Arc<str>) {
        (
            Arc::as_ptr(host) as *const () as usize,
            self.extension_id.clone(),
        )
    }

    /// Moves the host's contexts onto the io channel, on the current tokio runtime. Once.
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
            while let Some(context) = receiver.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::Context(context)).await;
            }
        });
    }

    /// `context.watch`.
    pub(crate) fn watch(&self, on: bool) -> Result<(), HostError> {
        let host = self.host()?;
        let key = self.key(host);
        if on {
            let sender = self.sender.clone();
            let filter = Arc::new(PrivacyFilter {
                host: host.clone(),
                workspace: self.workspace.clone(),
            });
            host.watch(
                &self.extension_id,
                Some(Arc::new(move |context| {
                    sender.unbounded_send(filter.apply(context)).ok();
                })),
            );
            WATCHING
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, self.instance);
            self.watching.store(true, Ordering::SeqCst);
        } else {
            self.stop();
        }
        Ok(())
    }

    /// Stops this instance's feed, if it is the one watching.
    pub(crate) fn stop(&self) {
        let Some(host) = &self.host else {
            return;
        };
        if !self.watching.swap(false, Ordering::SeqCst) {
            return;
        }
        let key = self.key(host);
        let mut watching = WATCHING.lock().unwrap_or_else(PoisonError::into_inner);
        if watching.get(&key) == Some(&self.instance) {
            watching.remove(&key);
            drop(watching);
            host.watch(&self.extension_id, None);
        }
    }

    /// `workspace.open`: a file of a worktree. `focus` is kept only with a gesture, or with a
    /// listener request less than 2 s old while one of the extension's terminals has focus;
    /// otherwise the file opens in the background.
    pub(crate) fn open(
        &self,
        path: FileRef,
        range: Option<TextRange>,
        focus: bool,
        gesture: bool,
    ) -> Result<(), HostError> {
        let host = self.host()?;
        path.check()?;
        let absolute = self.absolute(&path)?;
        if !absolute.is_file() {
            return Err(error(
                HostErrorCode::InvalidArgument,
                format!("{:?} is not a file", path.path),
            ));
        }
        let focus = focus
            && (gesture
                || (self
                    .listeners
                    .requested_within(context::FOCUS_REQUEST_WINDOW)
                    && host.terminal_focused(&self.extension_id)));
        host.open(&self.extension_id, path, range, focus)
            .map_err(|why| error(HostErrorCode::Unavailable, why))
    }

    /// `context.to-file-ref`: a tool's absolute path or `file://` URI in one of the worktrees.
    pub(crate) fn to_file_ref(&self, absolute: &str) -> Result<FileRef, HostError> {
        let host = self.host()?;
        let parsed = context::parse_tool_path(absolute).ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                "not an absolute path or file URI",
            )
        })?;
        let roots = self.roots()?;
        let file_ref = context::file_ref_in(
            &parsed,
            roots.iter().map(|(id, root)| (*id, root.as_path())),
        )
        .ok_or_else(|| {
            error(
                HostErrorCode::InvalidArgument,
                "the path is outside every open worktree",
            )
        })?;
        let full = self.absolute(&file_ref)?;
        if host.is_private(&full) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        Ok(file_ref)
    }

    /// `context.absolute-path`: a worktree file's absolute path, for a tool's reply.
    pub(crate) fn absolute_path(&self, path: &FileRef) -> Result<String, HostError> {
        let host = self.host()?;
        path.check()?;
        let absolute = self.absolute(path)?;
        if host.is_private(&absolute) {
            return Err(error(HostErrorCode::NotPermitted, "a private file"));
        }
        Ok(absolute.to_string_lossy().into_owned())
    }

    fn roots(&self) -> Result<Vec<(u64, PathBuf)>, HostError> {
        roots(self.workspace.as_deref())
    }

    fn absolute(&self, path: &FileRef) -> Result<PathBuf, HostError> {
        absolute(&self.roots()?, path)
    }
}

impl Drop for ContextHandle {
    /// The instance is gone: its feed stops, unless a newer instance took it over.
    fn drop(&mut self) {
        self.stop();
    }
}

fn roots(workspace: Option<&dyn Workspace>) -> Result<Vec<(u64, PathBuf)>, HostError> {
    let roots: Vec<(u64, PathBuf)> = workspace
        .map(|workspace| {
            workspace
                .worktrees()
                .into_iter()
                .map(|worktree| (worktree.id, worktree.root))
                .collect()
        })
        .unwrap_or_default();
    if roots.is_empty() {
        return Err(error(HostErrorCode::NoWorktree, "no worktree is open"));
    }
    Ok(roots)
}

/// A checked `file-ref`'s absolute path: in its worktree, or in the one worktree that has it.
fn absolute(roots: &[(u64, PathBuf)], path: &FileRef) -> Result<PathBuf, HostError> {
    let join = |root: &PathBuf| {
        path.path
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(root.clone(), |full, part| full.join(part))
    };
    match path.worktree {
        Some(id) => roots
            .iter()
            .find(|(root_id, _)| *root_id == id)
            .map(|(_, root)| join(root))
            .ok_or_else(|| error(HostErrorCode::InvalidArgument, "an unknown worktree")),
        None => {
            let mut found = roots
                .iter()
                .map(|(_, root)| join(root))
                .filter(|full| full.exists());
            match (found.next(), found.next()) {
                (Some(full), None) => Ok(full),
                (None, _) => Err(error(
                    HostErrorCode::InvalidArgument,
                    format!("no worktree has {:?}", path.path),
                )),
                (Some(_), Some(_)) => Err(error(
                    HostErrorCode::InvalidArgument,
                    format!("more than one worktree has {:?}; name one", path.path),
                )),
            }
        }
    }
}

/// What every context passes before the extension sees it: worktree files only, no text of a
/// private active file, and the text capped.
struct PrivacyFilter {
    host: Arc<dyn ContextHost>,
    workspace: Option<Arc<dyn Workspace>>,
}

impl PrivacyFilter {
    fn apply(&self, mut context: EditorContext) -> EditorContext {
        let roots = roots(self.workspace.as_deref()).unwrap_or_default();
        let in_worktree = |file: &FileRef| file.check().is_ok() && absolute(&roots, file).is_ok();
        context.open.retain(|file| in_worktree(file));
        if context
            .active
            .as_ref()
            .is_some_and(|file| !in_worktree(file))
        {
            context.active = None;
            context.selection = None;
            context.trusted = None;
        }
        let private = context.active.as_ref().is_none_or(|file| {
            absolute(&roots, file).map_or(true, |full| self.host.is_private(&full))
        });
        if let Some((_, text)) = &mut context.selection {
            if private {
                *text = None;
            } else if let Some(text) = text {
                context::cap_selection(text);
            }
        }
        context
    }
}
