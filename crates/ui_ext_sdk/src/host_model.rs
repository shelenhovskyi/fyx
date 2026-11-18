//! Conversions from the guest bindings to `ui_ext`'s plain model, so the SDK checks trees with the
//! host's own rules (`ui_ext::Props::is_container`, `ui_ext::Tree`) rather than a copy of them.
//!
//! The target is the 0.1 model, which `ui_ext::Tree` validates. The structure (ids, parents,
//! kinds) carries over exactly, which is all those rules look at; 0.2-only fields are dropped,
//! icons become upstream names, and `invalidate-rows`, which changes no node, is left out.

use crate::{
    Align, ButtonProps, ButtonStyle, ColorToken, DiffViewProps, FlexDirection, FlexProps,
    IconProps, InputProps, InsertOp, ListProps, ListRow, Node, Patch, Props, TableProps, TextProps,
    Update,
};

impl From<ColorToken> for ui_ext::ColorToken {
    fn from(color: ColorToken) -> Self {
        match color {
            ColorToken::Text => Self::Text,
            ColorToken::TextMuted => Self::TextMuted,
            ColorToken::TextAccent => Self::TextAccent,
            ColorToken::Error => Self::Error,
            ColorToken::Warning => Self::Warning,
            ColorToken::Success => Self::Success,
            ColorToken::Border => Self::Border,
            ColorToken::Surface => Self::Surface,
            ColorToken::ElevatedSurface => Self::ElevatedSurface,
        }
    }
}

impl From<FlexDirection> for ui_ext::FlexDirection {
    fn from(direction: FlexDirection) -> Self {
        match direction {
            FlexDirection::Row => Self::Row,
            FlexDirection::Column => Self::Column,
        }
    }
}

impl From<Align> for ui_ext::Align {
    fn from(align: Align) -> Self {
        match align {
            Align::Start => Self::Start,
            Align::Center => Self::Center,
            Align::End => Self::End,
            Align::Stretch => Self::Stretch,
        }
    }
}

impl From<ButtonStyle> for ui_ext::ButtonStyle {
    fn from(style: ButtonStyle) -> Self {
        match style {
            ButtonStyle::Default => Self::Default,
            ButtonStyle::Filled => Self::Filled,
            ButtonStyle::Subtle => Self::Subtle,
            ButtonStyle::Danger => Self::Danger,
        }
    }
}

impl From<&FlexProps> for ui_ext::FlexProps {
    fn from(props: &FlexProps) -> Self {
        Self {
            direction: props.direction.into(),
            gap: props.gap,
            padding: props.padding,
            grow: props.grow,
            align: props.align.into(),
            background: props.background.map(Into::into),
        }
    }
}

impl From<&TextProps> for ui_ext::TextProps {
    fn from(props: &TextProps) -> Self {
        Self {
            content: props.content.clone(),
            color: props.color.map(Into::into),
            bold: props.bold,
            monospace: props.monospace,
        }
    }
}

impl From<&ButtonProps> for ui_ext::ButtonProps {
    fn from(props: &ButtonProps) -> Self {
        Self {
            label: props.label.clone(),
            icon: props.icon.and_then(crate::v0_1::upstream_icon_name),
            style: props.style.into(),
            disabled: props.disabled,
        }
    }
}

impl From<&InputProps> for ui_ext::InputProps {
    fn from(props: &InputProps) -> Self {
        Self {
            value: props.value.clone(),
            placeholder: props.placeholder.clone(),
        }
    }
}

impl From<&ListProps> for ui_ext::ListProps {
    fn from(props: &ListProps) -> Self {
        Self {
            row_count: props.row_count,
            row_height: props.row_height,
        }
    }
}

impl From<&TableProps> for ui_ext::TableProps {
    fn from(props: &TableProps) -> Self {
        Self {
            columns: props.columns.clone(),
            row_count: props.row_count,
        }
    }
}

impl From<&IconProps> for ui_ext::IconProps {
    fn from(props: &IconProps) -> Self {
        Self {
            name: crate::v0_1::upstream_icon_name(props.name).unwrap_or_default(),
            color: props.color.map(Into::into),
        }
    }
}

impl From<&DiffViewProps> for ui_ext::DiffViewProps {
    fn from(props: &DiffViewProps) -> Self {
        Self {
            old_text: props.old_text.clone(),
            new_text: props.new_text.clone(),
            language: props.language.clone(),
        }
    }
}

impl From<&Props> for ui_ext::Props {
    fn from(props: &Props) -> Self {
        match props {
            Props::Flex(props) => Self::Flex(props.into()),
            Props::Text(props) => Self::Text(props.into()),
            Props::Button(props) => Self::Button(props.into()),
            Props::Input(props) => Self::Input(props.into()),
            Props::List(props) => Self::List(props.into()),
            Props::Table(props) => Self::Table(props.into()),
            Props::Icon(props) => Self::Icon(props.into()),
            Props::Markdown(source) => Self::Markdown(source.clone()),
            Props::DiffView(props) => Self::DiffView(props.into()),
            // The 0.1 model has no image or canvas yet. Both are leaves, which is all the tree
            // rules need, so a text node with their alt text stands in for them.
            Props::Image(image) => Self::Text(ui_ext::TextProps {
                content: image.alt.clone(),
                color: None,
                bold: false,
                monospace: false,
            }),
            Props::Canvas(canvas) => Self::Text(ui_ext::TextProps {
                content: canvas.description.clone(),
                color: None,
                bold: false,
                monospace: false,
            }),
        }
    }
}

impl From<&Node> for ui_ext::Node {
    fn from(node: &Node) -> Self {
        Self {
            id: node.id,
            parent: node.parent,
            props: (&node.props).into(),
        }
    }
}

/// `None` for `invalidate-rows`, which changes no node of the tree.
fn tree_patch(patch: &Patch) -> Option<ui_ext::Patch> {
    Some(match patch {
        Patch::SetProps((id, props)) => ui_ext::Patch::SetProps(*id, props.into()),
        Patch::Insert(InsertOp { node, before }) => ui_ext::Patch::Insert(ui_ext::InsertOp {
            node: node.into(),
            before: *before,
        }),
        Patch::Remove(id) => ui_ext::Patch::Remove(*id),
        Patch::InvalidateRows(_) => return None,
    })
}

impl From<&Update> for ui_ext::Update {
    fn from(update: &Update) -> Self {
        match update {
            Update::Unchanged => Self::Unchanged,
            Update::Replace(nodes) => Self::Replace(nodes.iter().map(Into::into).collect()),
            Update::Patches(patches) => {
                Self::Patches(patches.iter().filter_map(tree_patch).collect())
            }
        }
    }
}

impl From<&ListRow> for ui_ext::ListRow {
    fn from(row: &ListRow) -> Self {
        Self {
            index: row.index,
            nodes: row.nodes.iter().map(Into::into).collect(),
        }
    }
}
