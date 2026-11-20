//! The panel, the badges and opening files, on the 0.4 `context` imports.

use ui_ext_sdk::context::{self, EditorContext, FileDecoration};
use ui_ext_sdk::*;

use crate::{Bookmark, Bookmarks, Row};

const PANEL: &str = "bookmarks";
const ROW_HEIGHT: u16 = 24;
const BADGE: &str = "\u{2605}";

struct Nodes {
    status: NodeId,
    toggle: NodeId,
    list: NodeId,
}

pub struct BookmarksDemo {
    bookmarks: Bookmarks,
    /// The file the editor shows, if it is in a worktree.
    active: Option<Bookmark>,
    status: String,
    tree: SurfaceTree,
    nodes: Option<Nodes>,
}

fn decoration(bookmark: &Bookmark) -> FileDecoration {
    FileDecoration {
        path: bookmark.file_ref(),
        badge: Some(BADGE.to_string()),
        color: Some(ColorToken::TextAccent),
        tooltip: Some("Bookmarked".to_string()),
    }
}

impl BookmarksDemo {
    fn toggle_button(&self) -> ButtonProps {
        match &self.active {
            Some(active) if self.bookmarks.contains(active) => {
                ButtonProps::new(format!("Remove {}", active.name())).icon(Icon::Close)
            }
            Some(active) => {
                ButtonProps::new(format!("Bookmark {}", active.name())).icon(Icon::Bookmark)
            }
            None => ButtonProps::new("Bookmark the active file")
                .icon(Icon::Bookmark)
                .disabled(true),
        }
    }

    fn status_props(&self) -> TextProps {
        TextProps::new(self.status.as_str()).color(ColorToken::TextMuted)
    }

    /// Brings the nodes in line; the list's rows from `first_changed` on are fetched again.
    fn sync(&mut self, first_changed: Option<u32>) -> Result<Update, String> {
        let (status, toggle) = (self.status_props(), self.toggle_button());
        let row_count = self.bookmarks.row_count();
        let Some(nodes) = &self.nodes else {
            return Ok(Update::Unchanged);
        };
        let (status_node, toggle_node, list_node) = (nodes.status, nodes.toggle, nodes.list);
        self.tree.set_props(status_node, status)?;
        self.tree.set_props(toggle_node, toggle)?;
        if let Some(first) = first_changed {
            self.tree
                .set_props(list_node, ListProps::new(row_count, ROW_HEIGHT))?;
            let rows = RowRange {
                start: first,
                count: row_count.saturating_sub(first).max(1),
            };
            self.tree.invalidate_rows(list_node, Some(rows))?;
        }
        Ok(self.tree.take_update())
    }

    fn toggle_active(&mut self) -> Result<Update, String> {
        let Some(active) = self.active.clone() else {
            return Ok(Update::Unchanged);
        };
        let result = if self.bookmarks.contains(&active) {
            context::clear_file_decorations(Some(&[active.file_ref()])).map(|()| {
                self.bookmarks.remove(&active);
                format!("Removed {}.", active.path)
            })
        } else {
            context::decorate_files(&[decoration(&active)]).map(|()| {
                self.bookmarks.add(active.clone());
                format!("Bookmarked {}.", active.path)
            })
        };
        self.status = match result {
            Ok(status) => status,
            Err(error) => format!("{}: {}", active.path, error.message),
        };
        self.sync(Some(0))
    }

    fn editor_changed(&mut self, editor: EditorContext) -> Result<(), String> {
        let active = editor.active.and_then(|file| {
            Some(Bookmark {
                worktree: file.worktree?,
                path: file.path,
            })
        });
        if active == self.active {
            return Ok(());
        }
        self.active = active;
        let update = self.sync(None)?;
        if update != Update::Unchanged
            && let Err(error) = push_update(PANEL, &update)
        {
            log(
                LogLevel::Warn,
                &format!("updating the panel failed: {}", error.message),
            );
        }
        Ok(())
    }
}

