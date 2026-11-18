//! Fyx demo 7: a TODO highlighter. It finds `TODO`, `FIXME` and `HACK` comments and places an
//! editor decoration above each one (`fyx:ui/decorations`, `docs/fyx/wit-0.2/design.md`, L).
//! Decorations need `fyx:ui` 0.2: the default 0.1 build shows the panel and reports that placing
//! them is unavailable; build with `--features fyx-ui-0-2` for the real thing.
//!
//! The host gives extensions no access to the worktree, so the files come from a fixed
//! [`MOCK_FILES`] set, the same four files as `fyx_demo_todo`. Open a project with those paths
//! (`src/main.rs`, `src/render.rs`, `web/app.ts`, `docs/README.md`) to see the blocks.
//!
//! - Panel `"todo-highlights"` (left dock, title "TODO highlights", icon `todo-pending`, default
//!   size 320):
//!   - a summary, `N comments in F files, H highlighted`;
//!   - `Highlight all` and `Clear` buttons, with tooltips;
//!   - a status line, empty until a host call fails;
//!   - a list with one row per comment: an icon (`todo-pending`, or `todo-complete` while it is
//!     highlighted), `path:line` (1-based) and the comment. A row click places or removes that
//!     comment's decoration.
//! - Decoration `todo-<n>` for comment `n`: above its line, one line high. A row flex with ids 1
//!   to 4: the row, an icon coloured by kind, the text `KIND: comment`, and a `Done` button that
//!   removes the decoration and updates the panel by a push.

use ui_ext_sdk::*;

const PANEL: &str = "todo-highlights";
const ROW_HEIGHT: u16 = 24;
const DECORATION_DONE: NodeId = 4;

