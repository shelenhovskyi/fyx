//! Fyx demo 3: a gallery that shows every `fyx:ui` primitive, one section at a time, so the host
//! renderer can be checked primitive by primitive.
//!
//! Host tests rely on this behaviour, so keep it stable. The full contract, with the patches each
//! event returns, is in `tasks/progress/W4b.md`; `behaves_as_documented` and the other tests pin it.
//!
//! - Two surfaces: panel `"gallery"` (right dock, title "Gallery", icon `sparkle`, default size
//!   420) and status item `"gallery-status"` (right side, priority 0). Each surface has its own id
//!   space starting at 1.
//! - The panel never scrolls, so the gallery shows one section at a time; a row of navigation
//!   buttons (two flex rows) switches between them. The first `mount("gallery")` returns:
//!
//!   | id | parent | props |
//!   |---|---|---|
//!   | 1 | none | column flex, gap 8, padding 8, grow, align stretch |
//!   | 2 | 1 | text `fyx:ui gallery`, bold |
//!   | 3 | 1 | row flex, gap 4 |
//!   | 4–8 | 3 | buttons `Layout`, `Text`, `Buttons`, `Input`, `List` |
//!   | 9 | 1 | row flex, gap 4 |
//!   | 10–13 | 9 | buttons `Table`, `Icons`, `Markdown`, `Diff` |
//!   | 14 | 1 | column flex, gap 8, grow, align stretch: the section slot |
//!   | 15 | 14 | the shown section's root (column flex, gap 8, grow, align stretch) |
//!
//!   The shown section's navigation button is `filled`, the others `subtle`. The first section is
//!   Layout; its subtree takes ids 15 to 68.
//! - `click(<navigation button>)` of another section returns `patches`: `set-props` on the old
//!   section's button (now subtle), `set-props` on the new one (now filled), `remove(<section
//!   root>)`, then one `insert` per node of the new section, parents first, the root appended
//!   under 14. New sections always get new ids. Clicking the shown section's button returns
//!   `unchanged`.
//! - Every other event returns `patches` with only `set-props`, or `unchanged`; never `replace`.
//!   Events for nodes that are not shown return `unchanged`. An unknown surface id is an error.

use ui_ext_sdk::*;

const GALLERY_SURFACE: &str = "gallery";
const STATUS_SURFACE: &str = "gallery-status";

const LIST_ROW_COUNT: u32 = 1_000;
const LIST_ROW_HEIGHT: u16 = 24;
const TABLE_COLUMNS: [&str; 3] = ["Name", "Kind", "Lines"];
const TABLE_ROW_COUNT: u32 = 100;

const INPUT_PLACEHOLDER: &str = "Type and press Enter";

const ALL_TOKENS: [ColorToken; 9] = [
    ColorToken::Text,
    ColorToken::TextMuted,
    ColorToken::TextAccent,
    ColorToken::Error,
    ColorToken::Warning,
    ColorToken::Success,
    ColorToken::Border,
    ColorToken::Surface,
    ColorToken::ElevatedSurface,
];

const ALL_ALIGNS: [Align; 4] = [Align::Start, Align::Center, Align::End, Align::Stretch];

/// Frozen 0.2 icons with their WIT names, laid out four per row. `blocks` is left out on purpose:
/// it is the host's fallback, so it would not tell a drawn icon from a missing one.
const KNOWN_ICONS: [(Icon, &str, Option<ColorToken>); 12] = [
    (Icon::Check, "check", Some(ColorToken::Success)),
    (Icon::Close, "close", Some(ColorToken::Error)),
    (Icon::Warning, "warning", Some(ColorToken::Warning)),
    (Icon::Info, "info", Some(ColorToken::TextAccent)),
    (Icon::Plus, "plus", None),
    (Icon::Dash, "dash", None),
    (Icon::Folder, "folder", None),
    (Icon::File, "file", None),
    (Icon::Settings, "settings", None),
    (Icon::Search, "search", None),
    (Icon::Trash, "trash", None),
    (Icon::Pencil, "pencil", None),
];
const ICONS_PER_ROW: usize = 4;

const MARKDOWN_SOURCE: &str = "\
# Markdown

Rendered by the host's own markdown component. Paragraphs with **bold**, *italic*, \
`inline code` and a [link](https://example.com).

- A bullet list
- with two items

1. A numbered list
2. with two items

> A block quote.

```rust
fn main() {
    println!(\"a fenced code block\");
}
```
";

const DIFF_OLD_RUST: &str = "\
fn greet(name: &str) {
    println!(\"Hello, {}\", name);
}

