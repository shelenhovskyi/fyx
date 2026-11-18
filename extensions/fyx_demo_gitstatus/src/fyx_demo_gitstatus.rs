//! Fyx demo: a "git status" panel on mocked data.
//!
//! `fyx:ui@0.1.0` gives extensions no access to the repository, so the panel shows the fixed
//! [`MOCK_STATUS`]. Each changed file is a row of a virtualized list with a status icon and letter
//! coloured by theme token; clicking a row shows that file's details below the list.
//!
//! Host tests and the docs rely on this behaviour, so keep it stable:
//!
//! - One surface, id `"git-status"`: a left dock panel titled "Git status", icon `git_branch`,
//!   default size 320.
//! - The first `mount("git-status")` returns 9 nodes, parents first:
//!
//!   | id | parent | props |
//!   |---|---|---|
//!   | 1 | none | column flex, gap 8, padding 8, grow |
//!   | 2 | 1 | row flex, gap 8, align center |
//!   | 3 | 2 | icon `git_branch`, `text-accent` |
//!   | 4 | 2 | text `main`, bold |
//!   | 5 | 2 | text `2 ahead, 1 behind origin/main`, `text-muted` |
//!   | 6 | 1 | text `7 changed files, 3 staged`, `text-muted` |
//!   | 7 | 1 | list, 7 rows of height 24 |
//!   | 8 | 1 | column flex, gap 4, padding 8, background `elevated-surface` (details) |
//!   | 9 | 8 | text `Click a file to see its details.`, `text-muted` |
//!
//! - `list-rows("git-status", 7, start, count)` returns rows clamped to the 7 files. Row `i` is a
//!   row flex (id `0x8000_0000`) holding an icon, a bold monospace status letter (both coloured
//!   by status) and the path, plus a `staged` text when the change is staged.
//! - `row-click(7, i)` answers with patches that remove the children of 8 and insert the
//!   details of file `i`: a row with its icon and bold path, a status line coloured by status,
//!   and a row with `+added` (`success`) and `-removed` (`error`) counts. Clicking the selected
//!   row again, a row index past the end, or another event answers `unchanged`. An unknown
//!   surface id is an error.
//! - The `SurfaceTree` id allocator survives remounts; the selection survives a remount.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "git-status";
const ROW_HEIGHT: u16 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed { from: &'static str },
    Untracked,
    Conflicted,
}

