//! Fyx demo 2: a panel that collects `TODO`, `FIXME` and `HACK` comments from the workspace.
//!
//! The host cannot read the worktree for extensions yet, so the files come from a [`TodoSource`];
//! [`MockWorktree`] serves fixed contents until a host-backed source can replace it.
//!
//! Surface `"todos"` (left dock panel): a summary line, a filter input matched against the comment
//! text and path, kind buttons (All / TODO / FIXME / HACK), a Rescan button, a virtualized list of
//! matching items and a detail line for the clicked row. Typing and kind buttons answer with
//! patches; when the set of matching items changes, the list node is rebuilt under a new id so
//! the host requests its rows again.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "todos";
const ROW_HEIGHT: u16 = 24;

pub struct SourceFile {
    /// Relative to the worktree root, `/`-separated.
    pub path: String,
    pub text: String,
}

/// Where the panel reads files from.
pub trait TodoSource {
    fn files(&self) -> Result<Vec<SourceFile>, String>;
}

/// Fixed file contents standing in for the worktree.
pub struct MockWorktree;

const MOCK_FILES: &[(&str, &str)] = &[
    (
        "src/main.rs",
        "fn main() {\n    // TODO: read the config path from the command line\n    let config = load_config(\"fyx.toml\");\n    // FIXME(sam): this panics when the file is missing\n    run(config.unwrap());\n}\n",
    ),
    (
        "src/render.rs",
        "pub fn render(tree: &Tree) {\n    // HACK: skip empty nodes until the layout pass handles them\n    for node in tree.nodes().filter(|node| !node.is_empty()) {\n        draw(node);\n    }\n    /* TODO: batch draw calls */\n}\n",
    ),
    (
        "web/app.ts",
        "export function start() {\n  // TODO(ui): show a spinner while connecting\n  connect();\n  // NOTE: TODOS in identifiers like TODO_LIST are not comments\n  const TODO_LIST = [];\n  return TODO_LIST;\n}\n",
    ),
    (
        "docs/README.md",
        "# Demo\n\n<!-- FIXME: screenshots are out of date -->\n\nRun `fyx` to start.\n",
    ),
];

impl TodoSource for MockWorktree {
    fn files(&self) -> Result<Vec<SourceFile>, String> {
        Ok(MOCK_FILES
            .iter()
            .map(|(path, text)| SourceFile {
                path: (*path).to_string(),
                text: (*text).to_string(),
            })
            .collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoKind {
    Todo,
    Fixme,
    Hack,
}

impl TodoKind {
    const ALL: [Self; 3] = [Self::Todo, Self::Fixme, Self::Hack];

    fn marker(self) -> &'static str {
        match self {
            Self::Todo => "TODO",
            Self::Fixme => "FIXME",
            Self::Hack => "HACK",
        }
    }

    fn color(self) -> ColorToken {
        match self {
            Self::Todo => ColorToken::TextAccent,
            Self::Fixme => ColorToken::Error,
            Self::Hack => ColorToken::Warning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub kind: TodoKind,
    pub path: String,
    /// 1-based.
    pub line: u32,
    pub message: String,
    pub source_line: String,
}

/// Finds the comment markers in `text`. A marker is an upper-case word on its own (so
/// `TODO_LIST` and `TODOS` do not count), optionally followed by `(owner)` and `:`.
pub fn scan(path: &str, text: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        let Some((kind, rest)) = find_marker(line) else {
            continue;
        };
        items.push(TodoItem {
            kind,
            path: path.to_string(),
            line: u32::try_from(line_index + 1).unwrap_or(u32::MAX),
            message: clean_message(rest),
            source_line: line.trim().to_string(),
        });
    }
    items
}

fn find_marker(line: &str) -> Option<(TodoKind, &str)> {
    let is_word_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    TodoKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let marker = kind.marker();
            line.match_indices(marker).find_map(|(start, _)| {
                let end = start + marker.len();
                let before_is_word = start > 0 && is_word_byte(line.as_bytes()[start - 1]);
                let after_is_word = line.as_bytes().get(end).copied().is_some_and(is_word_byte);
                (!before_is_word && !after_is_word).then(|| (start, kind, &line[end..]))
            })
        })
        .min_by_key(|(start, _, _)| *start)
        .map(|(_, kind, rest)| (kind, rest))
}

fn clean_message(rest: &str) -> String {
    let mut message = rest.trim_start();
    if let Some(after_owner) = message
        .strip_prefix('(')
        .and_then(|inner| inner.split_once(')'))
        .map(|(_, after)| after)
    {
        message = after_owner;
    }
    let message = message.trim_start_matches(':').trim();
    let message = message
        .strip_suffix("*/")
        .or_else(|| message.strip_suffix("-->"))
        .unwrap_or(message);
    message.trim().to_string()
}

struct TodoNodes {
    summary: NodeId,
    filter: NodeId,
    kind_buttons: Vec<(Option<TodoKind>, NodeId)>,
    rescan: NodeId,
    list: NodeId,
    detail: NodeId,
}

struct TodoPanel {
    source: Box<dyn TodoSource>,
    items: Vec<TodoItem>,
    file_count: usize,
    scan_error: Option<String>,
    query: String,
    kind: Option<TodoKind>,
    /// Indices into `items` of the rows the list shows.
    visible: Vec<usize>,
    selected: Option<usize>,
    tree: SurfaceTree,
    nodes: Option<TodoNodes>,
}

impl TodoPanel {
    fn with_source(source: Box<dyn TodoSource>) -> Self {
        let mut panel = Self {
            source,
            items: Vec::new(),
            file_count: 0,
            scan_error: None,
            query: String::new(),
            kind: None,
            visible: Vec::new(),
            selected: None,
            tree: SurfaceTree::new(),
            nodes: None,
        };
        panel.rescan();
        panel
    }

