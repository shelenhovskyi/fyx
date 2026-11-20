//! Fyx (design wit-0.4 §F.3, plan 6.7): what extensions mark project-panel entries with. A
//! global entity `ui_ext_host` writes, after the runtime's checks, and `project_panel` reads.
//!
//! Each extension has its own map from [`ProjectPath`] to a [`FileDecoration`]. Several
//! extensions on one entry: [`FileDecorations::for_path`] shows their badges side by side, at most
//! [`MAX_BADGES_PER_ENTRY`], in extension id order, and the colour of the first one that has one.
//! A decoration is not inherited by a folder's children and does not propagate to its parents.

use std::collections::BTreeMap;

use collections::HashMap;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, SharedString};

use crate::ProjectPath;

/// Badges one entry shows at most, across extensions.
pub const MAX_BADGES_PER_ENTRY: usize = 2;

/// The tokens an extension may tint an entry's name with. Git status and diagnostics colours
/// win over them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDecorationColor {
    Accent,
    Warning,
    Error,
    Success,
    Muted,
}

/// One extension's mark on one entry, already checked (badge of at most 2 characters, tooltip of
/// at most 256 bytes).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileDecoration {
    pub badge: Option<SharedString>,
    pub color: Option<FileDecorationColor>,
    pub tooltip: Option<SharedString>,
}

/// What the project panel draws for one entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryDecorations {
    /// In extension id order, at most [`MAX_BADGES_PER_ENTRY`].
    pub badges: Vec<SharedString>,
    pub color: Option<FileDecorationColor>,
    /// `(extension name, tooltip)` in extension id order.
    pub tooltips: Vec<(SharedString, SharedString)>,
}

impl EntryDecorations {
    pub fn is_empty(&self) -> bool {
        self.badges.is_empty() && self.color.is_none() && self.tooltips.is_empty()
    }
}

/// Some extension's decorations changed; the panel redraws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDecorationsChanged;

#[derive(Debug, Default)]
struct ExtensionDecorations {
    /// Shown in tooltips.
    name: SharedString,
    paths: HashMap<ProjectPath, FileDecoration>,
}

/// Every running extension's decorations, by extension id.
#[derive(Debug, Default)]
pub struct FileDecorations {
    extensions: BTreeMap<String, ExtensionDecorations>,
}

struct GlobalFileDecorations(Entity<FileDecorations>);

impl Global for GlobalFileDecorations {}

impl EventEmitter<FileDecorationsChanged> for FileDecorations {}

