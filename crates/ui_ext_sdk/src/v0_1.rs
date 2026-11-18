//! `register_extension!(T, v0_1)`: an extension written against the SDK's types, exported as
//! `fyx:ui@0.1.0` for hosts that only run 0.1.
//!
//! Values cross the boundary converted. What 0.1 has no field for is dropped (tooltips, list
//! selection, row indent), icons become upstream `IconName`s, and what 0.1 cannot express without
//! changing behaviour is an error that names 0.2: modal surfaces, `invalidate-rows`, secret
//! inputs (0.1 would show the text), the `blank` icon node, ticks, runtime surfaces and
//! decorations, and, naming 0.3, images, canvases and pickers.

use crate::bindings_v0_1::fyx::ui::types as v1;
use crate::{
    Align, ButtonStyle, ColorToken, FlexDirection, HostError, HostErrorCode, Icon, ListRow,
    LogLevel, Node, Patch, Props, Surface, SurfaceKind, UiEvent, Update,
};

const NEEDS_0_2: &str = "needs fyx:ui 0.2; this extension is exported as 0.1";

fn needs_0_2(what: &str) -> String {
    format!("{what} {NEEDS_0_2}")
}

#[cfg(feature = "fyx-ui-0-3")]
fn needs_0_3(what: &str) -> String {
    format!("{what} needs fyx:ui 0.3; this extension is exported as 0.1")
}

/// The upstream `IconName` (snake case) a 0.1 host resolves for a frozen 0.2 icon, or `None` for
/// `blank`, which draws nothing. Most frozen names are an upstream name in kebab case; the
/// exceptions are listed, and a test checks every result against `crates/icons/src/icons.rs`.
pub fn upstream_icon_name(icon: Icon) -> Option<String> {
    let name = ui_ext::v0_2::Icon::ALL
        .get(icon as usize)
        .map(|icon| icon.name())
        .unwrap_or("blocks");
    let upstream = match name {
        "blank" => return None,
        "expand" => "expand_vertical",
        "collapse" => "list_collapse",
        "search" => "magnifying_glass",
        "refresh" => "rotate_cw",
        "play" => "play_filled",
        "pause" => "debug_pause",
        "file-text" => "file_doc",
        "error" => "x_circle",
        "help" => "circle_help",
        "dot" => "circle",
        "spinner" => "load_circle",
        "unlock" => "lock_off",
        "people" => "user_group",
        "globe" => "public",
        "database" => "database_zap",
        "bolt" => "bolt_filled",
        other => return Some(other.replace('-', "_")),
    };
    Some(upstream.to_string())
}

fn color(color: ColorToken) -> v1::ColorToken {
    match color {
        ColorToken::Text => v1::ColorToken::Text,
        ColorToken::TextMuted => v1::ColorToken::TextMuted,
        ColorToken::TextAccent => v1::ColorToken::TextAccent,
        ColorToken::Error => v1::ColorToken::Error,
        ColorToken::Warning => v1::ColorToken::Warning,
        ColorToken::Success => v1::ColorToken::Success,
        ColorToken::Border => v1::ColorToken::Border,
        ColorToken::Surface => v1::ColorToken::Surface,
        ColorToken::ElevatedSurface => v1::ColorToken::ElevatedSurface,
    }
}