    fn rescan(&mut self) {
        match self.source.files() {
            Ok(files) => {
                self.file_count = files.len();
                self.items = files
                    .iter()
                    .flat_map(|file| scan(&file.path, &file.text))
                    .collect();
                self.scan_error = None;
            }
            Err(error) => {
                log(
                    LogLevel::Error,
                    &format!("reading the worktree failed: {error}"),
                );
                self.file_count = 0;
                self.items.clear();
                self.scan_error = Some(error);
            }
        }
        self.selected = None;
        self.refilter();
    }

    fn refilter(&mut self) {
        let query = self.query.to_lowercase();
        self.visible = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| self.kind.is_none_or(|kind| kind == item.kind))
            .filter(|(_, item)| {
                query.is_empty()
                    || item.message.to_lowercase().contains(&query)
                    || item.path.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        if self
            .selected
            .is_some_and(|selected| !self.visible.contains(&selected))
        {
            self.selected = None;
        }
    }

    fn summary(&self) -> TextProps {
        let text = match &self.scan_error {
            Some(error) => format!("Could not read the worktree: {error}"),
            None => format!(
                "{} of {} items in {} files",
                self.visible.len(),
                self.items.len(),
                self.file_count
            ),
        };
        let color = if self.scan_error.is_some() {
            ColorToken::Error
        } else {
            ColorToken::TextMuted
        };
        TextProps::new(text).color(color)
    }

    fn list(&self) -> ListProps {
        ListProps::new(
            u32::try_from(self.visible.len()).unwrap_or(u32::MAX),
            ROW_HEIGHT,
        )
    }

    fn detail(&self) -> TextProps {
        match self.selected.and_then(|index| self.items.get(index)) {
            Some(item) => {
                TextProps::new(format!("{}:{}  {}", item.path, item.line, item.source_line))
                    .monospace()
            }
            None => {
                TextProps::new("Click an item to see its source line.").color(ColorToken::TextMuted)
            }
        }
    }

    fn build(&self, tree: &mut TreeBuilder) -> TodoNodes {
        let mut nodes = TodoNodes {
            summary: 0,
            filter: 0,
            kind_buttons: Vec::new(),
            rescan: 0,
            list: 0,
            detail: 0,
        };
        tree.flex(FlexProps::column().gap(8).padding(8).grow(), |tree| {
            tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                tree.text(TextProps::new("Workspace TODOs").bold());
                nodes.summary = tree.text(self.summary());
            });
            nodes.filter = tree.input(filter_input(&self.query));
            tree.flex(FlexProps::row().gap(4).align(Align::Center), |tree| {
                for kind in [
                    None,
                    Some(TodoKind::Todo),
                    Some(TodoKind::Fixme),
                    Some(TodoKind::Hack),
                ] {
                    let button = tree.button(kind_button(kind, self.kind));
                    nodes.kind_buttons.push((kind, button));
                }
                nodes.rescan = tree.button(ButtonProps::new("Rescan").icon(Icon::Refresh));
            });
            nodes.list = tree.list(self.list());
            nodes.detail = tree.text(self.detail());
        });
        nodes
    }

