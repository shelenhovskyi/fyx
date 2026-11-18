//! Host side of the `fyx:ui@0.2.0` world (`crates/ui_ext/wit/since_v0.2.0/fyx-ui.wit`): its
//! bindings, the conversions into `ui_ext::v0_2`, and the 0.2 host imports.
//!
//! Every import with an effect is checked when it is called (design.md, A) against the state in
//! [`ApiState`], and its effect is recorded there, to be returned with the call's reply.

use crate::UiState;
use ui_ext::v0_2 as model;

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit/since_v0.2.0",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
        exports: {
            default: async,
        },
    });
}

pub(crate) use bindings::UiExtension;
use bindings::fyx::ui::{decorations as wit_decorations, host as wit_host, types as wit};

impl wit::Host for UiState {}

impl wit_host::Host for UiState {
    fn push_update(
        &mut self,
        surface_id: String,
        update: wit::Update,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self
            .api
            .push_update(surface_id, update.into())
            .map_err(Into::into))
    }

    fn request_tick(
        &mut self,
        surface_id: String,
        delay_ms: u32,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self
            .api
            .request_tick(surface_id, delay_ms)
            .map_err(Into::into))
    }

    fn cancel_tick(&mut self, surface_id: String) -> wasmtime::Result<()> {
        self.api.cancel_tick(surface_id);
        Ok(())
    }

    fn add_surface(
        &mut self,
        surface: wit::Surface,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.api.add_surface(surface.into()).map_err(Into::into))
    }

    fn remove_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.api.remove_surface(surface_id).map_err(Into::into))
    }

    fn show_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.api.show_surface(surface_id).map_err(Into::into))
    }

    fn hide_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.api.hide_surface(surface_id).map_err(Into::into))
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) -> wasmtime::Result<()> {
        let level = match level {
            wit_host::LogLevel::Trace => log::Level::Trace,
            wit_host::LogLevel::Debug => log::Level::Debug,
            wit_host::LogLevel::Info => log::Level::Info,
            wit_host::LogLevel::Warn => log::Level::Warn,
            wit_host::LogLevel::Error => log::Level::Error,
        };
        log::log!(target: "fyx_ui", level, "{}: {message}", self.extension_id);
        Ok(())
    }
}

impl wit_decorations::Host for UiState {
    fn place_decoration(
        &mut self,
        id: String,
        location: wit::DecorationLocation,
        nodes: Vec<wit::Node>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        let nodes = nodes.into_iter().map(Into::into).collect();
        Ok(self
            .api
            .place_decoration(id, location.into(), nodes)
            .map_err(Into::into))
    }

    fn remove_decoration(
        &mut self,
        id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        Ok(self.api.remove_decoration(id).map_err(Into::into))
    }
}

impl From<model::HostError> for wit_host::HostError {
    fn from(error: model::HostError) -> Self {
        let model::HostError { code, message } = error;
        Self {
            code: match code {
                model::HostErrorCode::UnknownSurface => wit_host::HostErrorCode::UnknownSurface,
                model::HostErrorCode::LimitExceeded => wit_host::HostErrorCode::LimitExceeded,
                model::HostErrorCode::InvalidArgument => wit_host::HostErrorCode::InvalidArgument,
                model::HostErrorCode::NotPermitted => wit_host::HostErrorCode::NotPermitted,
                model::HostErrorCode::Unavailable => wit_host::HostErrorCode::Unavailable,
            },
            message,
        }
    }
}

macro_rules! icons {
    ($($variant:ident)*) => {
        /// One arm per frozen icon, so a case added to either enum without the other fails to
        /// compile.
        impl From<wit::Icon> for model::Icon {
            fn from(icon: wit::Icon) -> Self {
                match icon {
                    $(wit::Icon::$variant => model::Icon::$variant,)*
                }
            }
        }

        #[cfg(test)]
        pub(crate) const WIT_ICONS: &[wit::Icon] = &[$(wit::Icon::$variant,)*];
    };
}