/// The same fixed files as `fyx_demo_todo`.
pub const MOCK_FILES: &[(&str, &str)] = &[
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Todo,
    Fixme,
    Hack,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Todo, Kind::Fixme, Kind::Hack];

    fn marker(self) -> &'static str {
        match self {
            Kind::Todo => "TODO",
            Kind::Fixme => "FIXME",
            Kind::Hack => "HACK",
        }
    }

    fn color(self) -> ColorToken {
        match self {
            Kind::Todo => ColorToken::TextAccent,
            Kind::Fixme => ColorToken::Error,
            Kind::Hack => ColorToken::Warning,
        }
    }

    fn icon(self) -> Icon {
        match self {
            Kind::Todo => Icon::TodoPending,
            Kind::Fixme => Icon::Error,
            Kind::Hack => Icon::Warning,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    pub path: &'static str,
    /// Zero-based, as a decoration location counts lines.
    pub line: u32,
    pub kind: Kind,
    pub text: String,
}

const COMMENT_STARTS: [&str; 4] = ["//", "/*", "<!--", "#"];

/// The marker comments of one file: a marker inside a comment, followed by `:` or `(`, so
/// `TODOS` and `TODO_LIST` do not count.
pub fn scan(path: &'static str, text: &str) -> Vec<Comment> {
    let mut comments = Vec::new();
    for (line, content) in text.lines().enumerate() {
        let Some(comment_start) = COMMENT_STARTS
            .iter()
            .filter_map(|start| content.find(start))
            .min()
        else {
            continue;
        };
        let comment = &content[comment_start..];
        for kind in Kind::ALL {
            let Some(at) = comment.find(kind.marker()) else {
                continue;
            };
            let rest = &comment[at + kind.marker().len()..];
            let rest = match rest.chars().next() {
                Some('(') => rest.split_once(')').map_or("", |(_, rest)| rest),
                Some(':') => rest,
                _ => continue,
            };
            let text = rest
                .trim_start_matches(':')
                .trim_end_matches("*/")
                .trim_end_matches("-->")
                .trim();
            comments.push(Comment {
                path,
                line: u32::try_from(line).unwrap_or(u32::MAX),
                kind,
                text: text.to_string(),
            });
            break;
        }
    }
    comments
}

pub fn scan_mock_files() -> Vec<Comment> {
    MOCK_FILES
        .iter()
        .flat_map(|(path, text)| scan(path, text))
        .collect()
}

fn decoration_id(index: usize) -> String {
    format!("todo-{index}")
}

fn decoration_index(id: &str) -> Option<usize> {
    id.strip_prefix("todo-")?.parse().ok()
}

fn host_error(error: HostError) -> String {
    let code = match error.code {
        HostErrorCode::UnknownSurface => "unknown-surface",
        HostErrorCode::LimitExceeded => "limit-exceeded",
        HostErrorCode::InvalidArgument => "invalid-argument",
        HostErrorCode::NotPermitted => "not-permitted",
        HostErrorCode::Unavailable => "unavailable",
        HostErrorCode::Denied => "denied",
    };
    format!("{code}: {}", error.message)
}

struct PanelNodes {
    summary: NodeId,
    highlight_all: NodeId,
    clear: NodeId,
    status: NodeId,
    list: NodeId,
}

struct Highlighter {
    comments: Vec<Comment>,
    highlighted: Vec<bool>,
    status: String,
    panel: SurfaceTree,
    nodes: Option<PanelNodes>,
}

impl Highlighter {
    fn summary(&self) -> TextProps {
        let files = MOCK_FILES
            .iter()
            .filter(|(path, _)| self.comments.iter().any(|comment| comment.path == *path))
            .count();
        let highlighted = self.highlighted.iter().filter(|shown| **shown).count();
        TextProps::new(format!(
            "{} comments in {files} files, {highlighted} highlighted",
            self.comments.len()
        ))
    }

    fn status_label(&self) -> TextProps {
        TextProps::new(self.status.clone()).color(ColorToken::Error)
    }

    fn decoration_tree(comment: &Comment) -> Vec<Node> {
        let mut ids = IdAllocator::new();
        let mut tree = TreeBuilder::new(&mut ids);
        tree.flex(
            FlexProps::row()
                .gap(6)
                .padding(2)
                .align(Align::Center)
                .background(ColorToken::ElevatedSurface),
            |tree| {
                tree.icon(IconProps::new(comment.kind.icon()).color(comment.kind.color()));
                tree.text(
                    TextProps::new(format!("{}: {}", comment.kind.marker(), comment.text))
                        .color(comment.kind.color()),
                );
                tree.button(
                    ButtonProps::new("Done")
                        .icon(Icon::Check)
                        .style(ButtonStyle::Subtle)
                        .tooltip("Hide this highlight"),
                );
            },
        );
        tree.finish()
    }

    fn location(comment: &Comment) -> DecorationLocation {
        DecorationLocation {
            target: DecorationTarget::Path(comment.path.to_string()),
            line: comment.line,
            placement: DecorationPlacement::Above,
            height_lines: Some(1),
        }
    }

    /// Places or removes comment `index`'s decoration; `Err` is the host's refusal.
    fn set_highlight(&mut self, index: usize, shown: bool) -> Result<(), String> {
        if self.highlighted[index] == shown {
            return Ok(());
        }
        let id = decoration_id(index);
        let result = if shown {
            let comment = &self.comments[index];
            place_decoration(
                &id,
                &Self::location(comment),
                &Self::decoration_tree(comment),
            )
        } else {
            remove_decoration(&id)
        };
        result.map_err(host_error)?;
        self.highlighted[index] = shown;
        Ok(())
    }

    /// Queues the summary, the status line and the given rows on the panel.
    fn refresh_panel(&mut self, rows: Option<RowRange>) -> Result<(), String> {
        let Some(nodes) = &self.nodes else {
            return Ok(());
        };
        let (summary, status, list) = (nodes.summary, nodes.status, nodes.list);
        self.panel.set_props(summary, self.summary())?;
        self.panel.set_props(status, self.status_label())?;
        self.panel.invalidate_rows(list, rows)?;
        Ok(())
    }

    fn set_all(&mut self, shown: bool) -> Result<Update, String> {
        self.status.clear();
        for index in 0..self.comments.len() {
            if let Err(error) = self.set_highlight(index, shown) {
                self.status = error;
                break;
            }
        }
        self.refresh_panel(None)?;
        Ok(self.panel.take_update())
    }

    fn panel_event(&mut self, event: UiEvent) -> Result<Update, String> {
        self.panel.apply_event(&event);
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let (highlight_all, clear, list) = (nodes.highlight_all, nodes.clear, nodes.list);
        match event {
            UiEvent::Click(id) if id == highlight_all => self.set_all(true),
            UiEvent::Click(id) if id == clear => self.set_all(false),
            UiEvent::RowClick((id, row)) if id == list && (row as usize) < self.comments.len() => {
                let index = row as usize;
                self.status = match self.set_highlight(index, !self.highlighted[index]) {
                    Ok(()) => String::new(),
                    Err(error) => error,
                };
                self.refresh_panel(Some(RowRange {
                    start: row,
                    count: 1,
                }))?;
                Ok(self.panel.take_update())
            }
            _ => Ok(Update::Unchanged),
        }
    }

    /// `Done` in a decoration: remove it and tell the panel, which is another surface.
    fn decoration_event(&mut self, index: usize, event: UiEvent) -> Result<Update, String> {
        if event != UiEvent::Click(DECORATION_DONE) {
            return Ok(Update::Unchanged);
        }
        self.status = match self.set_highlight(index, false) {
            Ok(()) => String::new(),
            Err(error) => error,
        };
        let row = u32::try_from(index).unwrap_or(u32::MAX);
        self.refresh_panel(Some(RowRange {
            start: row,
            count: 1,
        }))?;
        if let Err(error) = self.panel.push(PANEL) {
            log(
                LogLevel::Warn,
                &format!("panel update refused: {}", error.message),
            );
        }
        Ok(Update::Unchanged)
    }

    fn mount_panel(&mut self) -> Result<Vec<Node>, String> {
        let mut tree = std::mem::take(&mut self.panel);
        let mut nodes = None;
        let summary = self.summary();
        let status = self.status_label();
        let count = u32::try_from(self.comments.len()).unwrap_or(u32::MAX);
        let mounted = tree.mount(|tree| {
            tree.flex(
                FlexProps::column()
                    .gap(8)
                    .padding(8)
                    .grow()
                    .align(Align::Stretch),
                |tree| {
                    let summary = tree.text(summary);
                    let mut highlight_all = 0;
                    let mut clear = 0;
                    tree.flex(FlexProps::row().gap(4), |tree| {
                        highlight_all = tree.button(
                            ButtonProps::new("Highlight all")
                                .icon(Icon::Eye)
                                .style(ButtonStyle::Filled)
                                .tooltip("Place a block above every comment"),
                        );
                        clear = tree.button(
                            ButtonProps::new("Clear")
                                .icon(Icon::EyeOff)
                                .style(ButtonStyle::Subtle)
                                .tooltip("Remove every block"),
                        );
                    });
                    let status = tree.text(status);
                    let list = tree.list(ListProps::new(count, ROW_HEIGHT));
                    nodes = Some(PanelNodes {
                        summary,
                        highlight_all,
                        clear,
                        status,
                        list,
                    });
                },
            );
        });
        self.panel = tree;
        let mounted = mounted?;
        self.nodes = nodes;
        Ok(mounted)
    }
}

impl Extension for Highlighter {
    fn new() -> Self {
        let comments = scan_mock_files();
        Self {
            highlighted: vec![false; comments.len()],
            comments,
            status: String::new(),
            panel: SurfaceTree::default(),
            nodes: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "TODO highlights",
            DockPosition::Left,
            Icon::TodoPending,
            320,
        )]
    }

    /// The panel, or a decoration the host remounts because its tree did not fit.
    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id == PANEL {
            return self.mount_panel();
        }
        match decoration_index(surface_id) {
            Some(index) if self.highlighted.get(index) == Some(&true) => {
                Ok(Self::decoration_tree(&self.comments[index]))
            }
            _ => Err(format!("unknown surface {surface_id:?}")),
        }
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if surface_id == PANEL {
            return self.panel_event(event);
        }
        match decoration_index(surface_id) {
            Some(index) if index < self.comments.len() => self.decoration_event(index, event),
            _ => Err(format!("unknown surface {surface_id:?}")),
        }
    }

    fn list_rows(
        &mut self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> Vec<ListRow> {
        if surface_id != PANEL || self.nodes.as_ref().map(|nodes| nodes.list) != Some(list_node) {
            return Vec::new();
        }
        let total = u32::try_from(self.comments.len()).unwrap_or(u32::MAX);
        list_rows(start, count, total, |index, tree| {
            let comment = &self.comments[index as usize];
            let icon = if self.highlighted[index as usize] {
                IconProps::new(Icon::TodoComplete).color(ColorToken::Success)
            } else {
                IconProps::new(Icon::TodoPending).color(comment.kind.color())
            };
            tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                tree.icon(icon);
                tree.text(
                    TextProps::new(format!("{}:{}", comment.path, comment.line + 1)).monospace(),
                );
                tree.text(TextProps::new(comment.text.clone()).color(ColorToken::TextMuted));
            });
        })
    }
}

