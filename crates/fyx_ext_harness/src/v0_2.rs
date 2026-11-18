//! The `fyx:ui@0.2.0` world (`crates/ui_ext/wit/since_v0.2.0`): its bindings, the host imports,
//! and the conversions to and from `ui_ext::v0_2`.

use ui_ext::v0_2 as model;

use crate::{State, props_text_len, update_text_len};

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit/since_v0.2.0",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
    });
}

pub(crate) use bindings::UiExtension;
use bindings::fyx::ui::{
    decorations as wit_decorations, host as wit_host, notifications as wit_notifications,
    types as wit,
};

impl wit::Host for State {}

impl wit_host::Host for State {
    fn push_update(
        &mut self,
        surface_id: String,
        update: wit::Update,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        let update: model::Update = update.into();
        self.count_lifted_text(surface_id.len() + update_text_len(&update))?;
        Ok(self.api.push_update(surface_id, update).map_err(Into::into))
    }

    fn request_tick(
        &mut self,
        surface_id: String,
        delay_ms: u32,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        Ok(self
            .api
            .request_tick(surface_id, delay_ms)
            .map_err(Into::into))
    }

    fn cancel_tick(&mut self, surface_id: String) -> wasmtime::Result<()> {
        self.count_lifted_text(surface_id.len())?;
        self.api.cancel_tick(surface_id);
        Ok(())
    }

    fn add_surface(
        &mut self,
        surface: wit::Surface,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface.id.len() + surface.title.len())?;
        Ok(self.api.add_surface(surface.into()).map_err(Into::into))
    }

    fn remove_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        Ok(self.api.remove_surface(surface_id).map_err(Into::into))
    }

    fn show_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        Ok(self.api.show_surface(surface_id).map_err(Into::into))
    }

    fn hide_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        Ok(self.api.hide_surface(surface_id).map_err(Into::into))
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) -> wasmtime::Result<()> {
        let level = match level {
            wit_host::LogLevel::Trace => ui_ext::LogLevel::Trace,
            wit_host::LogLevel::Debug => ui_ext::LogLevel::Debug,
            wit_host::LogLevel::Info => ui_ext::LogLevel::Info,
            wit_host::LogLevel::Warn => ui_ext::LogLevel::Warn,
            wit_host::LogLevel::Error => ui_ext::LogLevel::Error,
        };
        self.write_log(level, message)
    }
}

crate::notifications::notification_imports!();

impl wit_decorations::Host for State {
    fn place_decoration(
        &mut self,
        id: String,
        location: wit::DecorationLocation,
        nodes: Vec<wit::Node>,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        let nodes: Vec<model::Node> = nodes.into_iter().map(Into::into).collect();
        self.count_lifted_text(
            id.len()
                + location.path.len()
                + nodes
                    .iter()
                    .map(|node| props_text_len(&node.props))
                    .sum::<usize>(),
        )?;
        Ok(self
            .api
            .place_decoration(id, location.into(), nodes)
            .map_err(Into::into))
    }

