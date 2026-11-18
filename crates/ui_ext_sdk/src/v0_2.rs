//! `register_extension!(T, v0_2)`: an extension written against the SDK's types, exported as
//! `fyx:ui@0.2.0`.
//!
//! 0.2 is 0.3 minus additions, so almost every value maps one to one. What 0.2 cannot express
//! is an error that names 0.3, as `v0_1` does for 0.2:
//! - image and canvas nodes fail the reply;
//! - picker surfaces are left out of `surfaces()` and logged;
//! - a decoration anchored to a document instead of a path, and `add_image`, return
//!   `unavailable`.
//!
//! The host never sends a 0.2 component the 0.3-only events (`orphaned`, `canvas-click`).

use crate::bindings_v0_2::fyx::ui::types as v2;
use crate::{
    Align, ButtonStyle, ColorToken, DecorationLocation, DecorationPlacement, DecorationTarget,
    DismissReason, FlexDirection, HostError, HostErrorCode, Icon, ListRow, LogLevel, ModalWidth,
    Node, Patch, Props, Surface, SurfaceKind, UiEvent, Update,
};

fn needs_0_3(what: &str) -> String {
    format!("{what} needs fyx:ui 0.3; this extension is exported as 0.2")
}

macro_rules! icons {
    ($($variant:ident)*) => {
        /// One arm per frozen icon, so a case added to either world without the other fails to
        /// compile.
        fn icon(icon: Icon) -> v2::Icon {
            match icon {
                $(Icon::$variant => v2::Icon::$variant,)*
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

fn color(color: ColorToken) -> v2::ColorToken {
    match color {
        ColorToken::Text => v2::ColorToken::Text,
        ColorToken::TextMuted => v2::ColorToken::TextMuted,
        ColorToken::TextAccent => v2::ColorToken::TextAccent,
        ColorToken::Error => v2::ColorToken::Error,
        ColorToken::Warning => v2::ColorToken::Warning,
        ColorToken::Success => v2::ColorToken::Success,
        ColorToken::Border => v2::ColorToken::Border,
        ColorToken::Surface => v2::ColorToken::Surface,
        ColorToken::ElevatedSurface => v2::ColorToken::ElevatedSurface,
    }
}

fn props(props: &Props) -> Result<v2::Props, String> {
    Ok(match props {
        Props::Flex(flex) => v2::Props::Flex(v2::FlexProps {
            direction: match flex.direction {
                FlexDirection::Row => v2::FlexDirection::Row,
                FlexDirection::Column => v2::FlexDirection::Column,
            },
            gap: flex.gap,
            padding: flex.padding,
            grow: flex.grow,
            align: match flex.align {
                Align::Start => v2::Align::Start,
                Align::Center => v2::Align::Center,
                Align::End => v2::Align::End,
                Align::Stretch => v2::Align::Stretch,
            },
            background: flex.background.map(color),
        }),
        Props::Text(text) => v2::Props::Text(v2::TextProps {
            content: text.content.clone(),
            color: text.color.map(color),
            bold: text.bold,
            monospace: text.monospace,
        }),
        Props::Button(button) => v2::Props::Button(v2::ButtonProps {
            label: button.label.clone(),
            icon: button.icon.map(icon),
            style: match button.style {
                ButtonStyle::Default => v2::ButtonStyle::Default,
                ButtonStyle::Filled => v2::ButtonStyle::Filled,
                ButtonStyle::Subtle => v2::ButtonStyle::Subtle,
                ButtonStyle::Danger => v2::ButtonStyle::Danger,
            },
            disabled: button.disabled,
            tooltip: button.tooltip.clone(),
        }),
        Props::Input(input) => v2::Props::Input(v2::InputProps {
            value: input.value.clone(),
            placeholder: input.placeholder.clone(),
            secret: input.secret,
            label: input.label.clone(),
        }),
        Props::List(list) => v2::Props::List(v2::ListProps {
            row_count: list.row_count,
            row_height: list.row_height,
            selected: list.selected,
        }),
        Props::Table(table) => v2::Props::Table(v2::TableProps {
            columns: table.columns.clone(),
            row_count: table.row_count,
        }),
        Props::Icon(props) => v2::Props::Icon(v2::IconProps {
            name: icon(props.name),
            color: props.color.map(color),
        }),
        Props::Markdown(source) => v2::Props::Markdown(source.clone()),
        Props::DiffView(diff) => v2::Props::DiffView(v2::DiffViewProps {
            old_text: diff.old_text.clone(),
            new_text: diff.new_text.clone(),
            language: diff.language.clone(),
        }),
        Props::Image(_) => return Err(needs_0_3("an image node")),
        Props::Canvas(_) => return Err(needs_0_3("a canvas node")),
    })
}

fn node(node: &Node) -> Result<v2::Node, String> {
    Ok(v2::Node {
        id: node.id,
        parent: node.parent,
        props: props(&node.props)?,
    })
}

fn modal_width(width: ModalWidth) -> v2::ModalWidth {
    match width {
        ModalWidth::Small => v2::ModalWidth::Small,
        ModalWidth::Medium => v2::ModalWidth::Medium,
        ModalWidth::Large => v2::ModalWidth::Large,
    }
}

/// `Err` for a surface kind 0.2 does not have.
fn surface(surface: &Surface) -> Result<v2::Surface, String> {
    let kind = match &surface.kind {
        SurfaceKind::Panel(panel) => v2::SurfaceKind::Panel(v2::PanelDescriptor {
            position: match panel.position {
                crate::DockPosition::Left => v2::DockPosition::Left,
                crate::DockPosition::Right => v2::DockPosition::Right,
                crate::DockPosition::Bottom => v2::DockPosition::Bottom,
            },
            icon: icon(panel.icon),
            default_size: panel.default_size,
        }),
        SurfaceKind::StatusItem(item) => v2::SurfaceKind::StatusItem(v2::StatusItemDescriptor {
            side: match item.side {
                crate::StatusBarSide::Left => v2::StatusBarSide::Left,
                crate::StatusBarSide::Right => v2::StatusBarSide::Right,
            },
            priority: item.priority,
        }),
        SurfaceKind::Modal(modal) => v2::SurfaceKind::Modal(v2::ModalDescriptor {
            width: modal_width(modal.width),
        }),
        SurfaceKind::Picker(_) => {
            return Err(needs_0_3(&format!("picker surface {:?}", surface.id)));
        }
    };
    Ok(v2::Surface {
        id: surface.id.clone(),
        title: surface.title.clone(),
        kind,
    })
}

fn location(location: &DecorationLocation) -> Result<v2::DecorationLocation, String> {
    let path = match &location.target {
        DecorationTarget::Path(path) => path.clone(),
        DecorationTarget::Document(_) => {
            return Err(needs_0_3("a decoration anchored to a document"));
        }
    };
    Ok(v2::DecorationLocation {
        path,
        line: location.line,
        placement: match location.placement {
            DecorationPlacement::Above => v2::DecorationPlacement::Above,
            DecorationPlacement::Below => v2::DecorationPlacement::Below,
        },
        height_lines: location.height_lines,
    })
}

fn host_error(error: crate::bindings_v0_2::fyx::ui::host::HostError) -> HostError {
    use crate::bindings_v0_2::fyx::ui::host::HostErrorCode as V2Code;
    HostError {
        code: match error.code {
            V2Code::UnknownSurface => HostErrorCode::UnknownSurface,
            V2Code::LimitExceeded => HostErrorCode::LimitExceeded,
            V2Code::InvalidArgument => HostErrorCode::InvalidArgument,
            V2Code::NotPermitted => HostErrorCode::NotPermitted,
            V2Code::Unavailable => HostErrorCode::Unavailable,
        },
        message: error.message,
    }
}

mod v2n {
    pub use crate::bindings_v0_2::fyx::ui::notifications::*;
}

/// Section M is the same in 0.2 and 0.3; only the generated types differ.
pub(crate) fn notification(notification: &crate::Notification) -> v2n::Notification {
    v2n::Notification {
        owner: notification.owner.clone(),
        level: match notification.level {
            crate::NotificationLevel::Info => v2n::NotificationLevel::Info,
            crate::NotificationLevel::Success => v2n::NotificationLevel::Success,
            crate::NotificationLevel::Warning => v2n::NotificationLevel::Warning,
            crate::NotificationLevel::Error => v2n::NotificationLevel::Error,
        },
        title: notification.title.clone(),
        body: notification.body.clone(),
        actions: notification
            .actions
            .iter()
            .map(|action| v2n::NotificationAction {
                id: action.id.clone(),
                label: action.label.clone(),
            })
            .collect(),
        auto_dismiss: notification.auto_dismiss,
    }
}

pub(crate) fn progress_options(options: &crate::ProgressOptions) -> v2n::ProgressOptions {
    v2n::ProgressOptions {
        owner: options.owner.clone(),
        title: options.title.clone(),
        location: match options.location {
            crate::ProgressLocation::StatusBar => v2n::ProgressLocation::StatusBar,
            crate::ProgressLocation::Panel => v2n::ProgressLocation::Panel,
        },
        cancellable: options.cancellable,
    }
}

pub(crate) fn progress_value(value: crate::ProgressValue) -> v2n::ProgressValue {
    match value {
        crate::ProgressValue::Indeterminate => v2n::ProgressValue::Indeterminate,
        crate::ProgressValue::Percent(percent) => v2n::ProgressValue::Percent(percent),
    }
}

fn invalid_argument(message: String) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message,
    }
}

/// The SDK-side conversions and host shims `register_extension!(T, v0_2)` calls.
#[doc(hidden)]
pub mod glue {
    use super::*;

    pub const API_VERSION: (u16, u16, u16) = crate::API_VERSION_V0_2;

    pub fn nodes(nodes: &[Node]) -> Result<Vec<v2::Node>, String> {
        nodes.iter().map(node).collect()
    }

    pub fn update(update: &Update) -> Result<v2::Update, String> {
        Ok(match update {
            Update::Unchanged => v2::Update::Unchanged,
            Update::Replace(tree) => v2::Update::Replace(nodes(tree)?),
            Update::Patches(patches) => v2::Update::Patches(
                patches
                    .iter()
                    .map(|patch| {
                        Ok(match patch {
                            Patch::SetProps((id, new_props)) => {
                                v2::Patch::SetProps((*id, props(new_props)?))
                            }
                            Patch::Insert(insert) => v2::Patch::Insert(v2::InsertOp {
                                node: node(&insert.node)?,
                                before: insert.before,
                            }),
                            Patch::Remove(id) => v2::Patch::Remove(*id),
                            Patch::InvalidateRows(op) => {
                                v2::Patch::InvalidateRows(v2::InvalidateRowsOp {
                                    target: op.target,
                                    rows: op.rows.map(|rows| v2::RowRange {
                                        start: rows.start,
                                        count: rows.count,
                                    }),
                                })
                            }
                        })
                    })
                    .collect::<Result<_, String>>()?,
            ),
        })
    }

    /// Picker surfaces have no 0.2 form; they are left out, with an error in the log.
    pub fn surfaces(surfaces: Vec<Surface>) -> Vec<v2::Surface> {
        surfaces
            .iter()
            .filter_map(|declared| match surface(declared) {
                Ok(surface) => Some(surface),
                Err(error) => {
                    log(LogLevel::Error, &error);
                    None
                }
            })
            .collect()
    }

    /// A row 0.2 cannot draw is left out, with an error in the log.
    pub fn rows(rows: Vec<ListRow>) -> Vec<v2::ListRow> {
        rows.into_iter()
            .filter_map(|row| match nodes(&row.nodes) {
                Ok(nodes) => Some(v2::ListRow {
                    index: row.index,
                    indent: row.indent,
                    nodes,
                }),
                Err(error) => {
                    log(
                        LogLevel::Error,
                        &format!("list row {} left out: {error}", row.index),
                    );
                    None
                }
            })
            .collect()
    }

    pub fn event(event: v2::UiEvent) -> UiEvent {
        match event {
            v2::UiEvent::Click(id) => UiEvent::Click(id),
            v2::UiEvent::InputChanged(change) => UiEvent::InputChanged(change),
            v2::UiEvent::InputSubmitted(submit) => UiEvent::InputSubmitted(submit),
            v2::UiEvent::RowClick(click) => UiEvent::RowClick(click),
            v2::UiEvent::RowSelected(selected) => UiEvent::RowSelected(selected),
            v2::UiEvent::Dismissed(reason) => UiEvent::Dismissed(match reason {
                v2::DismissReason::Escape => DismissReason::Escape,
                v2::DismissReason::FocusLost => DismissReason::FocusLost,
                v2::DismissReason::Replaced => DismissReason::Replaced,
                v2::DismissReason::WindowClosed => DismissReason::WindowClosed,
            }),
            v2::UiEvent::NotificationAction(action) => {
                UiEvent::NotificationAction(crate::NotificationActionEvent {
                    notification: action.notification,
                    action: action.action,
                })
            }
            v2::UiEvent::NotificationClosed((notification, reason)) => {
                UiEvent::NotificationClosed((
                    notification,
                    match reason {
                        v2::NotificationCloseReason::User => crate::NotificationCloseReason::User,
                        v2::NotificationCloseReason::Expired => {
                            crate::NotificationCloseReason::Expired
                        }
                        v2::NotificationCloseReason::WindowClosed => {
                            crate::NotificationCloseReason::WindowClosed
                        }
                    },
                ))
            }
            v2::UiEvent::ProgressCancelRequested(progress) => {
                UiEvent::ProgressCancelRequested(progress)
            }
        }
    }

    pub fn log(level: LogLevel, message: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            use crate::bindings_v0_2::fyx::ui::host::{self, LogLevel as V2Level};
            let level = match level {
                LogLevel::Trace => V2Level::Trace,
                LogLevel::Debug => V2Level::Debug,
                LogLevel::Info => V2Level::Info,
                LogLevel::Warn => V2Level::Warn,
                LogLevel::Error => V2Level::Error,
            };
            host::log(level, message);
        }
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!("[{level:?}] {message}");
    }

    pub fn push_update(surface_id: &str, new_update: &Update) -> Result<(), HostError> {
        let new_update = update(new_update).map_err(invalid_argument)?;
        crate::bindings_v0_2::fyx::ui::host::push_update(surface_id, &new_update)
            .map_err(host_error)
    }

    pub fn request_tick(surface_id: &str, delay_ms: u32) -> Result<(), HostError> {
        crate::bindings_v0_2::fyx::ui::host::request_tick(surface_id, delay_ms).map_err(host_error)
    }

    pub fn cancel_tick(surface_id: &str) {
        crate::bindings_v0_2::fyx::ui::host::cancel_tick(surface_id);
    }

    pub fn add_surface(new_surface: &Surface) -> Result<(), HostError> {
        let new_surface = surface(new_surface).map_err(|message| HostError {
            code: HostErrorCode::Unavailable,
            message,
        })?;
        crate::bindings_v0_2::fyx::ui::host::add_surface(&new_surface).map_err(host_error)
    }

    pub fn remove_surface(surface_id: &str) -> Result<(), HostError> {
        crate::bindings_v0_2::fyx::ui::host::remove_surface(surface_id).map_err(host_error)
    }

    pub fn show_surface(surface_id: &str) -> Result<(), HostError> {
        crate::bindings_v0_2::fyx::ui::host::show_surface(surface_id).map_err(host_error)
    }

    pub fn hide_surface(surface_id: &str) -> Result<(), HostError> {
        crate::bindings_v0_2::fyx::ui::host::hide_surface(surface_id).map_err(host_error)
    }

    pub fn place_decoration(
        id: &str,
        new_location: &DecorationLocation,
        new_nodes: &[Node],
    ) -> Result<(), HostError> {
        let new_location = location(new_location).map_err(|message| HostError {
            code: HostErrorCode::Unavailable,
            message,
        })?;
        let new_nodes = nodes(new_nodes).map_err(invalid_argument)?;
        crate::bindings_v0_2::fyx::ui::decorations::place_decoration(id, &new_location, &new_nodes)
            .map_err(host_error)
    }

    pub fn remove_decoration(id: &str) -> Result<(), HostError> {
        crate::bindings_v0_2::fyx::ui::decorations::remove_decoration(id).map_err(host_error)
    }

    pub fn notify(new_notification: &crate::Notification) -> Result<u64, HostError> {
        crate::bindings_v0_2::fyx::ui::notifications::notify(&notification(new_notification))
            .map_err(host_error)
    }

    pub fn dismiss_notification(id: u64) {
        crate::bindings_v0_2::fyx::ui::notifications::dismiss_notification(id);
    }

    pub fn progress_start(options: &crate::ProgressOptions) -> Result<u64, HostError> {
        crate::bindings_v0_2::fyx::ui::notifications::progress_start(&progress_options(options))
            .map_err(host_error)
    }

    pub fn progress_update(
        id: u64,
        value: crate::ProgressValue,
        message: Option<&str>,
    ) -> Result<(), HostError> {
        crate::bindings_v0_2::fyx::ui::notifications::progress_update(
            id,
            progress_value(value),
            message,
        )
        .map_err(host_error)
    }

    pub fn progress_end(id: u64) {
        crate::bindings_v0_2::fyx::ui::notifications::progress_end(id);
    }

    pub fn add_image(_format: crate::ImageFormat, _bytes: &[u8]) -> Result<u32, HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("add_image"),
        })
    }

    /// Nothing to free: 0.2 has no generated images.
    pub fn remove_image(_image: u32) {}
}

