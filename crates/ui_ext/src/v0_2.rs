//! Plain Rust mirror of the `types` interface in `wit/since_v0.2.0/fyx-ui.wit`, and the model that
//! 0.1 data converts into: every 0.1 value has exactly one 0.2 counterpart (`From`), with icons as
//! [`IconRef::Upstream`] and the 0.2-only fields empty.
//!
//! It is the newest model, including the staged 0.3 additions built so far: [`Props::Image`]
//! (design.md §12), [`Props::Canvas`] and [`UiEvent::CanvasClick`] (§13), and the 0.4
//! [`Props::Terminal`]. A 0.1 or 0.2 component cannot produce those, because its world has no such
//! case; only a 0.3 component's values convert into them.
//!
//! It is a module of its own rather than new fields and cases on the 0.1 types in
//! [`crate::types`], because the host and the renderer build those with struct literals and match
//! them exhaustively. Both move to this model when they implement 0.2 (platform plan 2.2, 2.3);
//! until then the 0.1 types, their JSON form and [`crate::Tree`] stay exactly as they were.
//!
//! The serde form follows the 0.2 WIT names, as the 0.1 form follows the 0.1 WIT.

use serde::{Deserialize, Serialize};

pub mod notifications;
pub mod tree;

pub use notifications::{
    CAPABILITY_NOTIFICATIONS, Notification, NotificationAction, NotificationActionEvent,
    NotificationCloseReason, NotificationLevel, ProgressLocation, ProgressOptions, ProgressValue,
};

use crate::api_version::{ApiError, ApiVersion, MAX_INDENT_LEVELS, MAX_TOOLTIP_BYTES};
use crate::tree::{TreeError, TreeLimits};
use crate::types as v0_1;

pub use crate::types::{
    Align, ButtonStyle, ColorToken, DiffViewProps, DockPosition, FlexDirection, FlexProps,
    LogLevel, NodeId, StatusBarSide, StatusItemDescriptor, TableProps, TextProps,
};