    fn remove_decoration(
        &mut self,
        id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(id.len())?;
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
                // 0.2 has no `denied` or `untrusted-worktree`: the host maps them to
                // `not-permitted`, and `no-worktree` to `unavailable`.
                model::HostErrorCode::Denied | model::HostErrorCode::UntrustedWorktree => {
                    wit_host::HostErrorCode::NotPermitted
                }
                model::HostErrorCode::NoWorktree => wit_host::HostErrorCode::Unavailable,
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
    fn from(color: wit::ColorToken) -> Self {
        match color {
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
    fn from(props: wit::Props) -> Self {
        match props {
            wit::Props::Flex(flex) => Self::Flex(model::FlexProps {
                direction: match flex.direction {
                    wit::FlexDirection::Row => model::FlexDirection::Row,
                    wit::FlexDirection::Column => model::FlexDirection::Column,
                },
                gap: flex.gap,
                padding: flex.padding,
                grow: flex.grow,
                align: match flex.align {
                    wit::Align::Start => model::Align::Start,
                    wit::Align::Center => model::Align::Center,
                    wit::Align::End => model::Align::End,
                    wit::Align::Stretch => model::Align::Stretch,
                },
                background: flex.background.map(Into::into),
            }),
            wit::Props::Text(text) => Self::Text(model::TextProps {
                content: text.content,
                color: text.color.map(Into::into),
                bold: text.bold,
                monospace: text.monospace,
            }),
            wit::Props::Button(button) => Self::Button(model::ButtonProps {
                label: button.label,
                icon: button.icon.map(|icon| model::IconRef::Frozen(icon.into())),
                style: match button.style {
                    wit::ButtonStyle::Default => model::ButtonStyle::Default,
                    wit::ButtonStyle::Filled => model::ButtonStyle::Filled,
                    wit::ButtonStyle::Subtle => model::ButtonStyle::Subtle,
                    wit::ButtonStyle::Danger => model::ButtonStyle::Danger,
                },
                disabled: button.disabled,
                tooltip: button.tooltip,
                accessible_label: button.accessible_label,
            }),
            wit::Props::Input(input) => Self::Input(model::InputProps {
                value: input.value,
                placeholder: input.placeholder,
                secret: input.secret,
                label: input.label,
            }),
            wit::Props::List(list) => Self::List(model::ListProps {
                row_count: list.row_count,
                row_height: list.row_height,
                selected: list.selected,
            }),
            wit::Props::Table(table) => Self::Table(model::TableProps {
                columns: table.columns,
                row_count: table.row_count,
            }),
            wit::Props::Icon(icon) => Self::Icon(model::IconProps {
                name: model::IconRef::Frozen(icon.name.into()),
                color: icon.color.map(Into::into),
            }),
            wit::Props::Markdown(source) => Self::Markdown(source),
            wit::Props::DiffView(diff) => Self::DiffView(model::DiffViewProps {
                old_text: diff.old_text,
                new_text: diff.new_text,
                language: diff.language,
            }),
        }
    }
}

impl From<wit::Node> for model::Node {
    fn from(node: wit::Node) -> Self {
        Self {
            id: node.id,
            parent: node.parent,
            props: node.props.into(),
        }
    }
}

impl From<wit::Patch> for model::Patch {
    fn from(patch: wit::Patch) -> Self {
        match patch {
            wit::Patch::SetProps((id, props)) => Self::SetProps(id, props.into()),
            wit::Patch::Insert(insert) => Self::Insert(model::InsertOp {
                node: insert.node.into(),
                before: insert.before,
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
    fn from(update: wit::Update) -> Self {
        match update {
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
    fn from(row: wit::ListRow) -> Self {
        Self {
            index: row.index,
            indent: row.indent,
            nodes: row.nodes.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<wit::Surface> for model::Surface {
    fn from(surface: wit::Surface) -> Self {
        Self {
            id: surface.id,
            title: surface.title,
            kind: match surface.kind {
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
    fn from(location: wit::DecorationLocation) -> Self {
        Self {
            path: location.path,
            line: location.line,
            placement: match location.placement {
                wit::DecorationPlacement::Above => model::DecorationPlacement::Above,
                wit::DecorationPlacement::Below => model::DecorationPlacement::Below,
            },
            height_lines: location.height_lines,
        }
    }
}

impl From<model::UiEvent> for wit::UiEvent {
    fn from(event: model::UiEvent) -> Self {
        match event {
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
            model::UiEvent::NotificationAction(event) => {
                Self::NotificationAction(wit::NotificationActionEvent {
                    notification: event.notification,
                    action: event.action,
                })
            }
            model::UiEvent::NotificationClosed(id, reason) => Self::NotificationClosed((
                id,
                match reason {
                    model::NotificationCloseReason::User => wit::NotificationCloseReason::User,
                    model::NotificationCloseReason::Expired => {
                        wit::NotificationCloseReason::Expired
                    }
                    model::NotificationCloseReason::WindowClosed => {
                        wit::NotificationCloseReason::WindowClosed
                    }
                },
            )),
            model::UiEvent::ProgressCancelRequested(id) => Self::ProgressCancelRequested(id),
            // 0.2 has no canvas; the host maps the event to a click on the node, but never sends
            // it, since a 0.2 tree cannot hold a canvas.
            model::UiEvent::CanvasClick(click) => Self::Click(click.canvas),
        }
    }
}