#[cfg(test)]
mod tests {
    use super::glue::*;
    use super::*;
    use crate::{
        ButtonProps, ImageFit, ImageProps, ImageSource, InputProps, InvalidateRowsOp, ListProps,
    };

    fn one(props: Props) -> Vec<Node> {
        vec![Node {
            id: 1,
            parent: None,
            props,
        }]
    }

    #[test]
    fn every_0_2_value_carries_over() {
        let converted = nodes(&[
            Node {
                id: 1,
                parent: None,
                props: Props::Button(ButtonProps::new("Go").icon(Icon::Play).tooltip("Run it")),
            },
            Node {
                id: 2,
                parent: None,
                props: Props::Input(InputProps::new("x").secret()),
            },
            Node {
                id: 3,
                parent: None,
                props: Props::List(ListProps::new(3, 20).selected(Some(1))),
            },
        ])
        .expect("converts");
        assert_eq!(
            converted[0].props,
            v2::Props::Button(v2::ButtonProps {
                label: "Go".into(),
                icon: Some(v2::Icon::Play),
                style: v2::ButtonStyle::Default,
                disabled: false,
                tooltip: Some("Run it".into()),
            })
        );
        assert!(matches!(&converted[1].props, v2::Props::Input(input) if input.secret));
        let invalidate = Update::Patches(vec![Patch::InvalidateRows(InvalidateRowsOp {
            target: 3,
            rows: None,
        })]);
        assert!(update(&invalidate).is_ok());
        assert_eq!(
            event(v2::UiEvent::Dismissed(v2::DismissReason::Escape)),
            UiEvent::Dismissed(DismissReason::Escape)
        );
    }