impl Extension for BookmarksDemo {
    fn new() -> Self {
        let status = match context::watch(true) {
            Ok(()) => {
                "Open a file and bookmark it; it gets a \u{2605} in the project panel.".to_string()
            }
            Err(error) => format!("The editor context is off: {}", error.message),
        };
        Self {
            bookmarks: Bookmarks::default(),
            active: None,
            status,
            tree: SurfaceTree::new(),
            nodes: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Bookmarks",
            DockPosition::Left,
            Icon::Bookmark,
            280,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let (status, toggle) = (self.status_props(), self.toggle_button());
        let list = ListProps::new(self.bookmarks.row_count(), ROW_HEIGHT);
        let mut nodes = None;
        let mut tree = SurfaceTree::new();
        let mounted = tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8).grow(), |tree| {
                let toggle = tree.button(toggle);
                let status = tree.text(status);
                let list = tree.list(list);
                nodes = Some(Nodes {
                    status,
                    toggle,
                    list,
                });
            });
        })?;
        self.tree = tree;
        self.nodes = nodes;
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        self.tree.apply_event(&event);
        let Some(nodes) = &self.nodes else {
            return Ok(Update::Unchanged);
        };
        match event {
            UiEvent::Click(id) if id == nodes.toggle => self.toggle_active(),
            UiEvent::RowToggled((id, index, expand)) if id == nodes.list => {
                if self.bookmarks.toggle(index, expand) {
                    self.sync(Some(index))
                } else {
                    Ok(Update::Unchanged)
                }
            }
            UiEvent::RowClick((id, index)) if id == nodes.list => match self.bookmarks.row(index) {
                Some(Row::File(bookmark)) => {
                    // A click is a gesture, so the editor may take focus.
                    if let Err(error) = context::open(&bookmark.file_ref(), None, true) {
                        self.status =
                            format!("Could not open {}: {}", bookmark.path, error.message);
                        return self.sync(None);
                    }
                    Ok(Update::Unchanged)
                }
                Some(Row::Folder { expanded, .. }) => {
                    if self.bookmarks.toggle(index, !expanded) {
                        self.sync(Some(index))
                    } else {
                        Ok(Update::Unchanged)
                    }
                }
                None => Ok(Update::Unchanged),
            },
            _ => Ok(Update::Unchanged),
        }
    }

    fn list_rows(
        &mut self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> Vec<ListRow> {
        if surface_id != PANEL
            || self
                .nodes
                .as_ref()
                .is_none_or(|nodes| nodes.list != list_node)
        {
            return Vec::new();
        }
        let rows = self.bookmarks.rows();
        row_range(start, count, self.bookmarks.row_count())
            .filter_map(|index| {
                let row = rows.get(usize::try_from(index).ok()?)?;
                let built = match row {
                    Row::Folder {
                        folder,
                        files,
                        expanded,
                    } => {
                        let expansion = if *expanded {
                            Expansion::Expanded
                        } else {
                            Expansion::Collapsed
                        };
                        tree_row(index, 0, expansion, |tree| {
                            tree.flex(FlexProps::row().gap(6).align(Align::Center), |tree| {
                                tree.icon(if *expanded {
                                    Icon::FolderOpen
                                } else {
                                    Icon::Folder
                                });
                                tree.text(folder.label());
                                tree.text(
                                    TextProps::new(files.to_string()).color(ColorToken::TextMuted),
                                );
                            });
                        })
                    }
                    Row::File(bookmark) => tree_row(index, 1, Expansion::Leaf, |tree| {
                        tree.flex(FlexProps::row().gap(6).align(Align::Center), |tree| {
                            tree.icon(IconProps::new(Icon::Star).color(ColorToken::TextAccent));
                            tree.text(bookmark.name());
                        });
                    }),
                };
                match built {
                    Ok(row) => Some(row),
                    Err(error) => {
                        log(LogLevel::Error, &format!("row {index} left out: {error}"));
                        None
                    }
                }
            })
            .collect()
    }

    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::ContextChanged(editor) => self.editor_changed(editor),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext_sdk::context::ContextEffect;

    fn showing(path: &str) -> IoEvent {
        let file = FileRef {
            worktree: Some(1),
            path: path.into(),
        };
        IoEvent::ContextChanged(EditorContext {
            active: Some(file.clone()),
            open: vec![file],
            selection: None,
            trusted: Some(true),
        })
    }

    #[test]
    fn bookmarking_badges_the_file_and_toggling_folds_the_tree() {
        let mut demo = BookmarksDemo::new();
        demo.mount(PANEL).unwrap();
        take_host_effects();
        demo.handle_io(showing("src/main.rs")).unwrap();
        let (toggle, list) = {
            let nodes = demo.nodes.as_ref().unwrap();
            (nodes.toggle, nodes.list)
        };
        demo.handle_event(PANEL, UiEvent::Click(toggle)).unwrap();
        assert!(take_host_effects().iter().any(|effect| matches!(
            effect,
            HostEffect::Context(ContextEffect::DecorateFiles(decorations))
                if decorations.len() == 1 && decorations[0].badge.as_deref() == Some(BADGE)
        )));
        assert_eq!(demo.bookmarks.row_count(), 2);
        let rows = demo.list_rows(PANEL, list, 0, 10);
        assert_eq!(
            rows.iter()
                .map(|row| (row.indent, row.expansion))
                .collect::<Vec<_>>(),
            [(0, Expansion::Expanded), (1, Expansion::Leaf)]
        );

        let update = demo
            .handle_event(PANEL, UiEvent::RowToggled((list, 0, false)))
            .unwrap();
        assert_ne!(update, Update::Unchanged);
        assert_eq!(demo.bookmarks.row_count(), 1);

        demo.handle_event(PANEL, UiEvent::Click(toggle)).unwrap();
        assert_eq!(demo.bookmarks.row_count(), 0);
        assert!(take_host_effects().iter().any(|effect| matches!(
            effect,
            HostEffect::Context(ContextEffect::ClearFileDecorations(Some(paths)))
                if paths.len() == 1
        )));
    }
}