fn main() {
    greet(\"world\");
}
";

const DIFF_NEW_RUST: &str = "\
fn greet(name: &str, punctuation: char) {
    println!(\"Hello, {name}{punctuation}\");
}

fn main() {
    greet(\"world\", '!');
}
";

const DIFF_OLD_TEXT: &str = "first line\nsecond line\nthird line\n";
const DIFF_NEW_TEXT: &str = "first line\nsecond line, changed\nthird line\nfourth line\n";

struct ButtonDemo {
    /// How the "Last clicked" label names the button; differs from `label` for the icon-only one.
    name: &'static str,
    label: &'static str,
    icon: Option<Icon>,
    style: ButtonStyle,
    disabled: bool,
}

impl ButtonDemo {
    const fn new(name: &'static str, style: ButtonStyle) -> Self {
        Self {
            name,
            label: name,
            icon: None,
            style,
            disabled: false,
        }
    }

    const fn disabled(name: &'static str, style: ButtonStyle) -> Self {
        Self {
            disabled: true,
            ..Self::new(name, style)
        }
    }

    const fn with_icon(
        name: &'static str,
        label: &'static str,
        icon: Icon,
        style: ButtonStyle,
    ) -> Self {
        Self {
            name,
            label,
            icon: Some(icon),
            style,
            disabled: false,
        }
    }

    fn props(&self) -> ButtonProps {
        let props = ButtonProps::new(self.label)
            .style(self.style)
            .disabled(self.disabled);
        match self.icon {
            Some(icon) => props.icon(icon),
            None => props,
        }
    }
}

const BUTTON_GROUPS: [(&str, [ButtonDemo; 4]); 3] = [
    (
        "Styles",
        [
            ButtonDemo::new("Default", ButtonStyle::Default),
            ButtonDemo::new("Filled", ButtonStyle::Filled),
            ButtonDemo::new("Subtle", ButtonStyle::Subtle),
            ButtonDemo::new("Danger", ButtonStyle::Danger),
        ],
    ),
    (
        "Disabled",
        [
            ButtonDemo::disabled("Default", ButtonStyle::Default),
            ButtonDemo::disabled("Filled", ButtonStyle::Filled),
            ButtonDemo::disabled("Subtle", ButtonStyle::Subtle),
            ButtonDemo::disabled("Danger", ButtonStyle::Danger),
        ],
    ),
    (
        "With icon",
        [
            ButtonDemo::with_icon("Add", "Add", Icon::Plus, ButtonStyle::Filled),
            ButtonDemo::with_icon("Search", "Search", Icon::Search, ButtonStyle::Default),
            ButtonDemo::with_icon("Delete", "Delete", Icon::Trash, ButtonStyle::Danger),
            ButtonDemo::with_icon(
                "Icon only (settings)",
                "",
                Icon::Settings,
                ButtonStyle::Subtle,
            ),
        ],
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Layout,
    Text,
    Buttons,
    Input,
    List,
    Table,
    Icons,
    Markdown,
    Diff,
}

impl Section {
    /// Navigation order; the first `FIRST_NAV_ROW` go in the first row.
    const ALL: [Section; 9] = [
        Section::Layout,
        Section::Text,
        Section::Buttons,
        Section::Input,
        Section::List,
        Section::Table,
        Section::Icons,
        Section::Markdown,
        Section::Diff,
    ];
    const FIRST_NAV_ROW: usize = 5;

    fn label(self) -> &'static str {
        match self {
            Section::Layout => "Layout",
            Section::Text => "Text",
            Section::Buttons => "Buttons",
            Section::Input => "Input",
            Section::List => "List",
            Section::Table => "Table",
            Section::Icons => "Icons",
            Section::Markdown => "Markdown",
            Section::Diff => "Diff",
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Section::Layout => "Flex layouts",
            Section::Text => "Text",
            Section::Buttons => "Buttons",
            Section::Input => "Input",
            Section::List => "Virtualized list",
            Section::Table => "Table",
            Section::Icons => "Icons",
            Section::Markdown => "Markdown",
            Section::Diff => "Diff view",
        }
    }

    fn nav_button(self, shown: Section) -> ButtonProps {
        let style = if self == shown {
            ButtonStyle::Filled
        } else {
            ButtonStyle::Subtle
        };
        ButtonProps::new(self.label()).style(style)
    }
}

/// Everything the trees are built from; kept apart from the trees so builders can read it while
/// a tree is borrowed mutably.
#[derive(Default)]
struct GalleryState {
    section: Option<Section>,
    last_clicked: Option<String>,
    input_value: String,
    submitted: Option<String>,
    selected_row: Option<u32>,
}

impl GalleryState {
    fn section(&self) -> Section {
        self.section.unwrap_or(Section::Layout)
    }

    fn last_clicked_label(&self) -> TextProps {
        match &self.last_clicked {
            Some(name) => TextProps::new(format!("Last clicked: {name}")),
            None => TextProps::new("Last clicked: none").color(ColorToken::TextMuted),
        }
    }

    fn input(&self) -> InputProps {
        InputProps::new(self.input_value.clone()).placeholder(INPUT_PLACEHOLDER)
    }

    fn submitted_label(&self) -> TextProps {
        match self.submitted.as_deref() {
            Some("") => TextProps::new("Submitted: (empty)"),
            Some(value) => TextProps::new(format!("Submitted: {value}")),
            None => TextProps::new("Submitted: nothing yet").color(ColorToken::TextMuted),
        }
    }

    fn selected_label(&self) -> TextProps {
        match self.selected_row {
            Some(index) => TextProps::new(format!("Selected: Row {}", index + 1)),
            None => TextProps::new("Selected: none").color(ColorToken::TextMuted),
        }
    }

    fn status_label(&self) -> TextProps {
        match self.selected_row {
            Some(index) => TextProps::new(format!("Gallery: Row {}", index + 1)),
            None => TextProps::new("Gallery: none").color(ColorToken::TextMuted),
        }
    }
}

/// Nodes of the shown section that events or `list-rows` refer to.
enum SectionNodes {
    Static,
    Buttons {
        buttons: Vec<(NodeId, &'static ButtonDemo)>,
        last_clicked: NodeId,
    },
    Input {
        input: NodeId,
        submitted: NodeId,
    },
    List {
        list: NodeId,
        selected: NodeId,
    },
    Table {
        table: NodeId,
    },
}

struct GalleryNodes {
    /// In `Section::ALL` order.
    nav: Vec<(Section, NodeId)>,
    section_root: NodeId,
    section: SectionNodes,
}

#[derive(Default)]
struct Gallery {
    state: GalleryState,
    gallery: SurfaceTree,
    gallery_nodes: Option<GalleryNodes>,
    status: SurfaceTree,
    status_label: Option<NodeId>,
}

fn token_name(token: ColorToken) -> &'static str {
    match token {
        ColorToken::Text => "text",
        ColorToken::TextMuted => "text-muted",
        ColorToken::TextAccent => "text-accent",
        ColorToken::Error => "error",
        ColorToken::Warning => "warning",
        ColorToken::Success => "success",
        ColorToken::Border => "border",
        ColorToken::Surface => "surface",
        ColorToken::ElevatedSurface => "elevated-surface",
    }
}

fn align_name(align: Align) -> &'static str {
    match align {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
        Align::Stretch => "stretch",
    }
}

fn stretched_column() -> FlexProps {
    FlexProps::column().gap(8).grow().align(Align::Stretch)
}

fn caption(tree: &mut TreeBuilder, content: &str) -> NodeId {
    tree.text(TextProps::new(content).color(ColorToken::TextMuted))
}

/// A padded box with a background, so the extent of a flex child is visible.
fn swatch(tree: &mut TreeBuilder, content: &str) -> NodeId {
    tree.flex(
        FlexProps::row()
            .padding(4)
            .background(ColorToken::ElevatedSurface),
        |tree| {
            tree.text(content);
        },
    )
}

fn build_layout(tree: &mut TreeBuilder) {
    caption(tree, "Row: gap 4, padding 4, background surface");
    tree.flex(
        FlexProps::row()
            .gap(4)
            .padding(4)
            .background(ColorToken::Surface),
        |tree| {
            for content in ["A", "B", "C"] {
                swatch(tree, content);
            }
        },
    );

    caption(
        tree,
        "Column: gap 8, padding 8, background elevated-surface",
    );
    tree.flex(
        FlexProps::column()
            .gap(8)
            .padding(8)
            .background(ColorToken::ElevatedSurface),
        |tree| {
            tree.text("First");
            tree.text("Second");
        },
    );

    caption(tree, "Align: cross axis of a column");
    for align in ALL_ALIGNS {
        tree.flex(
            FlexProps::column()
                .padding(4)
                .align(align)
                .background(ColorToken::Surface),
            |tree| {
                swatch(tree, &format!("align {}", align_name(align)));
            },
        );
    }

    caption(tree, "Grow: the middle child takes the free space");
    tree.flex(FlexProps::row().gap(4), |tree| {
        swatch(tree, "fixed");
        tree.flex(
            FlexProps::row()
                .padding(4)
                .grow()
                .background(ColorToken::Border),
            |tree| {
                tree.text("grow");
            },
        );
        swatch(tree, "fixed");
    });

    caption(tree, "Background tokens");
    for token in ALL_TOKENS {
        tree.flex(FlexProps::row().padding(4).background(token), |tree| {
            tree.text(token_name(token));
        });
    }
}

/// `surface` and `elevated-surface` are background colours: as text colour they are invisible on
/// the panel by design, so the Text section shows them as a background behind a default-colour
/// label instead.
fn is_background_token(token: ColorToken) -> bool {
    matches!(token, ColorToken::Surface | ColorToken::ElevatedSurface)
}

fn build_text(tree: &mut TreeBuilder) {
    caption(tree, "Colour tokens");
    tree.text("no colour (the default)");
    for token in ALL_TOKENS {
        if is_background_token(token) {
            tree.flex(FlexProps::row().padding(4).background(token), |tree| {
                tree.text(format!("{} (background)", token_name(token)));
            });
        } else {
            tree.text(TextProps::new(token_name(token)).color(token));
        }
    }
    caption(tree, "Weight and font");
    tree.text(TextProps::new("bold").bold());
    tree.text(TextProps::new("monospace").monospace());
    tree.text(TextProps::new("bold monospace").bold().monospace());
}

fn build_buttons(state: &GalleryState, tree: &mut TreeBuilder) -> SectionNodes {
    let mut buttons = Vec::new();
    for (group, demos) in &BUTTON_GROUPS {
        caption(tree, group);
        tree.flex(FlexProps::row().gap(4).align(Align::Center), |tree| {
            for demo in demos {
                buttons.push((tree.button(demo.props()), demo));
            }
        });
    }
    let last_clicked = tree.text(state.last_clicked_label());
    SectionNodes::Buttons {
        buttons,
        last_clicked,
    }
}

fn build_input(state: &GalleryState, tree: &mut TreeBuilder) -> SectionNodes {
    caption(
        tree,
        "Typing is not echoed back; Enter submits and the value appears below",
    );
    let input = tree.input(state.input());
    let submitted = tree.text(state.submitted_label());
    SectionNodes::Input { input, submitted }
}

fn build_list(state: &GalleryState, tree: &mut TreeBuilder) -> SectionNodes {
    caption(tree, "1,000 rows of 24 px from list-rows; click a row");
    let selected = tree.text(state.selected_label());
    let list = tree.list(ListProps::new(LIST_ROW_COUNT, LIST_ROW_HEIGHT));
    SectionNodes::List { list, selected }
}

fn build_table(tree: &mut TreeBuilder) -> SectionNodes {
    caption(tree, "Three columns, 100 rows from list-rows");
    let table = tree.table(TableProps::new(TABLE_COLUMNS, TABLE_ROW_COUNT));
    SectionNodes::Table { table }
}

fn icon_cell(tree: &mut TreeBuilder, icon: Icon, name: &str, color: Option<ColorToken>) {
    tree.flex(
        FlexProps::column().gap(2).padding(4).align(Align::Center),
        |tree| {
            let icon = IconProps::new(icon);
            tree.icon(match color {
                Some(color) => icon.color(color),
                None => icon,
            });
            caption(tree, name);
        },
    );
}

fn build_icons(tree: &mut TreeBuilder) {
    caption(tree, "Frozen icons (fyx:ui 0.2 names)");
    for icons in KNOWN_ICONS.chunks(ICONS_PER_ROW) {
        tree.flex(FlexProps::row().gap(12), |tree| {
            for &(icon, name, color) in icons {
                icon_cell(tree, icon, name, color);
            }
        });
    }
    caption(tree, "Spinner: animated by a 0.2 host, load_circle on 0.1");
    tree.flex(FlexProps::row().gap(12), |tree| {
        icon_cell(tree, Icon::Spinner, "spinner", None);
    });
}

fn build_diff(tree: &mut TreeBuilder) {
    caption(tree, "Rust");
    tree.diff_view(DiffViewProps::new(DIFF_OLD_RUST, DIFF_NEW_RUST).language("rust"));
    caption(tree, "Plain text, no language");
    tree.diff_view(DiffViewProps::new(DIFF_OLD_TEXT, DIFF_NEW_TEXT));
}

/// Builds the shown section under the current parent and returns its root.
fn build_section(state: &GalleryState, tree: &mut TreeBuilder) -> (NodeId, SectionNodes) {
    let section = state.section();
    let mut nodes = SectionNodes::Static;
    let root = tree.flex(stretched_column(), |tree| {
        tree.text(TextProps::new(section.heading()).bold());
        nodes = match section {
            Section::Layout => {
                build_layout(tree);
                SectionNodes::Static
            }
            Section::Text => {
                build_text(tree);
                SectionNodes::Static
            }
            Section::Buttons => build_buttons(state, tree),
            Section::Input => build_input(state, tree),
            Section::List => build_list(state, tree),
            Section::Table => build_table(tree),
            Section::Icons => {
                build_icons(tree);
                SectionNodes::Static
            }
            Section::Markdown => {
                tree.markdown(MARKDOWN_SOURCE);
                SectionNodes::Static
            }
            Section::Diff => {
                build_diff(tree);
                SectionNodes::Static
            }
        };
    });
    (root, nodes)
}

fn build_list_row(index: u32, tree: &mut TreeBuilder) {
    tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
        tree.icon(Icon::File);
        tree.text(format!("Row {}", index + 1));
        tree.text(
            TextProps::new(format!("index {index}"))
                .color(ColorToken::TextMuted)
                .monospace(),
        );
    });
}