macro_rules! icons {
    ($($(#[$meta:meta])* $variant:ident => $name:literal,)*) => {
        /// The frozen icon set of 0.2 (design.md, section I). The host maps every case to its own
        /// icon set, so an upstream rename cannot change an extension's icons.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum Icon {
            $($(#[$meta])* #[serde(rename = $name)] $variant,)*
        }

        impl Icon {
            /// In WIT declaration order.
            pub const ALL: &'static [Icon] = &[$(Icon::$variant,)*];

            /// The WIT case name, which is also the JSON form.
            pub fn name(self) -> &'static str {
                match self {
                    $(Icon::$variant => $name,)*
                }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Icon::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

icons! {
        // Navigation and structure
        ArrowUp => "arrow-up",
        ArrowDown => "arrow-down",
        ArrowLeft => "arrow-left",
        ArrowRight => "arrow-right",
        ChevronUp => "chevron-up",
        ChevronDown => "chevron-down",
        ChevronLeft => "chevron-left",
        ChevronRight => "chevron-right",
        Expand => "expand",
        Collapse => "collapse",
        Menu => "menu",
        Ellipsis => "ellipsis",
        ListTree => "list-tree",
        ListTodo => "list-todo",
        Table => "table",
        FileTree => "file-tree",
        // Actions
        Plus => "plus",
        Dash => "dash",
        Check => "check",
        CheckDouble => "check-double",
        Close => "close",
        Search => "search",
        Filter => "filter",
        Refresh => "refresh",
        Undo => "undo",
        Copy => "copy",
        Pencil => "pencil",
        Trash => "trash",
        Replace => "replace",
        Download => "download",
        Send => "send",
        Share => "share",
        Link => "link",
        Pin => "pin",
        Unpin => "unpin",
        Play => "play",
        Pause => "pause",
        Stop => "stop",
        Maximize => "maximize",
        Minimize => "minimize",
        Split => "split",
        // Files and code
        File => "file",
        FileCode => "file-code",
        FileDiff => "file-diff",
        FileText => "file-text",
        Folder => "folder",
        FolderOpen => "folder-open",
        Code => "code",
        Terminal => "terminal",
        Diff => "diff",
        Regex => "regex",
        CaseSensitive => "case-sensitive",
        WholeWord => "whole-word",
        Hash => "hash",
        AtSign => "at-sign",
        // Version control
        GitBranch => "git-branch",
        GitCommit => "git-commit",
        PullRequest => "pull-request",
        // Status and feedback
        Info => "info",
        Warning => "warning",
        Error => "error",
        Help => "help",
        Bell => "bell",
        BellDot => "bell-dot",
        Circle => "circle",
        Dot => "dot",
        Star => "star",
        StarFilled => "star-filled",
        Bookmark => "bookmark",
        ThumbsUp => "thumbs-up",
        ThumbsDown => "thumbs-down",
        TodoPending => "todo-pending",
        TodoProgress => "todo-progress",
        TodoComplete => "todo-complete",
        /// Drawn rotating by the host, so a busy indicator needs no ticks.
        Spinner => "spinner",
        /// Draws nothing at icon size; aligns rows that have no icon (unchecked checkboxes).
        Blank => "blank",
        // Objects
        Settings => "settings",
        Clock => "clock",
        Debug => "debug",
        Eye => "eye",
        EyeOff => "eye-off",
        Lock => "lock",
        Unlock => "unlock",
        Person => "person",
        People => "people",
        Chat => "chat",
        Envelope => "envelope",
        Book => "book",
        Notepad => "notepad",
        Image => "image",
        Globe => "globe",
        Server => "server",
        Database => "database",
        Keyboard => "keyboard",
        Box => "box",
        Archive => "archive",
        Sparkle => "sparkle",
        Bolt => "bolt",
        Flame => "flame",
        Power => "power",
        /// The generic extension icon; also what the host shows for a 0.1 icon name it cannot map.
        Blocks => "blocks",
}

/// An icon of either world. A 0.1 component names upstream icons by string, and the renderer keeps
/// resolving those as it does today; a 0.2 component can only name a frozen [`Icon`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "IconRefRepr", into = "IconRefRepr")]
pub enum IconRef {
    Frozen(Icon),
    Upstream(String),
}

/// A frozen icon is its bare WIT name, as in the 0.2 WIT. A 0.1 name is `{"upstream": name}`, so it
/// is never mistaken for a frozen icon of the same spelling, and a misspelled frozen name fails to
/// parse instead of falling back to some upstream icon.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum IconRefRepr {
    Frozen(Icon),
    Upstream { upstream: String },
}

impl From<IconRefRepr> for IconRef {
    fn from(repr: IconRefRepr) -> Self {
        match repr {
            IconRefRepr::Frozen(icon) => IconRef::Frozen(icon),
            IconRefRepr::Upstream { upstream } => IconRef::Upstream(upstream),
        }
    }
}

impl From<IconRef> for IconRefRepr {
    fn from(icon: IconRef) -> Self {
        match icon {
            IconRef::Frozen(icon) => IconRefRepr::Frozen(icon),
            IconRef::Upstream(upstream) => IconRefRepr::Upstream { upstream },
        }
    }
}

impl From<Icon> for IconRef {
    fn from(icon: Icon) -> Self {
        IconRef::Frozen(icon)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ButtonProps {
    pub label: String,
    pub icon: Option<IconRef>,
    pub style: ButtonStyle,
    pub disabled: bool,
    /// Kept as the extension sent it; [`ButtonProps::tooltip_text`] is what the host shows.
    pub tooltip: Option<String>,
    /// The accessible name (integration-5 A5), read by screen readers; without it, a non-empty
    /// `label`, then the tooltip, then the icon's name. Kept as the extension sent it;
    /// [`ButtonProps::accessible_label_text`] is what the host uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessible_label: Option<String>,
}

impl ButtonProps {
    /// The tooltip as drawn: at most [`MAX_TOOLTIP_BYTES`], cut at a character boundary.
    pub fn tooltip_text(&self) -> Option<&str> {
        self.tooltip.as_deref().map(truncate_tooltip)
    }

    /// The accessible label as used: at most [`MAX_LABEL_BYTES`], cut at a character boundary.
    pub fn accessible_label_text(&self) -> Option<&str> {
        self.accessible_label
            .as_deref()
            .map(|label| truncate_to(label, MAX_LABEL_BYTES))
    }
}

/// The longest prefix of `text` that fits in [`MAX_TOOLTIP_BYTES`] without splitting a character.
pub fn truncate_tooltip(text: &str) -> &str {
    truncate_to(text, MAX_TOOLTIP_BYTES)
}

/// The longest prefix of `text` that fits in `max` bytes without splitting a character.
fn truncate_to(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct InputProps {
    pub value: String,
    pub placeholder: String,
    /// The host masks the text and never logs the value.
    pub secret: bool,
    /// The accessible name (UX F16), read by screen readers; without it, the placeholder. Kept as
    /// the extension sent it; [`InputProps::label_text`] is what the host uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The most bytes of an input's or a button's accessible label the host uses.
pub const MAX_LABEL_BYTES: usize = 256;

impl InputProps {
    /// The accessible label as used: at most [`MAX_LABEL_BYTES`], cut at a character boundary.
    pub fn label_text(&self) -> Option<&str> {
        self.label
            .as_deref()
            .map(|label| truncate_to(label, MAX_LABEL_BYTES))
    }
}

/// Virtualized: rows come from `list-rows`, not from child nodes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ListProps {
    pub row_count: u32,
    /// Pixels, the same for every row.
    pub row_height: u16,
    pub selected: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct IconProps {
    pub name: IconRef,
    pub color: Option<ColorToken>,
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
    /// 0.3 (design.md §12): checked by [`crate::image::check_image_props`] and
    /// [`crate::image::check_image_placement`] when the host applies the update.
    Image(crate::image::ImageProps),
    /// 0.3 (design.md §13): checked by [`crate::canvas::check_canvas_props`] and
    /// [`crate::canvas::check_canvas_placement`] when the host applies the update; the tree
    /// allows [`crate::canvas::MAX_CANVASES_PER_SURFACE`] and none in a list row.
    Canvas(crate::canvas::CanvasProps),
    /// 0.4 (docs/fyx/wit-0.4/design.md §B.2): the native view of one of the extension's own
    /// terminals (a pseudo-terminal or one it created) inside its panel. Not in a list row. The
    /// renderer resolves the handle within the extension, so another's terminal never shows,
    /// and draws one node per handle; a handle shown in a node is not also a tab.
    Terminal(TerminalNodeProps),
}

/// The WIT `terminal-props` (0.4): the extension's `terminal-handle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TerminalNodeProps {
    pub handle: u64,
}

impl Props {
    /// Only flex nodes may have child nodes. Lists and tables get their rows from `list-rows`.
    pub fn is_container(&self) -> bool {
        matches!(self, Props::Flex(_))
    }

    /// The string bytes these props retain: what [`tree::MAX_TREE_TEXT_BYTES`] counts, and
    /// what the host counts against its per-call cap on lifted text.
    pub fn text_len(&self) -> usize {
        let option_len = |text: &Option<String>| text.as_ref().map_or(0, String::len);
        let icon_len = |icon: &IconRef| match icon {
            IconRef::Frozen(_) => 0,
            IconRef::Upstream(name) => name.len(),
        };
        match self {
            Props::Flex(_) | Props::List(_) | Props::Terminal(_) => 0,
            Props::Text(props) => props.content.len(),
            Props::Button(props) => {
                props.label.len()
                    + props.icon.as_ref().map_or(0, icon_len)
                    + option_len(&props.tooltip)
                    + option_len(&props.accessible_label)
            }
            Props::Input(props) => {
                props.value.len() + props.placeholder.len() + option_len(&props.label)
            }
            Props::Table(props) => props.columns.iter().map(String::len).sum(),
            Props::Icon(props) => icon_len(&props.name),
            Props::Markdown(text) => text.len(),
            Props::DiffView(props) => {
                props.old_text.len() + props.new_text.len() + option_len(&props.language)
            }
            Props::Canvas(props) => crate::canvas::canvas_text_len(props),
            Props::Image(props) => {
                props.alt.len()
                    + match &props.source {
                        crate::image::ImageSource::Asset(path) => path.len(),
                        crate::image::ImageSource::Generated(_) => 0,
                    }
            }
        }
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
            Props::Image(_) => "image",
            Props::Canvas(_) => "canvas",
            Props::Terminal(_) => "terminal",
        }
    }
}

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
    pub before: Option<NodeId>,
}

/// `count` rows from `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RowRange {
    pub start: u32,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct InvalidateRowsOp {
    pub target: NodeId,
    /// `None` invalidates every row.
    pub rows: Option<RowRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Patch {
    SetProps(NodeId, Props),
    Insert(InsertOp),
    Remove(NodeId),
    InvalidateRows(InvalidateRowsOp),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Update {
    Unchanged,
    Replace(Vec<Node>),
    Patches(Vec<Patch>),
}

impl Update {
    /// The caps `push-update` checks at the call (design.md, F): at most
    /// [`TreeLimits::max_nodes_per_update`] patches, and a `replace` of at most
    /// [`TreeLimits::max_nodes`] nodes, the same cap as a whole tree and a `mount`, so a surface
    /// can always be resent whole. Whether the update fits the tree is only known when it is
    /// applied.
    pub fn check_caps(&self, limits: TreeLimits) -> Result<(), TreeError> {
        match self {
            Update::Unchanged => Ok(()),
            Update::Replace(nodes) if nodes.len() > limits.max_nodes => {
                Err(TreeError::TreeTooLarge {
                    max: limits.max_nodes,
                })
            }
            Update::Replace(_) => Ok(()),
            Update::Patches(patches) if patches.len() > limits.max_nodes_per_update => {
                Err(TreeError::UpdateTooLarge {
                    count: patches.len(),
                    max: limits.max_nodes_per_update,
                })
            }
            Update::Patches(_) => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ListRow {
    pub index: u32,
    /// Kept as the extension sent it; [`ListRow::indent_levels`] is what the host draws.
    pub indent: u8,
    /// A subtree whose single root has `parent: None`. Ids are row-local.
    pub nodes: Vec<Node>,
}

impl ListRow {
    /// The nesting depth as drawn: deeper rows are clamped to [`MAX_INDENT_LEVELS`].
    pub fn indent_levels(&self) -> u8 {
        self.indent.min(MAX_INDENT_LEVELS)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DismissReason {
    Escape,
    FocusLost,
    Replaced,
    WindowClosed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiEvent {
    Click(NodeId),
    InputChanged(NodeId, String),
    InputSubmitted(NodeId, String),
    /// The list node and the row index within it.
    RowClick(NodeId, u32),
    /// The list node and the newly selected row (modals only).
    RowSelected(NodeId, u32),
    /// A modal was closed by the user; it has no tree any more.
    Dismissed(DismissReason),
    /// §M. An action button of a notification the surface owns was clicked; a gesture, like
    /// `click`.
    NotificationAction(NotificationActionEvent),
    /// §M. A notification the surface owns closed without the extension asking.
    NotificationClosed(u64, NotificationCloseReason),
    /// §M. Cancel was pressed on a cancellable progress the surface owns.
    ProgressCancelRequested(u64),
    /// 0.3 (§13). A click on a clickable canvas, in canvas coordinates; a gesture, like `click`.
    CanvasClick(crate::canvas::CanvasClick),
}

impl UiEvent {
    /// The node the event is routed to; `dismissed` belongs to the whole surface.
    pub fn target(&self) -> Option<NodeId> {
        match self {
            UiEvent::Click(id)
            | UiEvent::InputChanged(id, _)
            | UiEvent::InputSubmitted(id, _)
            | UiEvent::RowClick(id, _)
            | UiEvent::RowSelected(id, _) => Some(*id),
            UiEvent::CanvasClick(click) => Some(click.canvas),
            UiEvent::Dismissed(_)
            | UiEvent::NotificationAction(_)
            | UiEvent::NotificationClosed(..)
            | UiEvent::ProgressCancelRequested(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PanelDescriptor {
    pub position: DockPosition,
    pub icon: IconRef,
    pub default_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModalWidth {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ModalDescriptor {
    pub width: ModalWidth,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceKind {
    Panel(PanelDescriptor),
    StatusItem(StatusItemDescriptor),
    Modal(ModalDescriptor),
}

impl SurfaceKind {
    /// The `[fyx_ui].capabilities` entry the manifest needs for a surface of this kind.
    pub fn capability(&self) -> &'static str {
        match self {
            SurfaceKind::Panel(_) => CAPABILITY_PANEL,
            SurfaceKind::StatusItem(_) => CAPABILITY_STATUS_BAR,
            SurfaceKind::Modal(_) => CAPABILITY_MODAL,
        }
    }
}

pub const CAPABILITY_PANEL: &str = "ui:panel";
pub const CAPABILITY_STATUS_BAR: &str = "ui:status-bar";
pub const CAPABILITY_MODAL: &str = "ui:modal";
pub const CAPABILITY_EDITOR_DECORATIONS: &str = "ui:editor-decorations";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Surface {
    pub id: String,
    pub title: String,
    pub kind: SurfaceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DecorationPlacement {
    Above,
    Below,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DecorationLocation {
    /// Relative to a worktree root; see [`crate::check_decoration_path`].
    pub path: String,
    /// Zero-based row at the time the decoration is anchored.
    pub line: u32,
    pub placement: DecorationPlacement,
    /// Block height in editor lines; `None` lets the host estimate it.
    pub height_lines: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostErrorCode {
    UnknownSurface,
    LimitExceeded,
    InvalidArgument,
    NotPermitted,
    Unavailable,
    /// 0.3: the user refused or revoked a T2 capability the manifest declares. A 0.2 component
    /// never gets it; it sees `not-permitted`.
    Denied,
    /// 0.3 (integration-5 A5): the worktree a T2 call works in is not trusted (restricted mode).
    /// A 0.2 component never gets it; it sees `not-permitted`.
    UntrustedWorktree,
    /// 0.3: the call needs an open worktree and there is none. A 0.2 component sees `unavailable`.
    NoWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct HostError {
    pub code: HostErrorCode,
    pub message: String,
}

impl From<&ApiError> for HostError {
    fn from(error: &ApiError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

impl From<v0_1::ButtonProps> for ButtonProps {
    fn from(props: v0_1::ButtonProps) -> Self {
        Self {
            label: props.label,
            icon: props.icon.map(IconRef::Upstream),
            style: props.style,
            disabled: props.disabled,
            tooltip: None,
            accessible_label: None,
        }
    }
}

impl From<v0_1::InputProps> for InputProps {
    fn from(props: v0_1::InputProps) -> Self {
        Self {
            value: props.value,
            placeholder: props.placeholder,
            secret: false,
            label: None,
        }
    }
}

impl From<v0_1::ListProps> for ListProps {
    fn from(props: v0_1::ListProps) -> Self {
        Self {
            row_count: props.row_count,
            row_height: props.row_height,
            selected: None,
        }
    }
}

impl From<v0_1::IconProps> for IconProps {
    fn from(props: v0_1::IconProps) -> Self {
        Self {
            name: IconRef::Upstream(props.name),
            color: props.color,
        }
    }
}

impl From<v0_1::Props> for Props {
    fn from(props: v0_1::Props) -> Self {
        match props {
            v0_1::Props::Flex(props) => Props::Flex(props),
            v0_1::Props::Text(props) => Props::Text(props),
            v0_1::Props::Button(props) => Props::Button(props.into()),
            v0_1::Props::Input(props) => Props::Input(props.into()),
            v0_1::Props::List(props) => Props::List(props.into()),
            v0_1::Props::Table(props) => Props::Table(props),
            v0_1::Props::Icon(props) => Props::Icon(props.into()),
            v0_1::Props::Markdown(text) => Props::Markdown(text),
            v0_1::Props::DiffView(props) => Props::DiffView(props),
        }
    }
}

impl From<v0_1::Node> for Node {
    fn from(node: v0_1::Node) -> Self {
        Self {
            id: node.id,
            parent: node.parent,
            props: node.props.into(),
        }
    }
}

impl From<v0_1::InsertOp> for InsertOp {
    fn from(op: v0_1::InsertOp) -> Self {
        Self {
            node: op.node.into(),
            before: op.before,
        }
    }
}

impl From<v0_1::Patch> for Patch {
    fn from(patch: v0_1::Patch) -> Self {
        match patch {
            v0_1::Patch::SetProps(id, props) => Patch::SetProps(id, props.into()),
            v0_1::Patch::Insert(op) => Patch::Insert(op.into()),
            v0_1::Patch::Remove(id) => Patch::Remove(id),
        }
    }
}

impl From<v0_1::Update> for Update {
    fn from(update: v0_1::Update) -> Self {
        match update {
            v0_1::Update::Unchanged => Update::Unchanged,
            v0_1::Update::Replace(nodes) => {
                Update::Replace(nodes.into_iter().map(Into::into).collect())
            }
            v0_1::Update::Patches(patches) => {
                Update::Patches(patches.into_iter().map(Into::into).collect())
            }
        }
    }
}

impl From<v0_1::ListRow> for ListRow {
    fn from(row: v0_1::ListRow) -> Self {
        Self {
            index: row.index,
            indent: 0,
            nodes: row.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<v0_1::UiEvent> for UiEvent {
    fn from(event: v0_1::UiEvent) -> Self {
        match event {
            v0_1::UiEvent::Click(id) => UiEvent::Click(id),
            v0_1::UiEvent::InputChanged(id, text) => UiEvent::InputChanged(id, text),
            v0_1::UiEvent::InputSubmitted(id, text) => UiEvent::InputSubmitted(id, text),
            v0_1::UiEvent::RowClick(id, row) => UiEvent::RowClick(id, row),
        }
    }
}

/// Events are the one thing that goes from the model back to a 0.1 component. The host never
/// produces `row-selected` or `dismissed` for a 0.1 surface (0.1 has no modals), so a failure here
/// means a host bug, and the event is dropped rather than sent.
impl TryFrom<UiEvent> for v0_1::UiEvent {
    type Error = ApiError;

    fn try_from(event: UiEvent) -> Result<Self, ApiError> {
        let not_in_v0_1 = |what| ApiError::NotInVersion {
            what,
            version: ApiVersion::V0_1_0,
        };
        match event {
            UiEvent::Click(id) => Ok(v0_1::UiEvent::Click(id)),
            UiEvent::InputChanged(id, text) => Ok(v0_1::UiEvent::InputChanged(id, text)),
            UiEvent::InputSubmitted(id, text) => Ok(v0_1::UiEvent::InputSubmitted(id, text)),
            UiEvent::RowClick(id, row) => Ok(v0_1::UiEvent::RowClick(id, row)),
            UiEvent::RowSelected(..) => Err(not_in_v0_1("the row-selected event")),
            UiEvent::Dismissed(_) => Err(not_in_v0_1("the dismissed event")),
            UiEvent::NotificationAction(_)
            | UiEvent::NotificationClosed(..)
            | UiEvent::ProgressCancelRequested(_) => {
                Err(not_in_v0_1("notification and progress events"))
            }
            UiEvent::CanvasClick(_) => Err(not_in_v0_1("the canvas-click event")),
        }
    }
}

impl From<v0_1::PanelDescriptor> for PanelDescriptor {
    fn from(panel: v0_1::PanelDescriptor) -> Self {
        Self {
            position: panel.position,
            icon: IconRef::Upstream(panel.icon),
            default_size: panel.default_size,
        }
    }
}

impl From<v0_1::SurfaceKind> for SurfaceKind {
    fn from(kind: v0_1::SurfaceKind) -> Self {
        match kind {
            v0_1::SurfaceKind::Panel(panel) => SurfaceKind::Panel(panel.into()),
            v0_1::SurfaceKind::StatusItem(item) => SurfaceKind::StatusItem(item),
        }
    }
}

impl From<v0_1::Surface> for Surface {
    fn from(surface: v0_1::Surface) -> Self {
        Self {
            id: surface.id,
            title: surface.title,
            kind: surface.kind.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_version::ApiVersion;

    const WIT: &str = include_str!("../wit/since_v0.2.0/fyx-ui.wit");

    /// The case names of `enum <name> { ... }` in the 0.2 WIT, in order.
    fn wit_enum_cases(name: &str) -> Vec<&'static str> {
        let start = WIT
            .find(&format!("enum {name} {{"))
            .expect("enum is declared in the WIT");
        let body = WIT.get(start..).unwrap_or_default();
        let body = body
            .get(body.find('{').map_or(0, |open| open + 1)..body.find('}').unwrap_or(0))
            .unwrap_or_default();
        body.lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//"))
            .flat_map(|line| line.split(','))
            .map(str::trim)
            .filter(|case| !case.is_empty())
            .collect()
    }

    #[test]
    fn icon_set_matches_the_wit() {
        let names: Vec<&str> = Icon::ALL.iter().map(|icon| icon.name()).collect();
        assert_eq!(names, wit_enum_cases("icon"));
        assert_eq!(Icon::ALL.len(), 101);
        for &icon in Icon::ALL {
            assert_eq!(Icon::from_name(icon.name()), Some(icon));
            assert_eq!(
                serde_json::to_value(icon).expect("serializes"),
                serde_json::json!(icon.name())
            );
        }
        assert_eq!(Icon::from_name("rotate_cw"), None);
        assert_eq!(Icon::from_name("Arrow-Up"), None);
    }

    #[test]
    fn small_enums_match_the_wit() {
        let names = |values: &[serde_json::Value]| -> Vec<String> {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        };
        let json = |value: serde_json::Result<serde_json::Value>| value.expect("serializes");
        assert_eq!(
            names(&[
                json(serde_json::to_value(DismissReason::Escape)),
                json(serde_json::to_value(DismissReason::FocusLost)),
                json(serde_json::to_value(DismissReason::Replaced)),
                json(serde_json::to_value(DismissReason::WindowClosed)),
            ]),
            wit_enum_cases("dismiss-reason")
        );
        assert_eq!(
            names(&[
                json(serde_json::to_value(ModalWidth::Small)),
                json(serde_json::to_value(ModalWidth::Medium)),
                json(serde_json::to_value(ModalWidth::Large)),
            ]),
            wit_enum_cases("modal-width")
        );
        assert_eq!(
            names(&[
                json(serde_json::to_value(DecorationPlacement::Above)),
                json(serde_json::to_value(DecorationPlacement::Below)),
            ]),
            wit_enum_cases("decoration-placement")
        );
        assert_eq!(
            names(&[
                json(serde_json::to_value(HostErrorCode::UnknownSurface)),
                json(serde_json::to_value(HostErrorCode::LimitExceeded)),
                json(serde_json::to_value(HostErrorCode::InvalidArgument)),
                json(serde_json::to_value(HostErrorCode::NotPermitted)),
                json(serde_json::to_value(HostErrorCode::Unavailable)),
            ]),
            wit_enum_cases("host-error-code")
        );
    }

    #[test]
    fn icon_ref_json_form() {
        let frozen = IconRef::Frozen(Icon::Check);
        let upstream = IconRef::Upstream("check".into());
        assert_eq!(
            serde_json::to_value(&frozen).expect("serializes"),
            serde_json::json!("check")
        );
        assert_eq!(
            serde_json::to_value(&upstream).expect("serializes"),
            serde_json::json!({ "upstream": "check" })
        );
        for icon in [frozen, upstream] {
            let json = serde_json::to_string(&icon).expect("serializes");
            assert_eq!(
                serde_json::from_str::<IconRef>(&json).expect("parses"),
                icon
            );
        }
        assert!(serde_json::from_str::<IconRef>("\"not-an-icon\"").is_err());
        assert!(serde_json::from_str::<IconRef>("{\"frozen\": \"check\"}").is_err());
    }

    #[test]
    fn tooltip_is_truncated_at_a_character_boundary() {
        assert_eq!(truncate_tooltip(""), "");
        assert_eq!(truncate_tooltip("Refresh"), "Refresh");
        let exact = "a".repeat(MAX_TOOLTIP_BYTES);
        assert_eq!(truncate_tooltip(&exact), exact);
        let long = "a".repeat(MAX_TOOLTIP_BYTES + 1);
        assert_eq!(truncate_tooltip(&long).len(), MAX_TOOLTIP_BYTES);
        // "é" is two bytes and would straddle the cap.
        let straddling = format!("{}é", "a".repeat(MAX_TOOLTIP_BYTES - 1));
        assert_eq!(
            truncate_tooltip(&straddling),
            "a".repeat(MAX_TOOLTIP_BYTES - 1)
        );

        let button = ButtonProps {
            tooltip: Some(long.clone()),
            ..ButtonProps::default()
        };
        assert_eq!(button.tooltip_text(), Some(&long[..MAX_TOOLTIP_BYTES]));
        assert_eq!(button.tooltip.as_deref(), Some(long.as_str()));
        assert_eq!(ButtonProps::default().tooltip_text(), None);

        let input = InputProps {
            label: Some(format!("{}é", "a".repeat(MAX_LABEL_BYTES - 1))),
            ..InputProps::default()
        };
        assert_eq!(
            input.label_text(),
            Some("a".repeat(MAX_LABEL_BYTES - 1).as_str())
        );
        assert_eq!(InputProps::default().label_text(), None);

        let button = ButtonProps {
            accessible_label: Some(format!("{}é", "a".repeat(MAX_LABEL_BYTES - 1))),
            ..ButtonProps::default()
        };
        assert_eq!(
            button.accessible_label_text(),
            Some("a".repeat(MAX_LABEL_BYTES - 1).as_str())
        );
        assert_eq!(ButtonProps::default().accessible_label_text(), None);
    }

    #[test]
    fn indent_is_clamped_when_drawn() {
        let row = |indent| ListRow {
            index: 0,
            indent,
            nodes: Vec::new(),
        };
        for (indent, drawn) in [(0, 0), (1, 1), (32, 32), (33, 32), (u8::MAX, 32)] {
            assert_eq!(row(indent).indent_levels(), drawn);
            assert_eq!(row(indent).indent, indent);
        }
    }

    #[test]
    fn update_caps_allow_a_whole_tree_in_a_replace() {
        let limits = ApiVersion::V0_2_0.tree_limits();
        let nodes = |count: usize| -> Vec<Node> {
            (0..count as NodeId)
                .map(|id| Node {
                    id,
                    parent: None,
                    props: Props::Markdown(String::new()),
                })
                .collect()
        };
        let patches = |count: usize| Update::Patches(vec![Patch::Remove(0); count]);
        assert_eq!(Update::Unchanged.check_caps(limits), Ok(()));
        assert_eq!(Update::Replace(nodes(16384)).check_caps(limits), Ok(()));
        assert_eq!(
            Update::Replace(nodes(16385)).check_caps(limits),
            Err(TreeError::TreeTooLarge { max: 16384 })
        );
        assert_eq!(patches(2048).check_caps(limits), Ok(()));
        assert_eq!(
            patches(2049).check_caps(limits),
            Err(TreeError::UpdateTooLarge {
                count: 2049,
                max: 2048
            })
        );
    }

    #[test]
    fn json_form_matches_wit_names() {
        let update = Update::Patches(vec![
            Patch::SetProps(
                3,
                Props::List(ListProps {
                    row_count: 10,
                    row_height: 22,
                    selected: Some(4),
                }),
            ),
            Patch::InvalidateRows(InvalidateRowsOp {
                target: 3,
                rows: Some(RowRange { start: 2, count: 5 }),
            }),
            Patch::SetProps(
                5,
                Props::Button(ButtonProps {
                    label: "Go".into(),
                    icon: Some(Icon::Play.into()),
                    style: ButtonStyle::Filled,
                    disabled: false,
                    tooltip: Some("Run it".into()),
                    accessible_label: None,
                }),
            ),
        ]);
        let json = serde_json::to_value(&update).expect("serializes");
        assert_eq!(
            json,
            serde_json::json!({
                "patches": [
                    { "set-props": [3, { "list": { "row-count": 10, "row-height": 22, "selected": 4 } }] },
                    { "invalidate-rows": { "target": 3, "rows": { "start": 2, "count": 5 } } },
                    { "set-props": [5, { "button": {
                        "label": "Go", "icon": "play", "style": "filled", "disabled": false,
                        "tooltip": "Run it"
                    } }] },
                ]
            })
        );
        assert_eq!(
            serde_json::from_value::<Update>(json).expect("parses"),
            update
        );

        let surface = Surface {
            id: "picker".into(),
            title: "Pick".into(),
            kind: SurfaceKind::Modal(ModalDescriptor {
                width: ModalWidth::Medium,
            }),
        };
        assert_eq!(
            serde_json::to_value(&surface).expect("serializes"),
            serde_json::json!({
                "id": "picker", "title": "Pick", "kind": { "modal": { "width": "medium" } }
            })
        );
        assert_eq!(
            serde_json::to_value(UiEvent::Dismissed(DismissReason::FocusLost)).expect("serializes"),
            serde_json::json!({ "dismissed": "focus-lost" })
        );
        let location = DecorationLocation {
            path: "src/main.rs".into(),
            line: 7,
            placement: DecorationPlacement::Below,
            height_lines: None,
        };
        assert_eq!(
            serde_json::to_value(&location).expect("serializes"),
            serde_json::json!({
                "path": "src/main.rs", "line": 7, "placement": "below", "height-lines": null
            })
        );
        let row = ListRow {
            index: 1,
            indent: 2,
            nodes: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&row).expect("serializes"),
            serde_json::json!({ "index": 1, "indent": 2, "nodes": [] })
        );
    }

    #[test]
    fn v0_1_values_convert_with_empty_v0_2_fields() {
        let button = Props::from(v0_1::Props::Button(v0_1::ButtonProps {
            label: "Refresh".into(),
            icon: Some("rotate_cw".into()),
            style: ButtonStyle::Subtle,
            disabled: true,
        }));
        assert_eq!(
            button,
            Props::Button(ButtonProps {
                label: "Refresh".into(),
                icon: Some(IconRef::Upstream("rotate_cw".into())),
                style: ButtonStyle::Subtle,
                disabled: true,
                tooltip: None,
                accessible_label: None,
            })
        );
        let surface = Surface::from(v0_1::Surface {
            id: "Counter Panel".into(),
            title: "Counter".into(),
            kind: v0_1::SurfaceKind::Panel(v0_1::PanelDescriptor {
                position: DockPosition::Left,
                icon: "hash".into(),
                default_size: 240,
            }),
        });
        assert_eq!(
            surface.kind,
            SurfaceKind::Panel(PanelDescriptor {
                position: DockPosition::Left,
                icon: IconRef::Upstream("hash".into()),
                default_size: 240,
            })
        );
        assert_eq!(surface.id, "Counter Panel");
        let row = ListRow::from(v0_1::ListRow {
            index: 3,
            nodes: Vec::new(),
        });
        assert_eq!(row.indent, 0);
    }

    #[test]
    fn only_v0_1_events_convert_back() {
        let event = UiEvent::from(v0_1::UiEvent::RowClick(4, 2));
        assert_eq!(event.target(), Some(4));
        assert_eq!(
            v0_1::UiEvent::try_from(event),
            Ok(v0_1::UiEvent::RowClick(4, 2))
        );
        for event in [
            UiEvent::RowSelected(4, 2),
            UiEvent::Dismissed(DismissReason::Escape),
        ] {
            let error = v0_1::UiEvent::try_from(event).unwrap_err();
            assert_eq!(error.code(), HostErrorCode::InvalidArgument);
        }
        assert_eq!(UiEvent::Dismissed(DismissReason::Replaced).target(), None);
    }

    #[test]
    fn surface_kinds_name_their_capability() {
        let modal = SurfaceKind::Modal(ModalDescriptor {
            width: ModalWidth::Small,
        });
        assert_eq!(modal.capability(), "ui:modal");
        let status = SurfaceKind::StatusItem(StatusItemDescriptor {
            side: StatusBarSide::Left,
            priority: 0,
        });
        assert_eq!(status.capability(), "ui:status-bar");
        let panel = SurfaceKind::Panel(PanelDescriptor {
            position: DockPosition::Bottom,
            icon: Icon::Terminal.into(),
            default_size: 200,
        });
        assert_eq!(panel.capability(), "ui:panel");
    }
}