fn props(props: &Props) -> Result<v1::Props, String> {
    Ok(match props {
        Props::Flex(flex) => v1::Props::Flex(v1::FlexProps {
            direction: match flex.direction {
                FlexDirection::Row => v1::FlexDirection::Row,
                FlexDirection::Column => v1::FlexDirection::Column,
            },
            gap: flex.gap,
            padding: flex.padding,
            grow: flex.grow,
            align: match flex.align {
                Align::Start => v1::Align::Start,
                Align::Center => v1::Align::Center,
                Align::End => v1::Align::End,
                Align::Stretch => v1::Align::Stretch,
            },
            background: flex.background.map(color),
        }),
        Props::Text(text) => v1::Props::Text(v1::TextProps {
            content: text.content.clone(),
            color: text.color.map(color),
            bold: text.bold,
            monospace: text.monospace,
        }),
        Props::Button(button) => v1::Props::Button(v1::ButtonProps {
            label: button.label.clone(),
            icon: button.icon.and_then(upstream_icon_name),
            style: match button.style {
                ButtonStyle::Default => v1::ButtonStyle::Default,
                ButtonStyle::Filled => v1::ButtonStyle::Filled,
                ButtonStyle::Subtle => v1::ButtonStyle::Subtle,
                ButtonStyle::Danger => v1::ButtonStyle::Danger,
            },
            disabled: button.disabled,
        }),
        Props::Input(input) => {
            if input.secret {
                return Err(needs_0_2("a secret input"));
            }
            v1::Props::Input(v1::InputProps {
                value: input.value.clone(),
                placeholder: input.placeholder.clone(),
            })
        }
        Props::List(list) => v1::Props::List(v1::ListProps {
            row_count: list.row_count,
            row_height: list.row_height,
        }),
        Props::Table(table) => v1::Props::Table(v1::TableProps {
            columns: table.columns.clone(),
            row_count: table.row_count,
        }),
        Props::Icon(icon) => v1::Props::Icon(v1::IconProps {
            name: upstream_icon_name(icon.name).ok_or_else(|| needs_0_2("the blank icon"))?,
            color: icon.color.map(color),
        }),
        Props::Markdown(source) => v1::Props::Markdown(source.clone()),
        Props::DiffView(diff) => v1::Props::DiffView(v1::DiffViewProps {
            old_text: diff.old_text.clone(),
            new_text: diff.new_text.clone(),
            language: diff.language.clone(),
        }),
        #[cfg(feature = "fyx-ui-0-3")]
        Props::Image(_) => return Err(needs_0_3("an image node")),
        #[cfg(feature = "fyx-ui-0-3")]
        Props::Canvas(_) => return Err(needs_0_3("a canvas node")),
    })
}

fn node(node: &Node) -> Result<v1::Node, String> {
    Ok(v1::Node {
        id: node.id,
        parent: node.parent,
        props: props(&node.props)?,
    })
}

/// The SDK-side conversions and host shims `register_extension!(T, v0_1)` calls.
#[doc(hidden)]
pub mod glue {
    use super::*;

    pub const API_VERSION: (u16, u16, u16) = crate::API_VERSION_V0_1;

    pub fn nodes(nodes: &[Node]) -> Result<Vec<v1::Node>, String> {
        nodes.iter().map(node).collect()
    }

    pub fn update(update: &Update) -> Result<v1::Update, String> {
        Ok(match update {
            Update::Unchanged => v1::Update::Unchanged,
            Update::Replace(tree) => v1::Update::Replace(nodes(tree)?),
            Update::Patches(patches) => v1::Update::Patches(
                patches
                    .iter()
                    .map(|patch| {
                        Ok(match patch {
                            Patch::SetProps((id, new_props)) => {
                                v1::Patch::SetProps((*id, props(new_props)?))
                            }
                            Patch::Insert(insert) => v1::Patch::Insert(v1::InsertOp {
                                node: node(&insert.node)?,
                                before: insert.before,
                            }),
                            Patch::Remove(id) => v1::Patch::Remove(*id),
                            Patch::InvalidateRows(_) => {
                                return Err(needs_0_2("the invalidate-rows patch"));
                            }
                        })
                    })
                    .collect::<Result<_, String>>()?,
            ),
        })
    }