    /// Brings every node in line with the state; the list is rebuilt when its rows changed.
    /// Applies `change` and syncs the tree. The list is rebuilt only when the rows it shows
    /// changed, since a rebuild loses the scroll position.
    fn update_rows(&mut self, change: impl FnOnce(&mut Self)) -> Result<Update, String> {
        let rows_before = self.visible_rows();
        change(self);
        let rows_changed = self.visible_rows() != rows_before;
        self.sync(rows_changed)
    }

    fn visible_rows(&self) -> Vec<TodoItem> {
        self.visible
            .iter()
            .filter_map(|&index| self.items.get(index))
            .cloned()
            .collect()
    }

    fn sync(&mut self, rows_changed: bool) -> Result<Update, String> {
        let summary = self.summary();
        let detail = self.detail();
        let list = self.list();
        let active_kind = self.kind;
        let Some(nodes) = &mut self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        self.tree.set_props(nodes.summary, summary)?;
        for (kind, button) in &nodes.kind_buttons {
            self.tree
                .set_props(*button, kind_button(*kind, active_kind))?;
        }
        self.tree.set_props(nodes.detail, detail)?;
        if rows_changed {
            let rebuilt = self.tree.rebuild(nodes.list, |tree| {
                tree.list(list);
            })?;
            if let Some(&list) = rebuilt.first() {
                nodes.list = list;
            }
        }
        Ok(self.tree.take_update())
    }

    fn row(&self, index: u32, tree: &mut TreeBuilder) {
        let Some(item) = usize::try_from(index)
            .ok()
            .and_then(|index| self.visible.get(index))
            .and_then(|&item| self.items.get(item))
        else {
            return;
        };
        tree.flex(
            FlexProps::row().gap(8).padding(4).align(Align::Center),
            |tree| {
                tree.text(
                    TextProps::new(item.kind.marker())
                        .color(item.kind.color())
                        .bold()
                        .monospace(),
                );
                tree.text(item.message.as_str());
                tree.text(
                    TextProps::new(format!("{}:{}", item.path, item.line))
                        .color(ColorToken::TextMuted),
                );
            },
        );
    }
}

fn kind_button(kind: Option<TodoKind>, active_kind: Option<TodoKind>) -> ButtonProps {
    let label = kind.map_or("All", TodoKind::marker);
    let style = if kind == active_kind {
        ButtonStyle::Filled
    } else {
        ButtonStyle::Subtle
    };
    ButtonProps::new(label).style(style)
}

fn filter_input(query: &str) -> InputProps {
    InputProps::new(query).placeholder("Filter by text or path")
}

