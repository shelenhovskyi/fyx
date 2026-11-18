//! The open worktrees, as `process:spawn` (W2c, plan 4.7) sees them: `ui_ext_wasm::process::
//! Workspace`. Every workspace window reports its project's visible worktrees and whether each is
//! trusted (decision 26: `process:spawn` runs only in trusted ones); an extension gets ids that are
//! opaque to it (design.md 0.3 §0.5: stable for the session, different per extension, so two
//! extensions cannot correlate what they see).

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use gpui::{App, EntityId, Global};
use sha2::{Digest as _, Sha256};
use ui_ext_wasm::process::{Workspace as ProcessWorkspace, WorktreeRoot};

/// One worktree as a workspace reported it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWorktree {
    /// The workspace and the worktree id within its project.
    pub key: (u64, u64),
    pub root: PathBuf,
    pub trusted: bool,
}

/// The worktrees of every open workspace, kept current on the main thread and read from extension
/// threads.
#[derive(Clone, Default)]
pub struct OpenWorktrees(Arc<RwLock<HashMap<EntityId, Vec<OpenWorktree>>>>);

impl Global for OpenWorktrees {}

impl OpenWorktrees {
    pub fn global(cx: &mut App) -> Self {
        cx.default_global::<Self>().clone()
    }

    /// What one workspace has open now.
    pub fn set(&self, workspace: EntityId, worktrees: Vec<OpenWorktree>) {
        let mut all = self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if worktrees.is_empty() {
            all.remove(&workspace);
        } else {
            all.insert(workspace, worktrees);
        }
    }

    pub fn remove(&self, workspace: EntityId) {
        self.set(workspace, Vec::new());
    }

    /// The provider for one extension.
    pub fn for_extension(&self, extension_id: &str) -> Arc<dyn ProcessWorkspace> {
        Arc::new(ExtensionWorktrees {
            extension_id: extension_id.into(),
            open: self.clone(),
        })
    }
}

struct ExtensionWorktrees {
    extension_id: Arc<str>,
    open: OpenWorktrees,
}

/// An id opaque to the extension: a keyed hash of the worktree's key.
fn opaque_id(extension_id: &str, key: (u64, u64)) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"fyx-worktree\0");
    hasher.update(extension_id.as_bytes());
    hasher.update([0]);
    hasher.update(key.0.to_le_bytes());
    hasher.update(key.1.to_le_bytes());
    let digest = hasher.finalize();
    u64::from_le_bytes(digest[..8].try_into().expect("8 bytes"))
}

impl ProcessWorkspace for ExtensionWorktrees {
    fn worktrees(&self) -> Vec<WorktreeRoot> {
        let all = self
            .open
            .0
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut roots: Vec<WorktreeRoot> = all
            .values()
            .flatten()
            .map(|worktree| WorktreeRoot {
                id: opaque_id(&self.extension_id, worktree.key),
                root: worktree.root.clone(),
                trusted: worktree.trusted,
            })
            .collect();
        roots.sort_by(|a, b| a.root.cmp(&b.root).then(a.id.cmp(&b.id)));
        roots
    }
}

/// What a workspace's project has open: its visible worktrees, each trusted when the trust store
/// says so (`can_trust`, which shows Zed's restricted-mode notice the first time it refuses).
/// Without a trust store (tests, a headless app) none is trusted.
pub(crate) fn of_project(
    workspace_key: u64,
    project: &gpui::Entity<project::Project>,
    cx: &mut App,
) -> Vec<OpenWorktree> {
    let (store, worktrees): (_, Vec<(project::WorktreeId, PathBuf)>) = {
        let project = project.read(cx);
        (
            project.worktree_store(),
            project
                .visible_worktrees(cx)
                .map(|worktree| {
                    let worktree = worktree.read(cx);
                    (worktree.id(), worktree.abs_path().to_path_buf())
                })
                .collect(),
        )
    };
    let trust = project::trusted_worktrees::TrustedWorktrees::try_get_global(cx);
    worktrees
        .into_iter()
        .map(|(id, root)| {
            let trusted = trust
                .as_ref()
                .is_some_and(|trust| trust.update(cx, |trust, cx| trust.can_trust(&store, id, cx)));
            OpenWorktree {
                key: (workspace_key, id.to_proto()),
                root,
                trusted,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project's visible worktrees, untrusted without a trust store.
    #[gpui::test]
    async fn test_of_project(cx: &mut gpui::TestAppContext) {
        crate::tests::init_test(cx);
        let fs = fs::FakeFs::new(cx.executor());
        fs.insert_tree(util::path!("/work"), serde_json::json!({ "a.txt": "" }))
            .await;
        let project =
            project::Project::test(fs, [std::path::Path::new(util::path!("/work"))], cx).await;
        let worktrees = cx.update(|cx| of_project(1, &project, cx));
        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].root, PathBuf::from(util::path!("/work")));
        assert!(!worktrees[0].trusted, "no trust store: not trusted");
    }

    #[test]
    fn test_ids_are_opaque_per_extension_and_stable() {
        let open = OpenWorktrees::default();
        open.set(
            EntityId::from(1u64),
            vec![
                OpenWorktree {
                    key: (1, 7),
                    root: PathBuf::from("/work/a"),
                    trusted: true,
                },
                OpenWorktree {
                    key: (1, 8),
                    root: PathBuf::from("/work/b"),
                    trusted: false,
                },
            ],
        );
        let first = open.for_extension("one").worktrees();
        let again = open.for_extension("one").worktrees();
        let other = open.for_extension("two").worktrees();
        assert_eq!(first, again, "stable");
        assert_eq!(first.len(), 2);
        assert_ne!(first[0].id, other[0].id, "per extension");
        assert_eq!(first[0].root, PathBuf::from("/work/a"));
        assert!(first[0].trusted && !first[1].trusted);
        open.remove(EntityId::from(1u64));
        assert!(open.for_extension("one").worktrees().is_empty());
    }
}