impl FileStatus {
    fn letter(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed { .. } => "R",
            Self::Untracked => "?",
            Self::Conflicted => "U",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Modified => Icon::Pencil,
            Self::Added => Icon::Plus,
            Self::Deleted => Icon::Trash,
            Self::Renamed { .. } => Icon::ArrowRight,
            Self::Untracked => Icon::Help,
            // The frozen 0.2 set has no merge-conflict icon.
            Self::Conflicted => Icon::Warning,
        }
    }

    fn color(self) -> ColorToken {
        match self {
            Self::Modified => ColorToken::Warning,
            Self::Added => ColorToken::Success,
            Self::Deleted | Self::Conflicted => ColorToken::Error,
            Self::Renamed { .. } => ColorToken::TextAccent,
            Self::Untracked => ColorToken::TextMuted,
        }
    }

    fn description(self) -> String {
        match self {
            Self::Modified => "Modified".to_string(),
            Self::Added => "Added".to_string(),
            Self::Deleted => "Deleted".to_string(),
            Self::Renamed { from } => format!("Renamed from {from}"),
            Self::Untracked => "Untracked".to_string(),
            Self::Conflicted => "Conflicted: resolve before committing".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileChange {
    pub path: &'static str,
    pub status: FileStatus,
    pub staged: bool,
    pub added_lines: u32,
    pub removed_lines: u32,
}

pub struct RepositoryStatus {
    pub branch: &'static str,
    pub upstream: &'static str,
    pub ahead: u32,
    pub behind: u32,
    pub changes: &'static [FileChange],
}

pub const MOCK_STATUS: RepositoryStatus = RepositoryStatus {
    branch: "main",
    upstream: "origin/main",
    ahead: 2,
    behind: 1,
    changes: &[
        FileChange {
            path: "src/editor.rs",
            status: FileStatus::Modified,
            staged: true,
            added_lines: 12,
            removed_lines: 3,
        },
        FileChange {
            path: "src/git_panel.rs",
            status: FileStatus::Added,
            staged: true,
            added_lines: 84,
            removed_lines: 0,
        },
        FileChange {
            path: "src/legacy_view.rs",
            status: FileStatus::Deleted,
            staged: false,
            added_lines: 0,
            removed_lines: 41,
        },
        FileChange {
            path: "docs/guide.md",
            status: FileStatus::Renamed {
                from: "docs/GUIDE.md",
            },
            staged: true,
            added_lines: 2,
            removed_lines: 2,
        },
        FileChange {
            path: "README.md",
            status: FileStatus::Modified,
            staged: false,
            added_lines: 1,
            removed_lines: 1,
        },
        FileChange {
            path: "Cargo.lock",
            status: FileStatus::Conflicted,
            staged: false,
            added_lines: 5,
            removed_lines: 5,
        },
        FileChange {
            path: "notes/todo.txt",
            status: FileStatus::Untracked,
            staged: false,
            added_lines: 7,
            removed_lines: 0,
        },
    ],
};

struct GitStatusNodes {
    list: NodeId,
    details: NodeId,
}

struct GitStatusPanel {
    status: RepositoryStatus,
    selected: Option<usize>,
    tree: SurfaceTree,
    nodes: Option<GitStatusNodes>,
}

impl GitStatusPanel {
    fn tracking(&self) -> TextProps {
        let status = &self.status;
        TextProps::new(format!(
            "{} ahead, {} behind {}",
            status.ahead, status.behind, status.upstream
        ))
        .color(ColorToken::TextMuted)
    }

    fn summary(&self) -> TextProps {
        let changes = self.status.changes;
        let staged = changes.iter().filter(|change| change.staged).count();
        let files = if changes.len() == 1 { "file" } else { "files" };
        TextProps::new(format!(
            "{} changed {files}, {staged} staged",
            changes.len()
        ))
        .color(ColorToken::TextMuted)
    }

    fn list(&self) -> ListProps {
        ListProps::new(
            u32::try_from(self.status.changes.len()).unwrap_or(u32::MAX),
            ROW_HEIGHT,
        )
    }

    fn selected_change(&self) -> Option<&FileChange> {
        self.selected
            .and_then(|index| self.status.changes.get(index))
    }

    fn build_details(change: Option<&FileChange>, tree: &mut TreeBuilder) {
        let Some(change) = change else {
            tree.text(
                TextProps::new("Click a file to see its details.").color(ColorToken::TextMuted),
            );
            return;
        };
        let status = change.status;
        tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
            tree.icon(IconProps::new(status.icon()).color(status.color()));
            tree.text(TextProps::new(change.path).bold());
        });
        let staging = if change.staged {
            "staged"
        } else {
            "not staged"
        };
        tree.text(
            TextProps::new(format!("{}, {staging}", status.description())).color(status.color()),
        );
        tree.flex(FlexProps::row().gap(8), |tree| {
            tree.text(
                TextProps::new(format!("+{}", change.added_lines))
                    .color(ColorToken::Success)
                    .monospace(),
            );
            tree.text(
                TextProps::new(format!("-{}", change.removed_lines))
                    .color(ColorToken::Error)
                    .monospace(),
            );
        });
    }

    fn build_row(change: &FileChange, tree: &mut TreeBuilder) {
        let status = change.status;
        tree.flex(
            FlexProps::row().gap(8).padding(4).align(Align::Center),
            |tree| {
                tree.icon(IconProps::new(status.icon()).color(status.color()));
                tree.text(
                    TextProps::new(status.letter())
                        .color(status.color())
                        .bold()
                        .monospace(),
                );
                tree.text(change.path);
                if change.staged {
                    tree.text(TextProps::new("staged").color(ColorToken::TextMuted));
                }
            },
        );
    }

    fn select(&mut self, index: u32) -> Result<Update, String> {
        let Some(index) = usize::try_from(index)
            .ok()
            .filter(|&index| index < self.status.changes.len())
        else {
            return Ok(Update::Unchanged);
        };
        if self.selected == Some(index) {
            return Ok(Update::Unchanged);
        }
        let Some(details) = self.nodes.as_ref().map(|nodes| nodes.details) else {
            return Err(TreeError::NotMounted.into());
        };
        self.selected = Some(index);
        let change = self.selected_change().copied();
        self.tree.clear_children(details)?;
        self.tree.insert(details, None, |tree| {
            Self::build_details(change.as_ref(), tree)
        })?;
        Ok(self.tree.take_update())
    }
}

