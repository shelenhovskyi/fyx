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
    host as wit_host, images as wit_images, io as wit_io, network as wit_network,
    notifications as wit_notifications, process as wit_process, types as wit,
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
    wit_network::add_to_linker::<_, State>(linker, |state| state)?;
    Ok(())
}

impl wit_network::Host for State {
    fn fetch(
        &mut self,
        request: wit_network::HttpRequest,
    ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
        use ui_ext::network as net;
        self.count_lifted_text(
            request.url.len()
                + request
                    .headers
                    .iter()
                    .map(|(name, value)| name.len() + value.len())
                    .sum::<usize>(),
        )?;
        let request = net::FetchRequest {
            method: match request.method {
                wit_network::HttpMethod::Get => net::Method::Get,
                wit_network::HttpMethod::Head => net::Method::Head,
                wit_network::HttpMethod::Post => net::Method::Post,
                wit_network::HttpMethod::Put => net::Method::Put,
                wit_network::HttpMethod::Patch => net::Method::Patch,
                wit_network::HttpMethod::Delete => net::Method::Delete,
                wit_network::HttpMethod::Options => net::Method::Options,
            },
            url: request.url,
            headers: request.headers,
            body: request.body,
            timeout_ms: request.timeout_ms,
        };
        Ok(self.api.fetch(request).map_err(Into::into))
    }

    fn cancel_fetch(&mut self, request: u64) -> wasmtime::Result<()> {
        self.api.cancel_fetch(request);
        Ok(())
    }
}

/// An outcome for the `handle-io` export.
pub(crate) fn io_event(event: ui_ext::network::IoEvent) -> Option<wit_io::IoEvent> {
    use ui_ext::network as net;
    match event {
        net::IoEvent::FetchDone(id, result) => Some(wit_io::IoEvent::FetchDone((
            id,
            result
                .map(|response| wit_network::HttpResponse {
                    status: response.status,
                    headers: response.headers,
                    body: response.body,
                    url: response.url,
                })
                .map_err(|error| match error {
                    net::FetchError::Denied => wit_network::FetchError::Denied,
                    net::FetchError::HostNotAllowed(host) => {
                        wit_network::FetchError::HostNotAllowed(host)
                    }
                    net::FetchError::RedirectNotAllowed(host) => {
                        wit_network::FetchError::RedirectNotAllowed(host)
                    }
                    net::FetchError::Dns(error) => wit_network::FetchError::Dns(error),
                    net::FetchError::Connect(error) => wit_network::FetchError::Connect(error),
                    net::FetchError::Tls(error) => wit_network::FetchError::Tls(error),
                    net::FetchError::Timeout => wit_network::FetchError::Timeout,
                    net::FetchError::TooLarge => wit_network::FetchError::TooLarge,
                    net::FetchError::Cancelled => wit_network::FetchError::Cancelled,
                    net::FetchError::Other(error) => wit_network::FetchError::Other(error),
                }),
        ))),
        net::IoEvent::ProcessOutput(output) => {
            Some(wit_io::IoEvent::ProcessOutput(wit_process::ProcessOutput {
                process: output.process,
                stream: match output.stream {
                    ui_ext::process::OutputStream::Stdout => wit_process::OutputStream::Stdout,
                    ui_ext::process::OutputStream::Stderr => wit_process::OutputStream::Stderr,
                },
                data: output.data,
            }))
        }
        net::IoEvent::ProcessExited(exit) => {
            Some(wit_io::IoEvent::ProcessExited(wit_process::ProcessExit {
                process: exit.process,
                code: exit.code,
                killed: exit.killed,
            }))
        }
        net::IoEvent::SpawnFailed(id, error) => Some(wit_io::IoEvent::SpawnFailed((
            id,
            match error {
                ui_ext::process::SpawnError::Denied => wit_process::SpawnError::Denied,
                ui_ext::process::SpawnError::NotFound(program) => {
                    wit_process::SpawnError::NotFound(program)
                }
                ui_ext::process::SpawnError::LimitExceeded => {
                    wit_process::SpawnError::LimitExceeded
                }
                ui_ext::process::SpawnError::Io(error) => wit_process::SpawnError::Io(error),
            },
        ))),
        // The harness changes no permission on its own.
        net::IoEvent::PermissionChanged(..) => None,
    }
}

/// The `fyx:ui` 0.3 interfaces [`add_to_linker`] links.
const LINKED_INTERFACES: [&str; 5] = ["types", "host", "images", "notifications", "network"];

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
                model::HostErrorCode::Denied => wit_host::HostErrorCode::Denied,
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
            wit::Props::Canvas(canvas) => Self::Canvas(canvas_props(canvas)),
        })
    }
}