fn check_surface(surface_id: &str) -> Result<(), String> {
    if surface_id == SURFACE_ID {
        Ok(())
    } else {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

impl Extension for TodoPanel {
    fn new() -> Self {
        Self::with_source(Box::new(MockWorktree))
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            SURFACE_ID,
            "TODOs",
            DockPosition::Left,
            Icon::ListTodo,
            360,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        check_surface(surface_id)?;
        let mut tree = std::mem::take(&mut self.tree);
        let mut nodes = None;
        let mounted = tree.mount(|tree| nodes = Some(self.build(tree)));
        self.tree = tree;
        let mounted = mounted?;
        self.nodes = nodes;
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        check_surface(surface_id)?;
        self.tree.apply_event(&event);
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let (filter, rescan, list) = (nodes.filter, nodes.rescan, nodes.list);
        let clicked_kind = match event {
            UiEvent::Click(id) => nodes
                .kind_buttons
                .iter()
                .find(|(_, button)| *button == id)
                .map(|(kind, _)| *kind),
            _ => None,
        };

        match event {
            UiEvent::InputChanged((id, value)) | UiEvent::InputSubmitted((id, value))
                if id == filter =>
            {
                if value == self.query {
                    return Ok(Update::Unchanged);
                }
                self.update_rows(|panel| {
                    panel.query = value;
                    panel.refilter();
                })
            }
            UiEvent::Click(id) if id == rescan => self.update_rows(Self::rescan),
            UiEvent::Click(_) => match clicked_kind {
                Some(kind) if kind != self.kind => self.update_rows(|panel| {
                    panel.kind = kind;
                    panel.refilter();
                }),
                _ => Ok(Update::Unchanged),
            },
            UiEvent::RowClick((id, index)) if id == list => {
                self.selected = usize::try_from(index)
                    .ok()
                    .and_then(|index| self.visible.get(index))
                    .copied();
                self.sync(false)
            }
            UiEvent::InputChanged(_)
            | UiEvent::InputSubmitted(_)
            | UiEvent::RowClick(_)
            | UiEvent::RowSelected(_)
            | UiEvent::Dismissed(_) => Ok(Update::Unchanged),
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
        let row_count = self.list().row_count;
        list_rows(start, count, row_count, |index, tree| self.row(index, tree))
    }
}

// fyx:ui 0.1 by default, which is what Fyx runs today; `--features fyx-ui-0-2` builds the 0.2
// world from the same code.
#[cfg(not(feature = "fyx-ui-0-2"))]
register_extension!(TodoPanel, v0_1);
#[cfg(feature = "fyx-ui-0-2")]
register_extension!(TodoPanel);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_marked_comments_only() {
        let items = MOCK_FILES
            .iter()
            .flat_map(|(path, text)| scan(path, text))
            .map(|item| (item.kind, item.path, item.line, item.message))
            .collect::<Vec<_>>();
        let expected = [
            (
                TodoKind::Todo,
                "src/main.rs",
                2,
                "read the config path from the command line",
            ),
            (
                TodoKind::Fixme,
                "src/main.rs",
                4,
                "this panics when the file is missing",
            ),
            (
                TodoKind::Hack,
                "src/render.rs",
                2,
                "skip empty nodes until the layout pass handles them",
            ),
            (TodoKind::Todo, "src/render.rs", 6, "batch draw calls"),
            (
                TodoKind::Todo,
                "web/app.ts",
                2,
                "show a spinner while connecting",
            ),
            (
                TodoKind::Fixme,
                "docs/README.md",
                3,
                "screenshots are out of date",
            ),
        ]
        .map(|(kind, path, line, message)| (kind, path.to_string(), line, message.to_string()));
        assert_eq!(items, expected);
    }

    struct FailingSource;

    impl TodoSource for FailingSource {
        fn files(&self) -> Result<Vec<SourceFile>, String> {
            Err("no worktree access".into())
        }
    }

    fn summary(panel: &TodoPanel) -> String {
        panel.summary().content
    }

    #[test]
    fn filters_answer_with_patches() {
        let mut panel = TodoPanel::new();
        panel.mount("todos").expect("mount");
        assert_eq!(summary(&panel), "6 of 6 items in 4 files");
        let (filter, fixme_button, list) = {
            let nodes = panel.nodes.as_ref().expect("mounted");
            (nodes.filter, nodes.kind_buttons[2].1, nodes.list)
        };

        let update = panel
            .handle_event("todos", UiEvent::InputChanged((filter, "main".into())))
            .expect("filter");
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        assert!(patches.contains(&Patch::Remove(list)));
        assert!(!patches.iter().any(|patch| matches!(
            patch,
            Patch::SetProps((id, _)) if *id == filter
        )));
        assert_eq!(summary(&panel), "2 of 6 items in 4 files");
        let list = panel.nodes.as_ref().expect("mounted").list;
        assert_eq!(panel.list_rows("todos", list, 0, 10).len(), 2);

        let update = panel
            .handle_event("todos", UiEvent::InputChanged((filter, "mai".into())))
            .expect("same rows");
        assert_eq!(panel.nodes.as_ref().expect("mounted").list, list);
        assert!(matches!(&update, Update::Unchanged), "{update:?}");
        let update = panel
            .handle_event("todos", UiEvent::InputChanged((filter, "main".into())))
            .expect("same rows again");
        assert!(matches!(&update, Update::Unchanged), "{update:?}");

        panel
            .handle_event("todos", UiEvent::Click(fixme_button))
            .expect("kind");
        assert_ne!(panel.nodes.as_ref().expect("mounted").list, list);
        assert_eq!(summary(&panel), "1 of 6 items in 4 files");
        assert_eq!(
            panel
                .handle_event("todos", UiEvent::Click(fixme_button))
                .expect("same kind"),
            Update::Unchanged
        );

        let list = panel.nodes.as_ref().expect("mounted").list;
        let update = panel
            .handle_event("todos", UiEvent::RowClick((list, 0)))
            .expect("row click");
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        assert_eq!(patches.len(), 1);
        assert!(
            panel
                .detail()
                .content
                .starts_with("src/main.rs:4  // FIXME")
        );
    }

    #[test]
    fn source_errors_are_shown() {
        let mut panel = TodoPanel::with_source(Box::new(FailingSource));
        panel.mount("todos").expect("mount");
        assert_eq!(
            summary(&panel),
            "Could not read the worktree: no worktree access"
        );
    }
}