    /// Modal surfaces have no 0.1 form; they are left out, with an error in the log.
    pub fn surfaces(surfaces: Vec<Surface>) -> Vec<v1::Surface> {
        surfaces
            .into_iter()
            .filter_map(|surface| {
                let kind = match surface.kind {
                    SurfaceKind::Panel(panel) => v1::SurfaceKind::Panel(v1::PanelDescriptor {
                        position: match panel.position {
                            crate::DockPosition::Left => v1::DockPosition::Left,
                            crate::DockPosition::Right => v1::DockPosition::Right,
                            crate::DockPosition::Bottom => v1::DockPosition::Bottom,
                        },
                        icon: upstream_icon_name(panel.icon).unwrap_or_else(|| "blocks".into()),
                        default_size: panel.default_size,
                    }),
                    SurfaceKind::StatusItem(item) => {
                        v1::SurfaceKind::StatusItem(v1::StatusItemDescriptor {
                            side: match item.side {
                                crate::StatusBarSide::Left => v1::StatusBarSide::Left,
                                crate::StatusBarSide::Right => v1::StatusBarSide::Right,
                            },
                            priority: item.priority,
                        })
                    }
                    SurfaceKind::Modal(_) => {
                        log(
                            LogLevel::Error,
                            &needs_0_2(&format!("modal surface {:?}", surface.id)),
                        );
                        return None;
                    }
                    #[cfg(feature = "fyx-ui-0-3")]
                    SurfaceKind::Picker(_) => {
                        log(
                            LogLevel::Error,
                            &needs_0_3(&format!("picker surface {:?}", surface.id)),
                        );
                        return None;
                    }
                };
                Some(v1::Surface {
                    id: surface.id,
                    title: surface.title,
                    kind,
                })
            })
            .collect()
    }