/// A 0.3 canvas in the model (`ui_ext::canvas`), unchecked: the checks run where the host runs
/// them, when the update or reply is applied.
fn canvas_props(canvas: wit::CanvasProps) -> ui_ext::canvas::CanvasProps {
    use ui_ext::canvas as model_canvas;
    let point = |point: wit::Point| model_canvas::Point {
        x: point.x.into(),
        y: point.y.into(),
    };
    let paint = |paint: wit::Paint| model_canvas::Paint {
        color: match paint.color {
            wit::CanvasColor::Token(token) => model_canvas::CanvasColor::Token(token.into()),
            wit::CanvasColor::Data(data) => model_canvas::CanvasColor::Data(match data {
                wit::DataColor::Series1 => model_canvas::DataColor::Series1,
                wit::DataColor::Series2 => model_canvas::DataColor::Series2,
                wit::DataColor::Series3 => model_canvas::DataColor::Series3,
                wit::DataColor::Series4 => model_canvas::DataColor::Series4,
                wit::DataColor::Series5 => model_canvas::DataColor::Series5,
                wit::DataColor::Series6 => model_canvas::DataColor::Series6,
                wit::DataColor::Series7 => model_canvas::DataColor::Series7,
                wit::DataColor::Series8 => model_canvas::DataColor::Series8,
            }),
        },
        opacity: paint.opacity,
    };
    let stroke = |stroke: wit::Stroke| model_canvas::Stroke {
        paint: paint(stroke.paint),
        width: stroke.width.into(),
    };
    let rect = |rect: wit::Rect| model_canvas::Rect {
        x: rect.x.into(),
        y: rect.y.into(),
        width: rect.width.into(),
        height: rect.height.into(),
    };
    model_canvas::CanvasProps {
        width: canvas.width,
        height: canvas.height,
        description: canvas.description,
        ops: canvas
            .ops
            .into_iter()
            .map(|op| match op {
                wit::DrawOp::Line(op) => model_canvas::DrawOp::Line {
                    from: point(op.from),
                    to: point(op.to),
                    stroke: stroke(op.stroke),
                },
                wit::DrawOp::Rect(op) => model_canvas::DrawOp::Rect {
                    bounds: rect(op.bounds),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::RoundedRect(op) => model_canvas::DrawOp::RoundedRect {
                    bounds: rect(op.bounds),
                    radius: op.radius.into(),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::Circle(op) => model_canvas::DrawOp::Circle {
                    center: point(op.center),
                    radius: op.radius.into(),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::Path(op) => model_canvas::DrawOp::Path {
                    segments: op
                        .segments
                        .into_iter()
                        .map(|segment| match segment {
                            wit::PathSegment::MoveTo(to) => {
                                model_canvas::PathSegment::MoveTo(point(to))
                            }
                            wit::PathSegment::LineTo(to) => {
                                model_canvas::PathSegment::LineTo(point(to))
                            }
                            wit::PathSegment::QuadTo((control, to)) => {
                                model_canvas::PathSegment::QuadTo(point(control), point(to))
                            }
                            wit::PathSegment::CubicTo((first, second, to)) => {
                                model_canvas::PathSegment::CubicTo(
                                    point(first),
                                    point(second),
                                    point(to),
                                )
                            }
                            wit::PathSegment::Close => model_canvas::PathSegment::Close,
                        })
                        .collect(),
                    fill: op.fill.map(paint),
                    stroke: op.stroke.map(stroke),
                },
                wit::DrawOp::Text(op) => model_canvas::DrawOp::Text {
                    at: point(op.at),
                    text: op.text,
                    paint: paint(op.paint),
                    size: match op.size {
                        wit::CanvasTextSize::Small => model_canvas::CanvasTextSize::Small,
                        wit::CanvasTextSize::Medium => model_canvas::CanvasTextSize::Medium,
                        wit::CanvasTextSize::Large => model_canvas::CanvasTextSize::Large,
                    },
                    anchor: match op.anchor {
                        wit::TextAnchor::Start => model_canvas::TextAnchor::Start,
                        wit::TextAnchor::Middle => model_canvas::TextAnchor::Middle,
                        wit::TextAnchor::End => model_canvas::TextAnchor::End,
                    },
                    monospace: op.monospace,
                },
            })
            .collect(),
        clickable: canvas.clickable,
        fit: match canvas.fit {
            wit::CanvasFit::Clip => model_canvas::CanvasFit::Clip,
            wit::CanvasFit::ScaleDown => model_canvas::CanvasFit::ScaleDown,
        },
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
            model::UiEvent::CanvasClick(click) => Self::CanvasClick(wit::CanvasClick {
                canvas: click.canvas,
                x: click.x.0,
                y: click.y.0,
            }),
        }
    }
}