fn check_surface(surface_id: &str) -> Result<(), String> {
    if surface_id == SURFACE_ID {
        Ok(())
    } else {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

impl Extension for GitStatusPanel {
    fn new() -> Self {
        Self {
            status: MOCK_STATUS,
            selected: None,
            tree: SurfaceTree::new(),
            nodes: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            SURFACE_ID,
            "Git status",
            DockPosition::Left,
            Icon::GitBranch,
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        check_surface(surface_id)?;
        let (tracking, summary, list) = (self.tracking(), self.summary(), self.list());
        let branch = self.status.branch;
        let selected = self.selected_change().copied();
        let mut nodes = GitStatusNodes {
            list: 0,
            details: 0,
        };
        let mounted = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8).grow(), |tree| {
                tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                    tree.icon(IconProps::new(Icon::GitBranch).color(ColorToken::TextAccent));
                    tree.text(TextProps::new(branch).bold());
                    tree.text(tracking);
                });
                tree.text(summary);
                nodes.list = tree.list(list);
                nodes.details = tree.flex(
                    FlexProps::column()
                        .gap(4)
                        .padding(8)
                        .background(ColorToken::ElevatedSurface),
                    |tree| Self::build_details(selected.as_ref(), tree),
                );
            });
        })?;
        self.nodes = Some(nodes);
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        check_surface(surface_id)?;
        let Some(list) = self.nodes.as_ref().map(|nodes| nodes.list) else {
            return Err(TreeError::NotMounted.into());
        };
        match event {
            UiEvent::RowClick((id, index)) if id == list => self.select(index),
            UiEvent::Click(_)
            | UiEvent::InputChanged(_)
            | UiEvent::InputSubmitted(_)
            | UiEvent::RowClick(_)
            | UiEvent::RowSelected(_)
            | UiEvent::Dismissed(_) => Ok(Update::Unchanged),
            // The SDK's fyx-ui-0-3 feature adds events this demo never gets.
            #[allow(unreachable_patterns)]
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
        let is_current_list = self
            .nodes
            .as_ref()
            .is_some_and(|nodes| nodes.list == list_node);
        if surface_id != SURFACE_ID || !is_current_list {
            return Vec::new();
        }
        let changes = self.status.changes;
        list_rows(start, count, self.list().row_count, |index, tree| {
            if let Some(change) = usize::try_from(index)
                .ok()
                .and_then(|index| changes.get(index))
            {
                Self::build_row(change, tree);
            }
        })
    }
}