    /// A row 0.1 cannot draw is left out, with an error in the log.
    pub fn rows(rows: Vec<ListRow>) -> Vec<v1::ListRow> {
        rows.into_iter()
            .filter_map(|row| match nodes(&row.nodes) {
                Ok(nodes) => Some(v1::ListRow {
                    index: row.index,
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

    pub fn event(event: v1::UiEvent) -> UiEvent {
        match event {
            v1::UiEvent::Click(id) => UiEvent::Click(id),
            v1::UiEvent::InputChanged(change) => UiEvent::InputChanged(change),
            v1::UiEvent::InputSubmitted(submit) => UiEvent::InputSubmitted(submit),
            v1::UiEvent::RowClick(click) => UiEvent::RowClick(click),
        }
    }

    fn unavailable(what: &str) -> Result<(), HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_2(what),
        })
    }

    pub fn log(level: LogLevel, message: &str) {
        #[cfg(target_arch = "wasm32")]
        {
            use crate::bindings_v0_1::fyx::ui::host::{self, LogLevel as V1Level};
            let level = match level {
                LogLevel::Trace => V1Level::Trace,
                LogLevel::Debug => V1Level::Debug,
                LogLevel::Info => V1Level::Info,
                LogLevel::Warn => V1Level::Warn,
                LogLevel::Error => V1Level::Error,
            };
            host::log(level, message);
        }
        #[cfg(not(target_arch = "wasm32"))]
        eprintln!("[{level:?}] {message}");
    }

    /// 0.1's `push-update` has no result; an update 0.1 cannot express is refused here instead.
    pub fn push_update(surface_id: &str, new_update: &Update) -> Result<(), HostError> {
        let new_update = update(new_update).map_err(|message| HostError {
            code: HostErrorCode::InvalidArgument,
            message,
        })?;
        crate::bindings_v0_1::fyx::ui::host::push_update(surface_id, &new_update);
        Ok(())
    }

    pub fn request_tick(_surface_id: &str, _delay_ms: u32) -> Result<(), HostError> {
        unavailable("request_tick")
    }

    /// Nothing to cancel: 0.1 has no ticks.
    pub fn cancel_tick(_surface_id: &str) {}

    pub fn add_surface(_surface: &Surface) -> Result<(), HostError> {
        unavailable("add_surface")
    }

    pub fn remove_surface(_surface_id: &str) -> Result<(), HostError> {
        unavailable("remove_surface")
    }

    pub fn show_surface(_surface_id: &str) -> Result<(), HostError> {
        unavailable("show_surface")
    }

    pub fn hide_surface(_surface_id: &str) -> Result<(), HostError> {
        unavailable("hide_surface")
    }

    pub fn place_decoration(
        _id: &str,
        _location: &crate::DecorationLocation,
        _nodes: &[Node],
    ) -> Result<(), HostError> {
        unavailable("place_decoration")
    }

    pub fn remove_decoration(_id: &str) -> Result<(), HostError> {
        unavailable("remove_decoration")
    }

    pub fn notify(_notification: &crate::Notification) -> Result<u64, HostError> {
        unavailable("notify").map(|()| 0)
    }

    /// Nothing to close: 0.1 has no notifications.
    pub fn dismiss_notification(_id: u64) {}

    pub fn progress_start(_options: &crate::ProgressOptions) -> Result<u64, HostError> {
        unavailable("progress_start").map(|()| 0)
    }

    pub fn progress_update(
        _id: u64,
        _value: crate::ProgressValue,
        _message: Option<&str>,
    ) -> Result<(), HostError> {
        unavailable("progress_update")
    }

    /// Nothing to end: 0.1 has no progress.
    pub fn progress_end(_id: u64) {}

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn add_image(_format: crate::ImageFormat, _bytes: &[u8]) -> Result<u32, HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("add_image"),
        })
    }

    /// Nothing to free: 0.1 has no generated images.
    #[cfg(feature = "fyx-ui-0-3")]
    pub fn remove_image(_image: u32) {}

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn fetch(_request: &crate::HttpRequest) -> Result<u64, HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("fetch"),
        })
    }

    /// Nothing to cancel: 0.1 has no network.
    #[cfg(feature = "fyx-ui-0-3")]
    pub fn cancel_fetch(_request: u64) {}

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn spawn(_request: &crate::SpawnRequest) -> Result<u64, HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("spawn"),
        })
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn write_stdin(_process: u64, _data: &[u8]) -> Result<(), HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("write_stdin"),
        })
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn close_stdin(_process: u64) -> Result<(), HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("close_stdin"),
        })
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn kill(_process: u64) -> Result<(), HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("kill"),
        })
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn apply_edit(_edit: &crate::WorkspaceEdit) -> Result<u64, HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("apply_edit"),
        })
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn observe_start() -> Result<(), HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("start_observing"),
        })
    }

    /// Nothing to stop: this world has no agent feed.
    #[cfg(feature = "fyx-ui-0-3")]
    pub fn observe_stop() {}

    /// This world has no T2 capabilities.
    #[cfg(feature = "fyx-ui-0-3")]
    pub fn query_permission(_capability: crate::T2Capability) -> crate::PermissionState {
        crate::PermissionState::Unavailable
    }

    #[cfg(feature = "fyx-ui-0-3")]
    pub fn request_permission(_capability: crate::T2Capability) -> Result<(), HostError> {
        Err(HostError {
            code: HostErrorCode::Unavailable,
            message: needs_0_3("request_permission"),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::glue::*;
    use super::*;
    use crate::{ButtonProps, IconProps, InputProps, InvalidateRowsOp, ListProps, ModalWidth};

    /// Every frozen icon maps to an upstream `IconName` a 0.1 host can draw, except `blank`.
    /// Checked against the source rather than through a dependency on the GPL `icons` crate.
    #[test]
    fn every_icon_has_an_upstream_name() {
        let source = include_str!("../../icons/src/icons.rs");
        let body = &source[source.find("pub enum IconName").expect("IconName")..];
        let body = &body[..body.find('}').expect("end of IconName")];
        let variants = body
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
        assert_eq!(ui_ext::v0_2::Icon::ALL.len(), 101);
        for (index, frozen) in ui_ext::v0_2::Icon::ALL.iter().enumerate() {
            let icon = icon_at(index);
            match upstream_icon_name(icon) {
                Some(name) => assert!(
                    variants.contains(camel_case(&name).as_str()),
                    "{} maps to {name}, which is not an IconName",
                    frozen.name()
                ),
                None => assert_eq!(frozen.name(), "blank"),
            }
        }
        assert_eq!(upstream_icon_name(Icon::Plus).as_deref(), Some("plus"));
        assert_eq!(
            upstream_icon_name(Icon::ListTodo).as_deref(),
            Some("list_todo")
        );
        assert_eq!(
            upstream_icon_name(Icon::Search).as_deref(),
            Some("magnifying_glass")
        );
    }

    /// The SDK's `Icon` at a WIT declaration index; wit-bindgen enums are `repr(u8)` in WIT order,
    /// like `ui_ext::v0_2::Icon::ALL`. The spot checks above fail if the two orders disagree.
    fn icon_at(index: usize) -> Icon {
        let index = u8::try_from(index).expect("fewer than 256 icons");
        // SAFETY: `index` is below the number of cases, checked against `Icon::ALL`'s length.
        unsafe { Icon::_lift(index) }
    }

    #[test]
    fn values_without_a_0_1_form_are_errors() {
        let secret = Props::Input(InputProps::new("x").secret());
        assert!(
            nodes(&[Node {
                id: 1,
                parent: None,
                props: secret
            }])
            .is_err()
        );
        let blank = Props::Icon(IconProps::new(Icon::Blank));
        assert!(
            nodes(&[Node {
                id: 1,
                parent: None,
                props: blank
            }])
            .is_err()
        );
        let invalidate = Update::Patches(vec![Patch::InvalidateRows(InvalidateRowsOp {
            target: 1,
            rows: None,
        })]);
        assert!(
            update(&invalidate)
                .unwrap_err()
                .contains("needs fyx:ui 0.2")
        );
        assert!(request_tick("x", 100).is_err());
        let notification =
            crate::Notification::new("panel", crate::NotificationLevel::Info, "Saved");
        let error = notify(&notification).unwrap_err();
        assert_eq!(error.code, HostErrorCode::Unavailable);
        assert!(
            error.message.contains("notify needs fyx:ui 0.2"),
            "{}",
            error.message
        );
        let options =
            crate::ProgressOptions::new("panel", "Indexing", crate::ProgressLocation::StatusBar);
        assert!(progress_start(&options).is_err());
        assert!(progress_update(1, crate::ProgressValue::Indeterminate, None).is_err());
        let modal = Surface::modal("picker", "Pick", ModalWidth::Medium);
        let panel = Surface::panel(
            "panel",
            "Panel",
            crate::DockPosition::Left,
            Icon::Search,
            200,
        );
        let converted = surfaces(vec![modal, panel]);
        assert_eq!(converted.len(), 1);
        let v1::SurfaceKind::Panel(descriptor) = &converted[0].kind else {
            panic!("a panel");
        };
        assert_eq!(descriptor.icon, "magnifying_glass");
    }

    #[test]
    fn extras_are_dropped() {
        let button = Props::Button(ButtonProps::new("Go").icon(Icon::Play).tooltip("Run it"));
        let list = Props::List(ListProps::new(3, 20).selected(Some(1)));
        let converted = nodes(&[
            Node {
                id: 1,
                parent: None,
                props: button,
            },
            Node {
                id: 2,
                parent: None,
                props: list,
            },
        ])
        .expect("converts");
        assert_eq!(
            converted[0].props,
            v1::Props::Button(v1::ButtonProps {
                label: "Go".into(),
                icon: Some("play_filled".into()),
                style: v1::ButtonStyle::Default,
                disabled: false,
            })
        );
        assert_eq!(
            converted[1].props,
            v1::Props::List(v1::ListProps {
                row_count: 3,
                row_height: 20
            })
        );
        let rows = rows(vec![ListRow {
            index: 4,
            indent: 2,
            nodes: vec![Node {
                id: crate::ROW_NODE_ID_BASE,
                parent: None,
                props: Props::Text(crate::TextProps::new("row")),
            }],
        }]);
        assert_eq!(rows[0].index, 4);
        assert_eq!(
            event(v1::UiEvent::RowClick((7, 2))),
            UiEvent::RowClick((7, 2))
        );
    }
}
