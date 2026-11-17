//! Plain Rust mirror of the `types` interface in `wit/fyx-ui.wit`.
//!
//! The host converts the wasmtime-generated bindings into these types at the boundary, so the
//! renderer never sees wasmtime types. The serde form (externally tagged, kebab-case, the same
//! names as the WIT) is the JSON schema for transports other than WASM, such as a sidecar.

use serde::{Deserialize, Serialize};

/// Chosen by the extension, unique within one surface.
pub type NodeId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorToken {
    Text,
    TextMuted,
    TextAccent,
    Error,
    Warning,
    Success,
    Border,
    Surface,
    ElevatedSurface,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlexDirection {
    Row,
    #[default]
    Column,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FlexProps {
    pub direction: FlexDirection,
    pub gap: u16,
    pub padding: u16,
    pub grow: bool,
    pub align: Align,
    pub background: Option<ColorToken>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TextProps {
    pub content: String,
    pub color: Option<ColorToken>,
    pub bold: bool,
    pub monospace: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ButtonStyle {
    #[default]
    Default,
    Filled,
    Subtle,
    Danger,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ButtonProps {
    pub label: String,
    pub icon: Option<String>,
    pub style: ButtonStyle,
    pub disabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct InputProps {
    pub value: String,
    pub placeholder: String,
}

/// Virtualized: rows come from `list-rows`, not from child nodes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ListProps {
    pub row_count: u32,
    /// Pixels, the same for every row.
    pub row_height: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TableProps {
    pub columns: Vec<String>,
    pub row_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct IconProps {
    pub name: String,
    pub color: Option<ColorToken>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DiffViewProps {
    pub old_text: String,
    pub new_text: String,
    pub language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Props {
    Flex(FlexProps),
    Text(TextProps),
    Button(ButtonProps),
    Input(InputProps),
    List(ListProps),
    Table(TableProps),
    Icon(IconProps),
    Markdown(String),
    DiffView(DiffViewProps),
}

impl Props {
    /// Only flex nodes may have child nodes. Lists and tables get their rows from `list-rows`.
    pub fn is_container(&self) -> bool {
        matches!(self, Props::Flex(_))
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Props::Flex(_) => "flex",
            Props::Text(_) => "text",
            Props::Button(_) => "button",
            Props::Input(_) => "input",
            Props::List(_) => "list",
            Props::Table(_) => "table",
            Props::Icon(_) => "icon",
            Props::Markdown(_) => "markdown",
            Props::DiffView(_) => "diff-view",
        }
    }
}

/// One entry of a flat tree. Parents precede children; siblings keep list order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub props: Props,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct InsertOp {
    pub node: Node,
    /// Insert before this sibling; `None` appends.
    pub before: Option<NodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Patch {
    SetProps(NodeId, Props),
    Insert(InsertOp),
    /// Removes the node and its whole subtree.
    Remove(NodeId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Update {
    Unchanged,
    Replace(Vec<Node>),
    Patches(Vec<Patch>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ListRow {
    pub index: u32,
    /// A subtree whose single root has `parent: None`.
    pub nodes: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiEvent {
    Click(NodeId),
    InputChanged(NodeId, String),
    InputSubmitted(NodeId, String),
    /// The list node and the row index within it.
    RowClick(NodeId, u32),
}

impl UiEvent {
    /// The node the event is routed to.
    pub fn target(&self) -> NodeId {
        match self {
            UiEvent::Click(id)
            | UiEvent::InputChanged(id, _)
            | UiEvent::InputSubmitted(id, _)
            | UiEvent::RowClick(id, _) => *id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DockPosition {
    Left,
    Right,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PanelDescriptor {
    pub position: DockPosition,
    pub icon: String,
    pub default_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusBarSide {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StatusItemDescriptor {
    pub side: StatusBarSide,
    /// Higher sorts closer to the edge.
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceKind {
    Panel(PanelDescriptor),
    StatusItem(StatusItemDescriptor),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Surface {
    pub id: String,
    pub title: String,
    pub kind: SurfaceKind,
}

/// `log-level` from the `host` interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