/// One flex row whose children are the cells, as the host's table expects.
fn build_table_row(index: u32, tree: &mut TreeBuilder) {
    let kind = match index % 3 {
        0 => "source",
        1 => "test",
        _ => "doc",
    };
    let lines = (index * 37) % 500 + 10;
    tree.flex(FlexProps::row(), |tree| {
        tree.text(format!("file_{:03}.rs", index + 1));
        tree.text(TextProps::new(kind).color(ColorToken::TextMuted));
        tree.text(TextProps::new(lines.to_string()).monospace());
    });
}

fn unknown_surface(surface_id: &str) -> String {
    format!("unknown surface {surface_id:?}")
}

impl Gallery {
    fn mount_gallery(&mut self) -> Result<Vec<Node>, String> {
        let shown = self.state.section();
        let state = &self.state;
        let mut gallery_nodes = None;
        let mounted = self.gallery.mount(|tree| {
            tree.flex(
                FlexProps::column()
                    .gap(8)
                    .padding(8)
                    .grow()
                    .align(Align::Stretch),
                |tree| {
                    tree.text(TextProps::new("fyx:ui gallery").bold());
                    let (first_row, second_row) = Section::ALL.split_at(Section::FIRST_NAV_ROW);
                    let mut nav = Vec::with_capacity(Section::ALL.len());
                    for sections in [first_row, second_row] {
                        tree.flex(FlexProps::row().gap(4), |tree| {
                            for &section in sections {
                                nav.push((section, tree.button(section.nav_button(shown))));
                            }
                        });
                    }
                    tree.flex(stretched_column(), |tree| {
                        let (section_root, section) = build_section(state, tree);
                        gallery_nodes = Some(GalleryNodes {
                            nav,
                            section_root,
                            section,
                        });
                    });
                },
            );
        })?;
        self.gallery_nodes = gallery_nodes;
        Ok(mounted)
    }