// fyx:ui 0.1 by default, which is what every Fyx runs; `--features fyx-ui-0-2` builds the 0.2
// world and `--features fyx-ui-0-3` the staged 0.3 world from the same code.
#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(Highlighter, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(Highlighter, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(Highlighter);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_marker_comments_only() {
        let found = scan_mock_files()
            .into_iter()
            .map(|comment| (comment.path, comment.line, comment.kind, comment.text))
            .collect::<Vec<_>>();
        assert_eq!(
            found,
            [
                (
                    "src/main.rs",
                    1,
                    Kind::Todo,
                    "read the config path from the command line".to_string()
                ),
                (
                    "src/main.rs",
                    3,
                    Kind::Fixme,
                    "this panics when the file is missing".to_string()
                ),
                (
                    "src/render.rs",
                    1,
                    Kind::Hack,
                    "skip empty nodes until the layout pass handles them".to_string()
                ),
                (
                    "src/render.rs",
                    5,
                    Kind::Todo,
                    "batch draw calls".to_string()
                ),
                (
                    "web/app.ts",
                    1,
                    Kind::Todo,
                    "show a spinner while connecting".to_string()
                ),
                (
                    "docs/README.md",
                    2,
                    Kind::Fixme,
                    "screenshots are out of date".to_string()
                ),
            ]
        );
    }

    fn mounted() -> (Highlighter, PanelNodesIds) {
        let mut highlighter = Highlighter::new();
        highlighter.mount(PANEL).expect("mount");
        let nodes = highlighter.nodes.as_ref().expect("mounted");
        let ids = PanelNodesIds {
            highlight_all: nodes.highlight_all,
            clear: nodes.clear,
            list: nodes.list,
        };
        take_host_effects();
        (highlighter, ids)
    }

    struct PanelNodesIds {
        highlight_all: NodeId,
        clear: NodeId,
        list: NodeId,
    }

    #[test]
    fn highlight_all_places_one_decoration_per_comment() {
        let (mut highlighter, ids) = mounted();
        highlighter
            .handle_event(PANEL, UiEvent::Click(ids.highlight_all))
            .expect("click");
        let placed = take_host_effects()
            .into_iter()
            .filter_map(|effect| match effect {
                HostEffect::PlaceDecoration { id, location, .. } => Some((id, location.line)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(placed.len(), 6);
        assert_eq!(placed[0], ("todo-0".to_string(), 1));
        assert!(highlighter.highlighted.iter().all(|shown| *shown));
        assert_eq!(
            highlighter.mount("todo-0").expect("remount").len(),
            4,
            "a highlighted decoration can be remounted"
        );

        highlighter
            .handle_event("todo-2", UiEvent::Click(DECORATION_DONE))
            .expect("done");
        let effects = take_host_effects();
        assert!(
            matches!(
                &effects[..],
                [HostEffect::RemoveDecoration(id), HostEffect::Push { surface_id, .. }]
                    if id == "todo-2" && surface_id == PANEL
            ),
            "{effects:?}"
        );
        assert!(highlighter.mount("todo-2").is_err());

        highlighter
            .handle_event(PANEL, UiEvent::Click(ids.clear))
            .expect("clear");
        assert_eq!(
            take_host_effects()
                .iter()
                .filter(|effect| matches!(effect, HostEffect::RemoveDecoration(_)))
                .count(),
            5
        );
    }

    #[test]
    fn a_row_click_toggles_one_comment() {
        let (mut highlighter, ids) = mounted();
        highlighter
            .handle_event(PANEL, UiEvent::RowClick((ids.list, 4)))
            .expect("row click");
        assert!(matches!(
            &take_host_effects()[..],
            [HostEffect::PlaceDecoration { id, .. }] if id == "todo-4"
        ));
        highlighter
            .handle_event(PANEL, UiEvent::RowClick((ids.list, 4)))
            .expect("row click");
        assert!(matches!(
            &take_host_effects()[..],
            [HostEffect::RemoveDecoration(id)] if id == "todo-4"
        ));
    }
}