icons! {
    ArrowUp ArrowDown ArrowLeft ArrowRight ChevronUp ChevronDown ChevronLeft ChevronRight Expand
    Collapse Menu Ellipsis ListTree ListTodo Table FileTree Plus Dash Check CheckDouble Close Search
    Filter Refresh Undo Copy Pencil Trash Replace Download Send Share Link Pin Unpin Play Pause Stop
    Maximize Minimize Split File FileCode FileDiff FileText Folder FolderOpen Code Terminal Diff Regex
    CaseSensitive WholeWord Hash AtSign GitBranch GitCommit PullRequest Info Warning Error Help Bell
    BellDot Circle Dot Star StarFilled Bookmark ThumbsUp ThumbsDown TodoPending TodoProgress
    TodoComplete Spinner Blank Settings Clock Debug Eye EyeOff Lock Unlock Person People Chat
    Envelope Book Notepad Image Globe Server Database Keyboard Box Archive Sparkle Bolt Flame Power
    Blocks
}

impl From<wit::ColorToken> for model::ColorToken {
    fn from(value: wit::ColorToken) -> Self {
        match value {
            wit::ColorToken::Text => Self::Text,
            wit::ColorToken::TextMuted => Self::TextMuted,
            wit::ColorToken::TextAccent => Self::TextAccent,
            wit::ColorToken::Error => Self::Error,
            wit::ColorToken::Warning => Self::Warning,
            wit::ColorToken::Success => Self::Success,
            wit::ColorToken::Border => Self::Border,
            wit::ColorToken::Surface => Self::Surface,
            wit::ColorToken::ElevatedSurface => Self::ElevatedSurface,
        }
    }
}

impl From<wit::Props> for model::Props {
    fn from(value: wit::Props) -> Self {
        match value {
            wit::Props::Flex(props) => Self::Flex(model::FlexProps {
                direction: match props.direction {
                    wit::FlexDirection::Row => model::FlexDirection::Row,
                    wit::FlexDirection::Column => model::FlexDirection::Column,
                },
                gap: props.gap,
                padding: props.padding,
                grow: props.grow,
                align: match props.align {
                    wit::Align::Start => model::Align::Start,
                    wit::Align::Center => model::Align::Center,
                    wit::Align::End => model::Align::End,
                    wit::Align::Stretch => model::Align::Stretch,
                },
                background: props.background.map(Into::into),
            }),
            wit::Props::Text(props) => Self::Text(model::TextProps {
                content: props.content,
                color: props.color.map(Into::into),
                bold: props.bold,
                monospace: props.monospace,
            }),
            wit::Props::Button(props) => Self::Button(model::ButtonProps {
                label: props.label,
                icon: props.icon.map(|icon| model::IconRef::Frozen(icon.into())),
                style: match props.style {
                    wit::ButtonStyle::Default => model::ButtonStyle::Default,
                    wit::ButtonStyle::Filled => model::ButtonStyle::Filled,
                    wit::ButtonStyle::Subtle => model::ButtonStyle::Subtle,
                    wit::ButtonStyle::Danger => model::ButtonStyle::Danger,
                },
                disabled: props.disabled,
                tooltip: props.tooltip,
            }),
            wit::Props::Input(props) => Self::Input(model::InputProps {
                value: props.value,
                placeholder: props.placeholder,
                secret: props.secret,
            }),
            wit::Props::List(props) => Self::List(model::ListProps {
                row_count: props.row_count,
                row_height: props.row_height,
                selected: props.selected,
            }),
            wit::Props::Table(props) => Self::Table(model::TableProps {
                columns: props.columns,
                row_count: props.row_count,
            }),
            wit::Props::Icon(props) => Self::Icon(model::IconProps {
                name: model::IconRef::Frozen(props.name.into()),
                color: props.color.map(Into::into),
            }),
            wit::Props::Markdown(text) => Self::Markdown(text),
            wit::Props::DiffView(props) => Self::DiffView(model::DiffViewProps {
                old_text: props.old_text,
                new_text: props.new_text,
                language: props.language,
            }),
        }
    }
}

impl From<wit::Node> for model::Node {
    fn from(value: wit::Node) -> Self {
        Self {
            id: value.id,
            parent: value.parent,
            props: value.props.into(),
        }
    }
}