    fn mount_status(&mut self) -> Result<Vec<Node>, String> {
        let state = &self.state;
        let mut status_label = None;
        let mounted = self.status.mount(|tree| {
            tree.flex(FlexProps::row().gap(4).align(Align::Center), |tree| {
                tree.icon(IconProps::new(Icon::ListTree).color(ColorToken::TextMuted));
                status_label = Some(tree.text(state.status_label()));
            });
        })?;
        self.status_label = status_label;
        Ok(mounted)
    }

    fn show_section(&mut self, section: Section) -> Result<(), String> {
        let Some(nodes) = &self.gallery_nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let section_root = nodes.section_root;
        let shown = self.state.section();
        let nav_button = |wanted: Section| {
            nodes
                .nav
                .iter()
                .find(|(nav_section, _)| *nav_section == wanted)
                .map(|&(_, button)| button)
        };
        let buttons = [(shown, nav_button(shown)), (section, nav_button(section))];
        self.state.section = Some(section);
        for (nav_section, button) in buttons {
            if let Some(button) = button {
                self.gallery
                    .set_props(button, nav_section.nav_button(section))?;
            }
        }
        let state = &self.state;
        let mut built = None;
        self.gallery.rebuild(section_root, |tree| {
            built = Some(build_section(state, tree));
        })?;
        if let (Some(nodes), Some((section_root, section))) = (&mut self.gallery_nodes, built) {
            nodes.section_root = section_root;
            nodes.section = section;
        }
        Ok(())
    }

    fn select_row(&mut self, selected: NodeId, index: u32) -> Result<(), String> {
        self.state.selected_row = Some(index);
        self.gallery
            .set_props(selected, self.state.selected_label())?;
        // The status item is another surface, so it can only change through a push.
        if let Some(status_label) = self.status_label {
            self.status
                .set_props(status_label, self.state.status_label())?;
            // On a refused push the next one resends the status item whole.
            if let Err(error) = self.status.push(STATUS_SURFACE) {
                log(
                    LogLevel::Warn,
                    &format!("status item update refused: {}", error.message),
                );
            }
        }
        Ok(())
    }

    fn handle_gallery_event(&mut self, event: UiEvent) -> Result<Update, String> {
        self.gallery.apply_event(&event);
        let Some(nodes) = &self.gallery_nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let action = match (event, &nodes.section) {
            (UiEvent::Click(id), section_nodes) => {
                if let Some(&(section, _)) = nodes.nav.iter().find(|(_, button)| *button == id) {
                    Action::ShowSection(section)
                } else if let SectionNodes::Buttons {
                    buttons,
                    last_clicked,
                } = section_nodes
                    && let Some(&(_, demo)) = buttons.iter().find(|(button, _)| *button == id)
                {
                    Action::ButtonClicked {
                        demo,
                        last_clicked: *last_clicked,
                    }
                } else {
                    Action::Nothing
                }
            }
            (UiEvent::InputChanged((id, value)), &SectionNodes::Input { input, .. })
                if id == input =>
            {
                Action::Typed { value }
            }
            (UiEvent::InputSubmitted((id, value)), &SectionNodes::Input { input, submitted })
                if id == input =>
            {
                Action::Submitted { submitted, value }
            }
            (UiEvent::RowClick((list_node, index)), &SectionNodes::List { list, selected })
                if list_node == list && index < LIST_ROW_COUNT =>
            {
                Action::SelectRow { selected, index }
            }
            _ => Action::Nothing,
        };

        match action {
            Action::ShowSection(section) => {
                if section != self.state.section() {
                    self.show_section(section)?;
                }
            }
            Action::ButtonClicked { demo, last_clicked } => {
                self.state.last_clicked = Some(if demo.disabled {
                    format!(
                        "{} (disabled: the host should not have sent this)",
                        demo.name
                    )
                } else {
                    demo.name.to_string()
                });
                self.gallery
                    .set_props(last_clicked, self.state.last_clicked_label())?;
            }
            // `apply_event` already put the typed value into the mirror, as the host did.
            Action::Typed { value } => self.state.input_value = value,
            Action::Submitted { submitted, value } => {
                self.state.input_value = value.clone();
                self.state.submitted = Some(value);
                self.gallery
                    .set_props(submitted, self.state.submitted_label())?;
            }
            Action::SelectRow { selected, index } => self.select_row(selected, index)?,
            Action::Nothing => {}
        }
        Ok(self.gallery.take_update())
    }
}

