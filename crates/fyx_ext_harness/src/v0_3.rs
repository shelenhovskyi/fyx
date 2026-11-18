//! The staged `fyx:ui@0.3.0` world (`crates/ui_ext/wit/since_v0.3.0`), as far as the host runs it
//! (plan 4.10): `types`, `host`, `images` and `notifications`. Every other 0.3 interface,
//! `decorations` included, traps when called, as in the host ([`define_unlinked_as_traps`]).
//!
//! Values the model cannot hold yet are refused as the host refuses them: a `canvas` node or a
//! `picker` surface is `invalid-argument` in a host call and the guest's error in a reply.

use ui_ext::image as model_image;
use ui_ext::v0_2 as model;

use crate::{State, update_text_len};

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../ui_ext/wit/since_v0.3.0",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
    });
}

pub(crate) use bindings::UiExtension;
use bindings::fyx::ui::{
    host as wit_host, images as wit_images, notifications as wit_notifications, types as wit,
};

/// What the host says for a value its model cannot hold yet.
pub(crate) fn unsupported(what: &str) -> model::HostError {
    model::HostError {
        code: model::HostErrorCode::InvalidArgument,
        message: format!("{what} is not supported by this build of Fyx yet"),
    }
}

fn result(
    result: Result<(), model::HostError>,
) -> wasmtime::Result<Result<(), wit_host::HostError>> {
    Ok(result.map_err(Into::into))
}

pub(crate) fn try_collect<T, U>(values: Vec<T>) -> Result<Vec<U>, model::HostError>
where
    U: TryFrom<T, Error = model::HostError>,
{
    values.into_iter().map(U::try_from).collect()
}

/// The 0.3 linker: WASI and the interfaces the host implements; the rest trap at instantiation.
pub(crate) fn add_to_linker(
    linker: &mut wasmtime::component::Linker<State>,
) -> wasmtime::Result<()> {
    wit::add_to_linker::<_, State>(linker, |state| state)?;
    wit_host::add_to_linker::<_, State>(linker, |state| state)?;
    wit_images::add_to_linker::<_, State>(linker, |state| state)?;
    wit_notifications::add_to_linker::<_, State>(linker, |state| state)?;
    Ok(())
}

/// The `fyx:ui` 0.3 interfaces [`add_to_linker`] links.
const LINKED_INTERFACES: [&str; 4] = ["types", "host", "images", "notifications"];

/// The resource type of a stubbed interface; no value of it is ever made.
struct Unlinked;