// fyx:ui 0.1 by default, which is what every Fyx runs; `--features fyx-ui-0-2` builds the 0.2
// world and `--features fyx-ui-0-3` the staged 0.3 world from the same code.
#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(GitStatusPanel, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(GitStatusPanel, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(GitStatusPanel);

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_patches(update: Update) -> Vec<Patch> {
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        patches
    }

    fn inserted_texts(patches: &[Patch]) -> Vec<String> {
        patches
            .iter()
            .filter_map(|patch| match patch {
                Patch::Insert(InsertOp {
                    node:
                        Node {
                            props: Props::Text(text),
                            ..
                        },
                    ..
                }) => Some(text.content.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn first_mount_is_pinned() {
        let mut panel = GitStatusPanel::new();
        assert_eq!(panel.surfaces()[0].id, "git-status");
        let nodes = panel.mount("git-status").expect("mount");
        let shape = nodes
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(2)),
                (4, Some(2)),
                (5, Some(2)),
                (6, Some(1)),
                (7, Some(1)),
                (8, Some(1)),
                (9, Some(8)),
            ]
        );
        assert_eq!(
            nodes[4].props,
            TextProps::new("2 ahead, 1 behind origin/main")
                .color(ColorToken::TextMuted)
                .into()
        );
        assert_eq!(
            nodes[5].props,
            TextProps::new("7 changed files, 3 staged")
                .color(ColorToken::TextMuted)
                .into()
        );
        assert_eq!(nodes[6].props, ListProps::new(7, 24).into());
    }

    #[test]
    fn rows_have_coloured_icons() {
        let mut panel = GitStatusPanel::new();
        panel.mount("git-status").expect("mount");
        let rows = panel.list_rows("git-status", 7, 0, 100);
        assert_eq!(rows.len(), 7);

        let first = &rows[0].nodes;
        assert_eq!(first[0].id, ROW_NODE_ID_BASE);
        assert_eq!(
            first[1].props,
            IconProps::new(Icon::Pencil)
                .color(ColorToken::Warning)
                .into()
        );
        assert_eq!(
            first[2].props,
            TextProps::new("M")
                .color(ColorToken::Warning)
                .bold()
                .monospace()
                .into()
        );
        assert_eq!(first[3].props, TextProps::new("src/editor.rs").into());
        assert_eq!(first.len(), 5);

        let deleted = &rows[2].nodes;
        assert_eq!(
            deleted[1].props,
            IconProps::new(Icon::Trash).color(ColorToken::Error).into()
        );
        assert_eq!(deleted.len(), 4);

        assert_eq!(panel.list_rows("git-status", 7, 5, 10).len(), 2);
        assert!(panel.list_rows("git-status", 8, 0, 10).is_empty());
    }

    #[test]
    fn row_click_shows_details() {
        let mut panel = GitStatusPanel::new();
        panel.mount("git-status").expect("mount");

        let update = panel
            .handle_event("git-status", UiEvent::RowClick((7, 3)))
            .expect("row click");
        let patches = expect_patches(update);
        assert_eq!(patches[0], Patch::Remove(9));
        assert!(
            patches[1..]
                .iter()
                .all(|patch| matches!(patch, Patch::Insert(_)))
        );
        assert_eq!(
            inserted_texts(&patches),
            [
                "docs/guide.md",
                "Renamed from docs/GUIDE.md, staged",
                "+2",
                "-2"
            ]
        );

        assert_eq!(
            panel
                .handle_event("git-status", UiEvent::RowClick((7, 3)))
                .expect("same row"),
            Update::Unchanged
        );
        assert_eq!(
            panel
                .handle_event("git-status", UiEvent::RowClick((7, 7)))
                .expect("past the end"),
            Update::Unchanged
        );
        assert_eq!(
            panel
                .handle_event("git-status", UiEvent::Click(7))
                .expect("click"),
            Update::Unchanged
        );

        let update = panel
            .handle_event("git-status", UiEvent::RowClick((7, 5)))
            .expect("other row");
        let patches = expect_patches(update);
        let removed = patches
            .iter()
            .filter(|patch| matches!(patch, Patch::Remove(_)))
            .count();
        assert_eq!(removed, 3);
        assert_eq!(
            inserted_texts(&patches)[1],
            "Conflicted: resolve before committing, not staged"
        );
        assert!(
            panel
                .handle_event("other", UiEvent::RowClick((7, 0)))
                .is_err()
        );
    }

    #[test]
    fn selection_survives_a_remount() {
        let mut panel = GitStatusPanel::new();
        panel.mount("git-status").expect("mount");
        panel
            .handle_event("git-status", UiEvent::RowClick((7, 1)))
            .expect("row click");

        let nodes = panel.mount("git-status").expect("remount");
        assert!(nodes.iter().all(|node| node.id > 9));
        let texts = nodes
            .iter()
            .filter_map(|node| match &node.props {
                Props::Text(text) => Some(text.content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(texts.contains(&"Added, staged"));
        let list = panel.nodes.as_ref().expect("mounted").list;
        assert!(panel.list_rows("git-status", 7, 0, 10).is_empty());
        assert_eq!(panel.list_rows("git-status", list, 0, 10).len(), 7);
    }

    /// Every update, applied to the host's own tree model, must be accepted and leave it equal to
    /// the extension's mirror.
    fn assert_host_accepts(panel: &mut GitStatusPanel, events: Vec<UiEvent>) {
        let nodes = panel.mount(SURFACE_ID).expect("mount");
        let mut host = ui_ext::Tree::from_nodes(
            nodes.iter().map(Into::into).collect(),
            TreeLimits::default(),
        )
        .expect("the host accepts the mount");
        for event in events {
            // Like the host: typed text lands in its own tree before the event is sent.
            if let UiEvent::InputChanged((id, value)) | UiEvent::InputSubmitted((id, value)) =
                &event
                && let Some(ui_ext::Props::Input(input)) =
                    host.get(*id).map(|node| node.props().clone())
            {
                let typed = ui_ext::Props::Input(ui_ext::InputProps {
                    value: value.clone(),
                    ..input
                });
                host.apply_patches(vec![ui_ext::Patch::SetProps(*id, typed)])
                    .expect("the host records typed text");
            }
            let update = panel.handle_event(SURFACE_ID, event).expect("handle-event");
            assert!(
                !matches!(update, Update::Replace(_)),
                "events answer with patches, never replace"
            );
            host.apply_update((&update).into())
                .expect("the host accepts the update");
            let mirror = panel
                .tree
                .to_nodes()
                .iter()
                .map(Into::into)
                .collect::<Vec<ui_ext::Node>>();
            assert_eq!(host.to_nodes(), mirror);
        }
    }

    #[test]
    fn host_accepts_every_update() {
        let mut panel = GitStatusPanel::new();
        assert_host_accepts(
            &mut panel,
            vec![
                UiEvent::RowClick((7, 0)),
                UiEvent::RowClick((7, 1)),
                UiEvent::RowClick((7, 2)),
                UiEvent::RowClick((7, 3)),
                UiEvent::RowClick((7, 4)),
                UiEvent::RowClick((7, 5)),
                UiEvent::RowClick((7, 6)),
                UiEvent::RowClick((7, 6)),
                UiEvent::RowClick((7, 9)),
                UiEvent::Click(3),
            ],
        );
    }
}