impl From<wit::Patch> for model::Patch {
    fn from(value: wit::Patch) -> Self {
        match value {
            wit::Patch::SetProps((id, props)) => Self::SetProps(id, props.into()),
            wit::Patch::Insert(op) => Self::Insert(model::InsertOp {
                node: op.node.into(),
                before: op.before,
            }),
            wit::Patch::Remove(id) => Self::Remove(id),
            wit::Patch::InvalidateRows(op) => Self::InvalidateRows(model::InvalidateRowsOp {
                target: op.target,
                rows: op.rows.map(|rows| model::RowRange {
                    start: rows.start,
                    count: rows.count,
                }),
            }),
        }
    }
}

impl From<wit::Update> for model::Update {
    fn from(value: wit::Update) -> Self {
        match value {
            wit::Update::Unchanged => Self::Unchanged,
            wit::Update::Replace(nodes) => {
                Self::Replace(nodes.into_iter().map(Into::into).collect())
            }
            wit::Update::Patches(patches) => {
                Self::Patches(patches.into_iter().map(Into::into).collect())
            }
        }
    }
}

impl From<wit::ListRow> for model::ListRow {
    fn from(value: wit::ListRow) -> Self {
        Self {
            index: value.index,
            indent: value.indent,
            nodes: value.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<wit::Surface> for model::Surface {
    fn from(value: wit::Surface) -> Self {
        Self {
            id: value.id,
            title: value.title,
            kind: match value.kind {
                wit::SurfaceKind::Panel(panel) => {
                    model::SurfaceKind::Panel(model::PanelDescriptor {
                        position: match panel.position {
                            wit::DockPosition::Left => model::DockPosition::Left,
                            wit::DockPosition::Right => model::DockPosition::Right,
                            wit::DockPosition::Bottom => model::DockPosition::Bottom,
                        },
                        icon: model::IconRef::Frozen(panel.icon.into()),
                        default_size: panel.default_size,
                    })
                }
                wit::SurfaceKind::StatusItem(item) => {
                    model::SurfaceKind::StatusItem(model::StatusItemDescriptor {
                        side: match item.side {
                            wit::StatusBarSide::Left => model::StatusBarSide::Left,
                            wit::StatusBarSide::Right => model::StatusBarSide::Right,
                        },
                        priority: item.priority,
                    })
                }
                wit::SurfaceKind::Modal(modal) => {
                    model::SurfaceKind::Modal(model::ModalDescriptor {
                        width: match modal.width {
                            wit::ModalWidth::Small => model::ModalWidth::Small,
                            wit::ModalWidth::Medium => model::ModalWidth::Medium,
                            wit::ModalWidth::Large => model::ModalWidth::Large,
                        },
                    })
                }
            },
        }
    }
}

impl From<wit::DecorationLocation> for model::DecorationLocation {
    fn from(value: wit::DecorationLocation) -> Self {
        Self {
            path: value.path,
            line: value.line,
            placement: match value.placement {
                wit::DecorationPlacement::Above => model::DecorationPlacement::Above,
                wit::DecorationPlacement::Below => model::DecorationPlacement::Below,
            },
            height_lines: value.height_lines,
        }
    }
}

impl From<model::UiEvent> for wit::UiEvent {
    fn from(value: model::UiEvent) -> Self {
        match value {
            model::UiEvent::Click(id) => Self::Click(id),
            model::UiEvent::InputChanged(id, text) => Self::InputChanged((id, text)),
            model::UiEvent::InputSubmitted(id, text) => Self::InputSubmitted((id, text)),
            model::UiEvent::RowClick(id, row) => Self::RowClick((id, row)),
            model::UiEvent::RowSelected(id, row) => Self::RowSelected((id, row)),
            model::UiEvent::Dismissed(reason) => Self::Dismissed(match reason {
                model::DismissReason::Escape => wit::DismissReason::Escape,
                model::DismissReason::FocusLost => wit::DismissReason::FocusLost,
                model::DismissReason::Replaced => wit::DismissReason::Replaced,
                model::DismissReason::WindowClosed => wit::DismissReason::WindowClosed,
            }),
        }
    }
}