impl FileDecorations {
    /// The app's decorations, made on first use.
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(global) = cx.try_global::<GlobalFileDecorations>() {
            return global.0.clone();
        }
        let decorations = cx.new(|_| Self::default());
        cx.set_global(GlobalFileDecorations(decorations.clone()));
        decorations
    }

    /// The app's decorations if anything made them; readers that only draw use this.
    pub fn try_global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalFileDecorations>()
            .map(|global| global.0.clone())
    }

    /// Sets or replaces `extension_id`'s decoration of each path. `name` is the extension's
    /// display name, for tooltips.
    pub fn decorate(
        &mut self,
        extension_id: &str,
        name: SharedString,
        decorations: impl IntoIterator<Item = (ProjectPath, FileDecoration)>,
        cx: &mut Context<Self>,
    ) {
        let extension = self.extensions.entry(extension_id.to_string()).or_default();
        extension.name = name;
        extension.paths.extend(decorations);
        cx.emit(FileDecorationsChanged);
        cx.notify();
    }

    /// Removes `extension_id`'s decorations of `paths`, or all of them with `None` (the
    /// extension stopped, crashed, reloaded or lost the capability).
    pub fn clear(
        &mut self,
        extension_id: &str,
        paths: Option<&[ProjectPath]>,
        cx: &mut Context<Self>,
    ) {
        let changed = match paths {
            None => self.extensions.remove(extension_id).is_some(),
            Some(paths) => match self.extensions.get_mut(extension_id) {
                Some(extension) => {
                    let before = extension.paths.len();
                    for path in paths {
                        extension.paths.remove(path);
                    }
                    let changed = extension.paths.len() != before;
                    if extension.paths.is_empty() {
                        self.extensions.remove(extension_id);
                    }
                    changed
                }
                None => false,
            },
        };
        if changed {
            cx.emit(FileDecorationsChanged);
            cx.notify();
        }
    }

    /// What the panel draws for `path`; empty when no extension decorates it.
    pub fn for_path(&self, path: &ProjectPath) -> EntryDecorations {
        let mut entry = EntryDecorations::default();
        for extension in self.extensions.values() {
            let Some(decoration) = extension.paths.get(path) else {
                continue;
            };
            if let Some(badge) = &decoration.badge
                && entry.badges.len() < MAX_BADGES_PER_ENTRY
            {
                entry.badges.push(badge.clone());
            }
            if entry.color.is_none() {
                entry.color = decoration.color;
            }
            if let Some(tooltip) = &decoration.tooltip {
                entry
                    .tooltips
                    .push((extension.name.clone(), tooltip.clone()));
            }
        }
        entry
    }

    /// Whether any extension decorates anything: the panel skips the lookups otherwise.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use util::rel_path::rel_path;
    use worktree::WorktreeId;

    fn path(name: &str) -> ProjectPath {
        ProjectPath {
            worktree_id: WorktreeId::from_usize(1),
            path: rel_path(name).into(),
        }
    }

    fn badge(text: &'static str, color: Option<FileDecorationColor>) -> FileDecoration {
        FileDecoration {
            badge: Some(text.into()),
            color,
            tooltip: Some(format!("{text} tip").into()),
        }
    }

    /// §F.2, §F.5: two extensions on one file show both badges in id order, the first colour
    /// wins, a third badge is left out, and clearing one extension leaves the others.
    #[gpui::test]
    fn several_extensions_on_one_entry(cx: &mut TestAppContext) {
        let decorations = cx.update(FileDecorations::global);
        let events = std::rc::Rc::new(std::cell::Cell::new(0));
        let _subscription = cx.update(|cx| {
            let events = events.clone();
            cx.subscribe(&decorations, move |_, _: &FileDecorationsChanged, _| {
                events.set(events.get() + 1);
            })
        });
        decorations.update(cx, |decorations, cx| {
            decorations.decorate(
                "b-bookmarks",
                "Bookmarks".into(),
                [(path("src/a.rs"), badge("★", None))],
                cx,
            );
            decorations.decorate(
                "a-lint",
                "Lint".into(),
                [(
                    path("src/a.rs"),
                    badge("!", Some(FileDecorationColor::Warning)),
                )],
                cx,
            );
            decorations.decorate(
                "c-third",
                "Third".into(),
                [(
                    path("src/a.rs"),
                    badge("3", Some(FileDecorationColor::Error)),
                )],
                cx,
            );
        });
        decorations.read_with(cx, |decorations, _| {
            let entry = decorations.for_path(&path("src/a.rs"));
            assert_eq!(entry.badges, ["!", "★"]);
            assert_eq!(entry.color, Some(FileDecorationColor::Warning));
            assert_eq!(entry.tooltips.len(), 3);
            assert!(decorations.for_path(&path("src")).is_empty());
        });
        decorations.update(cx, |decorations, cx| {
            decorations.clear("a-lint", None, cx);
            decorations.clear("b-bookmarks", Some(&[path("src/a.rs")]), cx);
            // Nothing to clear: no event.
            decorations.clear("missing", None, cx);
        });
        decorations.read_with(cx, |decorations, _| {
            let entry = decorations.for_path(&path("src/a.rs"));
            assert_eq!(entry.badges, ["3"]);
            assert_eq!(entry.color, Some(FileDecorationColor::Error));
        });
        assert_eq!(events.get(), 5);
    }
}