    #[test]
    fn section_m_carries_over() {
        let converted = notification(
            &crate::Notification::new("panel", crate::NotificationLevel::Error, "Failed")
                .body("details")
                .action("retry", "Retry")
                .sticky(),
        );
        assert_eq!(
            converted,
            v2n::Notification {
                owner: "panel".into(),
                level: v2n::NotificationLevel::Error,
                title: "Failed".into(),
                body: Some("details".into()),
                actions: vec![v2n::NotificationAction {
                    id: "retry".into(),
                    label: "Retry".into(),
                }],
                auto_dismiss: false,
            }
        );
        let options = progress_options(
            &crate::ProgressOptions::new("panel", "Indexing", crate::ProgressLocation::Panel)
                .cancellable(),
        );
        assert_eq!(options.location, v2n::ProgressLocation::Panel);
        assert!(options.cancellable);
        assert_eq!(
            progress_value(crate::ProgressValue::Percent(7)),
            v2n::ProgressValue::Percent(7)
        );
    }

    #[test]
    fn values_without_a_0_2_form_are_errors() {
        let image = Props::Image(ImageProps {
            source: ImageSource::Asset("assets/logo.svg".into()),
            alt: "Logo".into(),
            width: None,
            height: None,
            fit: ImageFit::Contain,
            tint: None,
        });
        assert!(nodes(&one(image)).unwrap_err().contains("needs fyx:ui 0.3"));
        let in_document = DecorationLocation {
            target: DecorationTarget::Document(7),
            line: 1,
            placement: DecorationPlacement::Above,
            height_lines: None,
        };
        assert!(location(&in_document).is_err());
        assert!(add_image(crate::ImageFormat::Png, &[]).is_err());
    }
}