/// Stubs every `fyx:ui` interface `component` imports that [`add_to_linker`] does not link: its
/// functions trap when called, so a component that never calls them loads and runs.
///
/// Only `fyx:ui` imports: wasmtime's `define_unknown_imports_as_traps` also stubs a WASI import
/// whose patch version is newer than `wasmtime_wasi`'s (`wasi:random/insecure-seed@0.2.9`), which
/// hides the semver-compatible definition, so std's `HashMap` seeding traps.
pub(crate) fn define_unlinked_as_traps(
    linker: &mut wasmtime::component::Linker<State>,
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
) -> wasmtime::Result<()> {
    use wasmtime::component::ResourceType;
    use wasmtime::component::types::ComponentItem;

    let component_type = component.component_type();
    for (name, item) in component_type.imports(engine) {
        let Some(interface) = name.strip_prefix("fyx:ui/") else {
            continue;
        };
        let interface = interface
            .split_once('@')
            .map_or(interface, |(name, _)| name);
        if LINKED_INTERFACES.contains(&interface) {
            continue;
        }
        let ComponentItem::ComponentInstance(instance) = item.ty else {
            continue;
        };
        let mut stub = linker.instance(name)?;
        for (export, item) in instance.exports(engine) {
            match item.ty {
                ComponentItem::ComponentFunc(_) => {
                    let what = format!("{name}#{export}");
                    stub.func_new(export, move |_, _, _, _| {
                        Err(wasmtime::Error::msg(format!(
                            "{what} is not implemented by this build of Fyx yet"
                        )))
                    })?;
                }
                ComponentItem::Resource(_) => {
                    stub.resource(export, ResourceType::host::<Unlinked>(), |_, _| Ok(()))?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

impl wit::Host for State {}

impl wit_host::Host for State {
    fn push_update(
        &mut self,
        surface_id: String,
        update: wit::Update,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        let update = match model::Update::try_from(update) {
            Ok(update) => update,
            Err(error) => return result(Err(error)),
        };
        self.count_lifted_text(surface_id.len() + update_text_len(&update))?;
        result(self.api.push_update(surface_id, update))
    }

    fn request_tick(
        &mut self,
        surface_id: String,
        delay_ms: u32,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        result(self.api.request_tick(surface_id, delay_ms))
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
        let surface = match model::Surface::try_from(surface) {
            Ok(surface) => surface,
            Err(error) => return result(Err(error)),
        };
        self.count_lifted_text(surface.id.len() + surface.title.len())?;
        result(self.api.add_surface(surface))
    }

    fn remove_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        result(self.api.remove_surface(surface_id))
    }

    fn show_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        result(self.api.show_surface(surface_id))
    }

    fn hide_surface(
        &mut self,
        surface_id: String,
    ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
        self.count_lifted_text(surface_id.len())?;
        result(self.api.hide_surface(surface_id))
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

impl wit_images::Host for State {
    fn add_image(
        &mut self,
        format: wit_images::ImageFormat,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<u32, wit_host::HostError>> {
        let format = match format {
            wit_images::ImageFormat::Png => model_image::ImageFormat::Png,
            wit_images::ImageFormat::Svg => model_image::ImageFormat::Svg,
        };
        Ok(self.api.add_image(format, bytes).map_err(Into::into))
    }

    fn remove_image(&mut self, image: u32) -> wasmtime::Result<()> {
        self.api.remove_image(image);
        Ok(())
    }
}

crate::notifications::notification_imports!();

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

impl TryFrom<wit::Props> for model::Props {
    type Error = model::HostError;

    fn try_from(props: wit::Props) -> Result<Self, Self::Error> {
        Ok(match props {
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
            wit::Props::Image(image) => Self::Image(model_image::ImageProps {
                source: match image.source {
                    wit::ImageSource::Asset(path) => model_image::ImageSource::Asset(path),
                    wit::ImageSource::Generated(handle) => {
                        model_image::ImageSource::Generated(handle)
                    }
                },
                alt: image.alt,
                width: image.width,
                height: image.height,
                fit: match image.fit {
                    wit::ImageFit::Contain => model_image::ImageFit::Contain,
                    wit::ImageFit::Cover => model_image::ImageFit::Cover,
                },
                tint: image.tint.map(Into::into),
            }),
            wit::Props::Canvas(_) => return Err(unsupported("a canvas node")),
        })
    }
}

impl TryFrom<wit::Node> for model::Node {
    type Error = model::HostError;

    fn try_from(node: wit::Node) -> Result<Self, Self::Error> {
        Ok(Self {
            id: node.id,
            parent: node.parent,
            props: node.props.try_into()?,
        })
    }
}

impl TryFrom<wit::Patch> for model::Patch {
    type Error = model::HostError;

    fn try_from(patch: wit::Patch) -> Result<Self, Self::Error> {
        Ok(match patch {
            wit::Patch::SetProps((id, props)) => Self::SetProps(id, props.try_into()?),
            wit::Patch::Insert(insert) => Self::Insert(model::InsertOp {
                node: insert.node.try_into()?,
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
        })
    }
}

impl TryFrom<wit::Update> for model::Update {
    type Error = model::HostError;

    fn try_from(update: wit::Update) -> Result<Self, Self::Error> {
        Ok(match update {
            wit::Update::Unchanged => Self::Unchanged,
            wit::Update::Replace(nodes) => Self::Replace(try_collect(nodes)?),
            wit::Update::Patches(patches) => Self::Patches(try_collect(patches)?),
        })
    }
}

impl TryFrom<wit::ListRow> for model::ListRow {
    type Error = model::HostError;

    fn try_from(row: wit::ListRow) -> Result<Self, Self::Error> {
        Ok(Self {
            index: row.index,
            indent: row.indent,
            nodes: try_collect(row.nodes)?,
        })
    }
}

impl TryFrom<wit::Surface> for model::Surface {
    type Error = model::HostError;

    fn try_from(surface: wit::Surface) -> Result<Self, Self::Error> {
        Ok(Self {
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
                wit::SurfaceKind::Picker(_) => return Err(unsupported("a picker surface")),
            },
        })
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
        }
    }
}