/// What a gallery event asks for, decided while the shown nodes are borrowed.
enum Action {
    ShowSection(Section),
    ButtonClicked {
        demo: &'static ButtonDemo,
        last_clicked: NodeId,
    },
    Typed {
        value: String,
    },
    Submitted {
        submitted: NodeId,
        value: String,
    },
    SelectRow {
        selected: NodeId,
        index: u32,
    },
    Nothing,
}

impl Extension for Gallery {
    fn new() -> Self {
        Self::default()
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![
            Surface::panel(
                GALLERY_SURFACE,
                "Gallery",
                DockPosition::Right,
                Icon::Sparkle,
                420,
            ),
            Surface::status_item(STATUS_SURFACE, "Gallery selection", StatusBarSide::Right, 0),
        ]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        match surface_id {
            GALLERY_SURFACE => self.mount_gallery(),
            STATUS_SURFACE => self.mount_status(),
            _ => Err(unknown_surface(surface_id)),
        }
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        match surface_id {
            GALLERY_SURFACE => self.handle_gallery_event(event),
            STATUS_SURFACE => {
                if self.status_label.is_none() {
                    return Err(TreeError::NotMounted.into());
                }
                Ok(Update::Unchanged)
            }
            _ => Err(unknown_surface(surface_id)),
        }
    }

    fn list_rows(
        &mut self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> Vec<ListRow> {
        let section = (surface_id == GALLERY_SURFACE)
            .then_some(self.gallery_nodes.as_ref())
            .flatten()
            .map(|nodes| &nodes.section);
        match section {
            Some(&SectionNodes::List { list, .. }) if list == list_node => {
                list_rows(start, count, LIST_ROW_COUNT, build_list_row)
            }
            Some(&SectionNodes::Table { table }) if table == list_node => {
                list_rows(start, count, TABLE_ROW_COUNT, build_table_row)
            }
            _ => {
                log(
                    LogLevel::Warn,
                    &format!("list-rows for unknown list {list_node} on {surface_id:?}"),
                );
                Vec::new()
            }
        }
    }
}

// fyx:ui 0.1 by default, which is what Fyx runs today; `--features fyx-ui-0-2` builds the 0.2
// world from the same code.
#[cfg(not(feature = "fyx-ui-0-2"))]
register_extension!(Gallery, v0_1);
#[cfg(feature = "fyx-ui-0-2")]
register_extension!(Gallery);

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn to_host(nodes: &[Node]) -> Vec<ui_ext::Node> {
        nodes.iter().map(Into::into).collect()
    }

    /// The gallery as the host sees it: every update is applied to a real `ui_ext::Tree` with the
    /// host's own rules and must leave it equal to the extension's mirror.
    struct Harness {
        gallery: Gallery,
        host: ui_ext::Tree,
    }

    impl Harness {
        fn mounted() -> Self {
            let mut gallery = Gallery::new();
            let nodes = gallery.mount(GALLERY_SURFACE).expect("mount");
            let host = ui_ext::Tree::from_nodes(to_host(&nodes), TreeLimits::default())
                .expect("the host accepts the mount");
            Self { gallery, host }
        }

        fn event(&mut self, event: UiEvent) -> Update {
            // Like the host (W3 integrator decision 6): typed text is written into the host's own
            // tree before the event is sent, and the extension never echoes it.
            if let UiEvent::InputChanged((id, value)) | UiEvent::InputSubmitted((id, value)) =
                &event
                && let Some(ui_ext::Props::Input(input)) =
                    self.host.get(*id).map(|node| node.props().clone())
            {
                let typed = ui_ext::Props::Input(ui_ext::InputProps {
                    value: value.clone(),
                    ..input
                });
                self.host
                    .apply_patches(vec![ui_ext::Patch::SetProps(*id, typed)])
                    .expect("the host records typed text");
            }
            let update = self
                .gallery
                .handle_event(GALLERY_SURFACE, event)
                .expect("handle-event");
            assert!(
                !matches!(update, Update::Replace(_)),
                "events answer with patches, never replace"
            );
            self.host
                .apply_update((&update).into())
                .expect("the host accepts the update");
            assert_eq!(
                self.host.to_nodes(),
                to_host(&self.gallery.gallery.to_nodes())
            );
            update
        }

        fn click(&mut self, id: NodeId) -> Update {
            self.event(UiEvent::Click(id))
        }

        fn nodes(&self) -> &GalleryNodes {
            self.gallery.gallery_nodes.as_ref().expect("mounted")
        }

        fn nav(&self, section: Section) -> NodeId {
            self.nodes()
                .nav
                .iter()
                .find(|(nav_section, _)| *nav_section == section)
                .map(|&(_, button)| button)
                .expect("every section has a navigation button")
        }

        fn show(&mut self, section: Section) -> Update {
            self.click(self.nav(section))
        }

        fn props(&self, id: NodeId) -> Props {
            self.gallery
                .gallery
                .props(id)
                .cloned()
                .expect("node is shown")
        }

