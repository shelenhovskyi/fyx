//! Fyx demo: bookmarks as a tree view, with badges in the project panel (staged `fyx:ui` 0.4,
//! plan 6.7, `docs/fyx/wit-0.4/design.md` §F).
//!
//! Panel `"bookmarks"` (left dock, "Bookmarks"): a button that bookmarks the active file, or
//! removes its bookmark (`context:editor` says which file is active), and a two-level tree of the
//! bookmarks grouped by folder. A folder row has a chevron: a click on it, or Right and Left on
//! the focused list, sends `row-toggled`, and the demo answers with the new row count and the
//! rows from the folder on ([`tree_row`] sets each row's indent and chevron). A click on a file
//! row opens the file (`workspace:open`). Every bookmarked file gets a `★` badge in the project
//! panel (`workspace:file-decorations`); removing the bookmark removes it, and stopping the
//! extension removes them all.
//!
//! The tree logic ([`Bookmarks`]) needs no host, so it builds in every world; the panel needs the
//! 0.4 imports.

use std::collections::BTreeSet;

use ui_ext_sdk::*;

#[cfg(feature = "fyx-ui-0-4")]
mod panel;
#[cfg(feature = "fyx-ui-0-4")]
pub use panel::BookmarksDemo;

/// A bookmarked file: its worktree and its path in it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bookmark {
    pub worktree: u64,
    pub path: String,
}

impl Bookmark {
    #[cfg(feature = "fyx-ui-0-4")]
    pub fn file_ref(&self) -> FileRef {
        FileRef {
            worktree: Some(self.worktree),
            path: self.path.clone(),
        }
    }

    /// The folder it is grouped under: its parent path, empty for the worktree's root.
    pub fn folder(&self) -> Folder {
        Folder {
            worktree: self.worktree,
            path: self
                .path
                .rsplit_once('/')
                .map_or(String::new(), |(parent, _)| parent.to_string()),
        }
    }

    pub fn name(&self) -> &str {
        self.path
            .rsplit_once('/')
            .map_or(self.path.as_str(), |(_, name)| name)
    }
}

/// A folder of the tree.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Folder {
    pub worktree: u64,
    pub path: String,
}

impl Folder {
    pub fn label(&self) -> String {
        if self.path.is_empty() {
            "(root)".to_string()
        } else {
            format!("{}/", self.path)
        }
    }
}

/// One row of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Folder {
        folder: Folder,
        files: usize,
        expanded: bool,
    },
    File(Bookmark),
}

/// The bookmarks and which folders are expanded.
#[derive(Debug, Default)]
pub struct Bookmarks {
    files: BTreeSet<Bookmark>,
    expanded: BTreeSet<Folder>,
}

impl Bookmarks {
    pub fn contains(&self, bookmark: &Bookmark) -> bool {
        self.files.contains(bookmark)
    }

    /// Adds a bookmark; its folder opens so the new row shows.
    pub fn add(&mut self, bookmark: Bookmark) {
        self.expanded.insert(bookmark.folder());
        self.files.insert(bookmark);
    }

    pub fn remove(&mut self, bookmark: &Bookmark) {
        self.files.remove(bookmark);
        let folder = bookmark.folder();
        if !self.files.iter().any(|file| file.folder() == folder) {
            self.expanded.remove(&folder);
        }
    }

    /// Expands or collapses the folder of row `index`. `false` when the row is not a folder or
    /// already is that way.
    pub fn toggle(&mut self, index: u32, expand: bool) -> bool {
        let Some(Row::Folder {
            folder, expanded, ..
        }) = self.row(index)
        else {
            return false;
        };
        if expanded == expand {
            return false;
        }
        if expand {
            self.expanded.insert(folder);
        } else {
            self.expanded.remove(&folder);
        }
        true
    }

    /// Every visible row: each folder, then its files when it is expanded.
    pub fn rows(&self) -> Vec<Row> {
        let folders: BTreeSet<Folder> = self.files.iter().map(Bookmark::folder).collect();
        let mut rows = Vec::new();
        for folder in folders {
            let files: Vec<&Bookmark> = self
                .files
                .iter()
                .filter(|file| file.folder() == folder)
                .collect();
            let expanded = self.expanded.contains(&folder);
            rows.push(Row::Folder {
                folder,
                files: files.len(),
                expanded,
            });
            if expanded {
                rows.extend(files.into_iter().cloned().map(Row::File));
            }
        }
        rows
    }

    pub fn row(&self, index: u32) -> Option<Row> {
        let index = usize::try_from(index).ok()?;
        self.rows().into_iter().nth(index)
    }

    pub fn row_count(&self) -> u32 {
        u32::try_from(self.rows().len()).unwrap_or(u32::MAX)
    }
}

/// Exported as 0.3 or older the demo has no `context`, so it shows nothing.
#[cfg(not(feature = "fyx-ui-0-4"))]
pub struct BookmarksDemo;

#[cfg(not(feature = "fyx-ui-0-4"))]
impl Extension for BookmarksDemo {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        Vec::new()
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

// The panel needs the 0.4 imports (`script/fyx-build-demos --fyx-ui-0-4`); the older worlds
// build the empty extension above.
#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(BookmarksDemo, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(BookmarksDemo, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(BookmarksDemo);

#[cfg(test)]
mod tests {
    use super::*;

    fn bookmark(path: &str) -> Bookmark {
        Bookmark {
            worktree: 1,
            path: path.into(),
        }
    }

    #[test]
    fn bookmarks_form_a_two_level_tree() {
        let mut bookmarks = Bookmarks::default();
        bookmarks.add(bookmark("src/b.rs"));
        bookmarks.add(bookmark("README.md"));
        bookmarks.add(bookmark("src/a.rs"));
        let shape = |bookmarks: &Bookmarks| {
            bookmarks
                .rows()
                .into_iter()
                .map(|row| match row {
                    Row::Folder {
                        folder,
                        files,
                        expanded,
                    } => format!("{} {files} {expanded}", folder.label()),
                    Row::File(file) => format!("  {}", file.name()),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            shape(&bookmarks),
            ["(root) 1 true", "  README.md", "src/ 2 true", "  a.rs", "  b.rs"]
        );

        assert!(bookmarks.toggle(2, false));
        assert_eq!(
            shape(&bookmarks),
            ["(root) 1 true", "  README.md", "src/ 2 false"]
        );
        assert!(!bookmarks.toggle(2, false), "already collapsed");
        assert!(!bookmarks.toggle(1, true), "a file has no chevron");
        assert!(bookmarks.toggle(2, true));
        assert_eq!(bookmarks.row_count(), 5);

        bookmarks.remove(&bookmark("README.md"));
        assert_eq!(shape(&bookmarks), ["src/ 2 true", "  a.rs", "  b.rs"]);
    }
}