        /// Kind names of the shown section's nodes.
        fn section_kinds(&self) -> BTreeSet<&'static str> {
            let root = self.nodes().section_root;
            let mut kinds = BTreeSet::new();
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                if let Some(node) = self.host.get(id) {
                    kinds.insert(node.props().kind_name());
                    stack.extend(node.children());
                }
            }
            kinds
        }
    }

    /// `set N`, `remove N` and `insert N under P` for each patch, in order.
    fn describe(update: &Update) -> Vec<String> {
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        patches
            .iter()
            .map(|patch| match patch {
                Patch::SetProps((id, _)) => format!("set {id}"),
                Patch::InvalidateRows(op) => format!("invalidate {}", op.target),
                Patch::Remove(id) => format!("remove {id}"),
                Patch::Insert(InsertOp { node, before }) => {
                    assert_eq!(*before, None, "the gallery only appends");
                    match node.parent {
                        Some(parent) => format!("insert {} under {parent}", node.id),
                        None => format!("insert {} as a root", node.id),
                    }
                }
            })
            .collect()
    }

    fn set_props(update: &Update) -> Vec<(NodeId, Props)> {
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        patches
            .iter()
            .map(|patch| match patch {
                Patch::SetProps((id, props)) => (*id, props.clone()),
                patch => panic!("expected only set-props, got {patch:?}"),
            })
            .collect()
    }

    fn row_texts(rows: &[ListRow]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| {
                row.nodes
                    .iter()
                    .filter_map(|node| match &node.props {
                        Props::Text(text) => Some(text.content.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn behaves_as_documented() {
        let mut gallery = Gallery::new();
        let surfaces = gallery.surfaces();
        assert_eq!(
            surfaces,
            [
                Surface::panel(
                    "gallery",
                    "Gallery",
                    DockPosition::Right,
                    Icon::Sparkle,
                    420
                ),
                Surface::status_item(
                    "gallery-status",
                    "Gallery selection",
                    StatusBarSide::Right,
                    0
                ),
            ]
        );

        let nodes = gallery.mount("gallery").expect("mount");
        let frame = nodes
            .iter()
            .take(15)
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            frame,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(3)),
                (5, Some(3)),
                (6, Some(3)),
                (7, Some(3)),
                (8, Some(3)),
                (9, Some(1)),
                (10, Some(9)),
                (11, Some(9)),
                (12, Some(9)),
                (13, Some(9)),
                (14, Some(1)),
                (15, Some(14)),
            ]
        );
        assert_eq!(nodes.len(), 68, "the Layout section takes ids 15 to 68");
        assert_eq!(nodes.last().map(|node| node.id), Some(68));
        assert_eq!(
            nodes[1].props,
            TextProps::new("fyx:ui gallery").bold().into()
        );
        let nav_labels = nodes[3..8]
            .iter()
            .chain(&nodes[9..13])
            .map(|node| match &node.props {
                Props::Button(button) => (button.label.as_str(), button.style),
                props => panic!("expected a button, got {props:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            nav_labels,
            [
                ("Layout", ButtonStyle::Filled),
                ("Text", ButtonStyle::Subtle),
                ("Buttons", ButtonStyle::Subtle),
                ("Input", ButtonStyle::Subtle),
                ("List", ButtonStyle::Subtle),
                ("Table", ButtonStyle::Subtle),
                ("Icons", ButtonStyle::Subtle),
                ("Markdown", ButtonStyle::Subtle),
                ("Diff", ButtonStyle::Subtle),
            ]
        );
        assert_eq!(nodes[14].props, stretched_column().into());
        assert_eq!(
            nodes[15].props,
            TextProps::new("Flex layouts").bold().into()
        );

        let host = ui_ext::Tree::from_nodes(to_host(&nodes), TreeLimits::default())
            .expect("the host accepts the mount");
        let mut harness = Harness { gallery, host };

        assert_eq!(
            harness.click(4),
            Update::Unchanged,
            "Layout is already shown"
        );
        assert_eq!(
            describe(&harness.click(8)),
            [
                "set 4",
                "set 8",
                "remove 15",
                "insert 69 under 14",
                "insert 70 under 69",
                "insert 71 under 69",
                "insert 72 under 69",
                "insert 73 under 69",
            ],
            "List: root 69, heading 70, caption 71, selected label 72, list 73"
        );
        assert_eq!(
            harness.props(4),
            Section::Layout.nav_button(Section::List).into()
        );
        assert_eq!(
            harness.props(73),
            ListProps::new(1_000, 24).into(),
            "the list is node 73"
        );

        let rows = harness.gallery.list_rows("gallery", 73, 998, 10);
        assert_eq!(
            rows.iter().map(|row| row.index).collect::<Vec<_>>(),
            [998, 999]
        );
        assert_eq!(
            row_texts(&rows),
            [["Row 999", "index 998"], ["Row 1000", "index 999"]]
        );

        assert_eq!(
            set_props(&harness.event(UiEvent::RowClick((73, 41)))),
            [(72, TextProps::new("Selected: Row 42").into())]
        );
        assert_eq!(
            harness.event(UiEvent::RowClick((73, 1_000))),
            Update::Unchanged
        );
        assert_eq!(harness.click(99), Update::Unchanged);
        assert!(
            harness
                .gallery
                .handle_event("other", UiEvent::Click(4))
                .is_err()
        );
        assert!(harness.gallery.mount("other").is_err());
    }

    #[test]
    fn every_section_shows_and_patches_cleanly() {
        let mut harness = Harness::mounted();
        let mut kinds = harness.section_kinds();
        // Every section from every other one, so each switch removes a different subtree.
        for &from in &Section::ALL {
            for &to in &Section::ALL {
                if from == to {
                    continue;
                }
                harness.show(from);
                let old_root = harness.nodes().section_root;
                let patches = describe(&harness.show(to));
                assert_eq!(
                    patches[..3],
                    [
                        format!("set {}", harness.nav(from)),
                        format!("set {}", harness.nav(to)),
                        format!("remove {old_root}"),
                    ]
                );
                let new_root = harness.nodes().section_root;
                assert_eq!(patches[3], format!("insert {new_root} under 14"));
                assert_eq!(harness.host.children(Some(14)), [new_root]);
                assert_eq!(harness.gallery.state.section(), to);
                kinds.extend(harness.section_kinds());
            }
        }
        assert_eq!(
            kinds,
            BTreeSet::from([
                "button",
                "diff-view",
                "flex",
                "icon",
                "input",
                "list",
                "markdown",
                "table",
                "text",
            ])
        );
    }

    #[test]
    fn sections_cover_their_primitives() {
        let mut harness = Harness::mounted();
        let expected: [(Section, &[&str]); 9] = [
            (Section::Layout, &["flex", "text"]),
            (Section::Text, &["flex", "text"]),
            (Section::Buttons, &["button", "flex", "text"]),
            (Section::Input, &["flex", "input", "text"]),
            (Section::List, &["flex", "list", "text"]),
            (Section::Table, &["flex", "table", "text"]),
            (Section::Icons, &["flex", "icon", "text"]),
            (Section::Markdown, &["flex", "markdown", "text"]),
            (Section::Diff, &["diff-view", "flex", "text"]),
        ];
        for (section, kinds) in expected {
            harness.show(section);
            assert_eq!(
                harness.section_kinds(),
                kinds.iter().copied().collect(),
                "{section:?}"
            );
        }

        harness.show(Section::Text);
        let nodes = harness.host.to_nodes();
        let text_colors = nodes
            .iter()
            .filter_map(|node| match &node.props {
                ui_ext::Props::Text(text) => text.color.map(|color| format!("{color:?}")),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let foreground_tokens = ALL_TOKENS
            .into_iter()
            .filter(|&token| !is_background_token(token))
            .map(|token| format!("{:?}", ui_ext::ColorToken::from(token)))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            text_colors, foreground_tokens,
            "every text token as text colour"
        );
        let text_backgrounds = nodes
            .iter()
            .filter_map(|node| match &node.props {
                ui_ext::Props::Flex(flex) => flex.background.map(|color| format!("{color:?}")),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            text_backgrounds,
            BTreeSet::from(["ElevatedSurface".to_string(), "Surface".to_string()]),
            "surface tokens as backgrounds"
        );
        for label in ["surface (background)", "elevated-surface (background)"] {
            assert!(
                nodes.iter().any(|node| node.props
                    == ui_ext::Props::Text(ui_ext::TextProps {
                        content: label.into(),
                        ..Default::default()
                    })),
                "{label} in the default colour"
            );
        }

        harness.show(Section::Layout);
        let backgrounds = harness
            .host
            .to_nodes()
            .into_iter()
            .filter_map(|node| match node.props {
                ui_ext::Props::Flex(flex) => flex.background.map(|color| format!("{color:?}")),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            backgrounds.len(),
            ALL_TOKENS.len(),
            "every token as background"
        );
    }

    #[test]
    fn buttons_report_the_last_click() {
        let mut harness = Harness::mounted();
        harness.show(Section::Buttons);
        let SectionNodes::Buttons {
            buttons,
            last_clicked,
        } = &harness.nodes().section
        else {
            panic!("Buttons section not shown");
        };
        let (buttons, last_clicked) = (buttons.clone(), *last_clicked);
        assert_eq!(buttons.len(), 12);
        assert_eq!(
            harness.props(last_clicked),
            TextProps::new("Last clicked: none")
                .color(ColorToken::TextMuted)
                .into()
        );

        let names = buttons
            .iter()
            .map(|&(button, _)| {
                let props = set_props(&harness.click(button));
                assert_eq!(props.len(), 1);
                assert_eq!(props[0].0, last_clicked);
                match &props[0].1 {
                    Props::Text(text) => text.content.clone(),
                    props => panic!("expected text, got {props:?}"),
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(names[0], "Last clicked: Default");
        assert_eq!(names[1], "Last clicked: Filled");
        assert_eq!(
            names[5],
            "Last clicked: Filled (disabled: the host should not have sent this)"
        );
        assert_eq!(names[11], "Last clicked: Icon only (settings)");

        let (icon_only, _) = buttons[11];
        assert_eq!(
            harness.props(icon_only),
            ButtonProps::new("")
                .icon(Icon::Settings)
                .style(ButtonStyle::Subtle)
                .into()
        );
        assert_eq!(
            harness.click(icon_only),
            Update::Unchanged,
            "same label again"
        );
    }

    #[test]
    fn input_echoes_only_on_submit() {
        let mut harness = Harness::mounted();
        harness.show(Section::Input);
        let &SectionNodes::Input { input, submitted } = &harness.nodes().section else {
            panic!("Input section not shown");
        };
        assert_eq!(
            harness.props(input),
            InputProps::new("").placeholder(INPUT_PLACEHOLDER).into()
        );

        assert_eq!(
            harness.event(UiEvent::InputChanged((input, "hel".into()))),
            Update::Unchanged
        );
        assert_eq!(
            harness.event(UiEvent::InputChanged((input, "hello".into()))),
            Update::Unchanged
        );
        assert_eq!(
            set_props(&harness.event(UiEvent::InputSubmitted((input, "hello".into())))),
            [(submitted, TextProps::new("Submitted: hello").into())],
            "only the label changes; the input's text stays the user's"
        );
        assert_eq!(
            set_props(&harness.event(UiEvent::InputSubmitted((input, String::new())))),
            [(submitted, TextProps::new("Submitted: (empty)").into())]
        );
        assert_eq!(
            harness.event(UiEvent::InputSubmitted((submitted, "x".into()))),
            Update::Unchanged,
            "not an input"
        );

        // The typed text survives leaving the section: the mirror recorded it without a patch.
        harness.event(UiEvent::InputChanged((input, "kept".into())));
        harness.show(Section::Text);
        harness.show(Section::Input);
        let &SectionNodes::Input { input, submitted } = &harness.nodes().section else {
            panic!("Input section not shown");
        };
        assert_eq!(
            harness.props(input),
            InputProps::new("kept")
                .placeholder(INPUT_PLACEHOLDER)
                .into()
        );
        assert_eq!(
            harness.props(submitted),
            TextProps::new("Submitted: (empty)").into()
        );
    }

    #[test]
    fn row_click_updates_the_label_and_the_status_item() {
        let mut harness = Harness::mounted();
        take_pushed_updates();
        harness.show(Section::List);
        let &SectionNodes::List { list, selected } = &harness.nodes().section else {
            panic!("List section not shown");
        };

        // Without a mounted status item there is nothing to push to.
        harness.event(UiEvent::RowClick((list, 0)));
        assert_eq!(take_pushed_updates(), []);

        let status = harness.gallery.mount(STATUS_SURFACE).expect("status mount");
        assert_eq!(
            status
                .iter()
                .map(|node| (node.id, node.parent))
                .collect::<Vec<_>>(),
            [(1, None), (2, Some(1)), (3, Some(1))]
        );
        assert_eq!(
            status[0].props,
            FlexProps::row().gap(4).align(Align::Center).into()
        );
        assert_eq!(
            status[1].props,
            IconProps::new(Icon::ListTree)
                .color(ColorToken::TextMuted)
                .into()
        );
        assert_eq!(
            status[2].props,
            TextProps::new("Gallery: Row 1").into(),
            "a late mount shows the current selection"
        );

        assert_eq!(
            set_props(&harness.event(UiEvent::RowClick((list, 999)))),
            [(selected, TextProps::new("Selected: Row 1000").into())]
        );
        assert_eq!(
            take_pushed_updates(),
            [(
                STATUS_SURFACE.to_string(),
                Update::Patches(vec![Patch::SetProps((
                    3,
                    TextProps::new("Gallery: Row 1000").into()
                ))])
            )]
        );

        assert_eq!(
            harness.event(UiEvent::RowClick((list, 999))),
            Update::Unchanged,
            "same row again"
        );
        assert_eq!(take_pushed_updates(), []);

        assert_eq!(
            harness
                .gallery
                .handle_event(STATUS_SURFACE, UiEvent::Click(3)),
            Ok(Update::Unchanged)
        );

        // The selection survives leaving the section.
        harness.show(Section::Table);
        harness.show(Section::List);
        let &SectionNodes::List { selected, .. } = &harness.nodes().section else {
            panic!("List section not shown");
        };
        assert_eq!(
            harness.props(selected),
            TextProps::new("Selected: Row 1000").into()
        );
        assert_eq!(take_pushed_updates(), []);
    }

    #[test]
    fn table_rows_fill_every_column() {
        let mut harness = Harness::mounted();
        harness.show(Section::Table);
        let &SectionNodes::Table { table } = &harness.nodes().section else {
            panic!("Table section not shown");
        };
        assert_eq!(
            harness.props(table),
            TableProps::new(["Name", "Kind", "Lines"], 100).into()
        );
        let rows = harness.gallery.list_rows(GALLERY_SURFACE, table, 0, 200);
        assert_eq!(rows.len(), 100);
        for row in &rows {
            let host_row = ui_ext::Tree::from_row(to_host(&row.nodes), TreeLimits::default())
                .expect("the host accepts the row");
            let root = host_row.roots()[0];
            assert!(matches!(
                host_row.get(root).map(|node| node.props()),
                Some(ui_ext::Props::Flex(_))
            ));
            assert_eq!(host_row.children(Some(root)).len(), TABLE_COLUMNS.len());
        }
        assert_eq!(
            row_texts(&rows[..2]),
            [
                ["file_001.rs", "source", "10"],
                ["file_002.rs", "test", "47"],
            ]
        );
        assert_eq!(
            harness.event(UiEvent::RowClick((table, 3))),
            Update::Unchanged
        );
    }

    #[test]
    fn list_rows_only_for_the_shown_list() {
        let mut harness = Harness::mounted();
        harness.show(Section::List);
        let &SectionNodes::List { list, .. } = &harness.nodes().section else {
            panic!("List section not shown");
        };
        assert_eq!(
            harness.gallery.list_rows(GALLERY_SURFACE, list, 0, 3).len(),
            3
        );
        assert!(
            harness
                .gallery
                .list_rows(STATUS_SURFACE, list, 0, 3)
                .is_empty()
        );
        assert!(
            harness
                .gallery
                .list_rows(GALLERY_SURFACE, 1, 0, 3)
                .is_empty()
        );

        harness.show(Section::Table);
        assert!(
            harness
                .gallery
                .list_rows(GALLERY_SURFACE, list, 0, 3)
                .is_empty(),
            "the old list is gone"
        );
        assert_eq!(
            harness.event(UiEvent::RowClick((list, 0))),
            Update::Unchanged
        );
    }

    #[test]
    fn remount_keeps_the_shown_section_with_new_ids() {
        let mut harness = Harness::mounted();
        harness.show(Section::Icons);
        let old_root = harness.nodes().section_root;
        let old_nav = harness.nav(Section::Icons);

        let nodes = harness.gallery.mount(GALLERY_SURFACE).expect("remount");
        let first = nodes.first().map(|node| node.id).expect("nodes");
        assert!(first > old_root, "ids keep counting up across mounts");
        let shown_nav = harness.nav(Section::Icons);
        assert_ne!(shown_nav, old_nav);
        assert_eq!(
            harness.gallery.gallery.props(shown_nav),
            Some(&Section::Icons.nav_button(Section::Icons).into())
        );
        assert_eq!(
            harness
                .gallery
                .handle_event(GALLERY_SURFACE, UiEvent::Click(old_nav)),
            Ok(Update::Unchanged),
            "ids of the previous mount are dead"
        );
    }

    #[test]
    fn icons_reach_a_0_1_host_as_zed_icon_names() {
        // Checked against the source rather than through a dependency on the GPL `icons` crate.
        let icons_source = include_str!("../../../crates/icons/src/icons.rs");
        let variants = icons_source
            .lines()
            .filter_map(|line| line.trim().strip_suffix(','))
            .collect::<BTreeSet<_>>();
        let camel_case = |name: &str| {
            name.split('_')
                .map(|word| {
                    let mut chars = word.chars();
                    chars.next().map_or_else(String::new, |first| {
                        first.to_uppercase().chain(chars).collect()
                    })
                })
                .collect::<String>()
        };
        let mut harness = Harness::mounted();
        harness.show(Section::Icons);
        // The harness holds the host's 0.1 view, so frozen icons appear as upstream names.
        let names = harness
            .host
            .to_nodes()
            .into_iter()
            .filter_map(|node| match node.props {
                ui_ext::Props::Icon(icon) => Some(icon.name),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(names.len(), KNOWN_ICONS.len() + 1);
        for name in &names {
            assert!(variants.contains(camel_case(name).as_str()), "{name}");
        }
        assert!(
            names.iter().any(|name| name == "load_circle"),
            "the spinner cell"
        );
        assert!(
            !names.iter().any(|name| name == "blocks"),
            "blocks is the host's fallback"
        );
    }
}
