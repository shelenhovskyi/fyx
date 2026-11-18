//! What the 0.2 and 0.3 bindings share: the `host` imports (the 0.3 `host` interface is 0.2's
//! with one more error code) and the conversions of the types 0.3 copies forward from 0.2.
//!
//! The macros name `wit` (the version's `types` module), `wit_host` (its `host` module) and
//! `model` (`ui_ext::v0_2`) as they resolve where they are invoked, so each version module invokes
//! them once with its own bindings in scope.

use ui_ext::v0_2 as model;

use crate::UiState;

impl UiState {
    /// The permission broker's answer for `capability` ([`UiState::check_capability`]), asked at
    /// every call so a revocation applies at once, as `not-permitted`. The capability must also be
    /// declared in the manifest, which [`crate::effects::ApiState`] checks.
    pub(crate) fn permit(&self, capability: &str) -> Result<(), model::HostError> {
        self.check_capability(capability)
            .map_err(|denied| model::HostError {
                code: model::HostErrorCode::NotPermitted,
                message: denied.to_string(),
            })
    }
}

/// A value of a newer world the model cannot hold yet, as an `invalid-argument`.
pub(crate) fn unsupported(what: &str) -> model::HostError {
    model::HostError {
        code: model::HostErrorCode::InvalidArgument,
        message: format!("{what} is not supported by this build of Fyx yet"),
    }
}

/// String bytes of a node's props, for [`crate::MAX_LIFTED_TEXT_PER_CALL`].
pub(crate) fn props_text_len(props: &model::Props) -> usize {
    let option_len = |text: &Option<String>| text.as_ref().map_or(0, String::len);
    let icon_len = |icon: &model::IconRef| match icon {
        model::IconRef::Frozen(_) => 0,
        model::IconRef::Upstream(name) => name.len(),
    };
    match props {
        model::Props::Flex(_) | model::Props::List(_) => 0,
        model::Props::Text(props) => props.content.len(),
        model::Props::Button(props) => {
            props.label.len() + props.icon.as_ref().map_or(0, icon_len) + option_len(&props.tooltip)
        }
        model::Props::Input(props) => props.value.len() + props.placeholder.len(),
        model::Props::Table(props) => props.columns.iter().map(String::len).sum(),
        model::Props::Icon(props) => icon_len(&props.name),
        model::Props::Markdown(text) => text.len(),
        model::Props::DiffView(props) => {
            props.old_text.len() + props.new_text.len() + option_len(&props.language)
        }
        model::Props::Image(props) => {
            props.alt.len()
                + match &props.source {
                    ui_ext::image::ImageSource::Asset(path) => path.len(),
                    ui_ext::image::ImageSource::Generated(_) => 0,
                }
        }
    }
}

pub(crate) fn update_text_len(update: &model::Update) -> usize {
    match update {
        model::Update::Unchanged => 0,
        model::Update::Replace(nodes) => nodes.iter().map(|node| props_text_len(&node.props)).sum(),
        model::Update::Patches(patches) => patches
            .iter()
            .map(|patch| match patch {
                model::Patch::SetProps(_, props) => props_text_len(props),
                model::Patch::Insert(insert) => props_text_len(&insert.node.props),
                model::Patch::Remove(_) | model::Patch::InvalidateRows(_) => 0,
            })
            .sum(),
    }
}

/// `impl wit_host::Host for UiState`: the effectful imports of design.md A–F, checked against
/// [`crate::effects::ApiState`].
macro_rules! host_imports {
    () => {
        impl wit::Host for UiState {}

        fn result(
            result: Result<(), model::HostError>,
        ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
            Ok(result.map_err(Into::into))
        }

        impl wit_host::Host for UiState {
            fn push_update(
                &mut self,
                surface_id: String,
                update: wit::Update,
            ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
                let update = match model::Update::try_from(update) {
                    Ok(update) => update,
                    Err(error) => return result(Err(error)),
                };
                self.count_lifted_text(
                    surface_id.len() + $crate::conversions::update_text_len(&update),
                )?;
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
                if let Err(error) = self.permit(surface.kind.capability()) {
                    return result(Err(error));
                }
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
                if let Some(capability) = self
                    .api
                    .live_surface_kind(&surface_id)
                    .map(model::SurfaceKind::capability)
                    && let Err(error) = self.permit(capability)
                {
                    return result(Err(error));
                }
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
                    wit_host::LogLevel::Trace => log::Level::Trace,
                    wit_host::LogLevel::Debug => log::Level::Debug,
                    wit_host::LogLevel::Info => log::Level::Info,
                    wit_host::LogLevel::Warn => log::Level::Warn,
                    wit_host::LogLevel::Error => log::Level::Error,
                };
                self.write_log(level, message)
            }
        }

        impl From<model::HostError> for wit_host::HostError {
            fn from(error: model::HostError) -> Self {
                let model::HostError { code, message } = error;
                Self {
                    code: match code {
                        model::HostErrorCode::UnknownSurface => {
                            wit_host::HostErrorCode::UnknownSurface
                        }
                        model::HostErrorCode::LimitExceeded => {
                            wit_host::HostErrorCode::LimitExceeded
                        }
                        model::HostErrorCode::InvalidArgument => {
                            wit_host::HostErrorCode::InvalidArgument
                        }
                        model::HostErrorCode::NotPermitted => wit_host::HostErrorCode::NotPermitted,
                        model::HostErrorCode::Unavailable => wit_host::HostErrorCode::Unavailable,
                    },
                    message,
                }
            }
        }
    };
}

/// `impl wit_notifications::Host for UiState` (design.md M): `wit_notifications` is the version's
/// `notifications` module. Both worlds have the same interface.
macro_rules! notification_imports {
    () => {
        impl wit_notifications::Host for UiState {
            fn notify(
                &mut self,
                notification: wit_notifications::Notification,
            ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
                let notification = model::Notification {
                    owner: notification.owner,
                    level: match notification.level {
                        wit_notifications::NotificationLevel::Info => model::NotificationLevel::Info,
                        wit_notifications::NotificationLevel::Success => {
                            model::NotificationLevel::Success
                        }
                        wit_notifications::NotificationLevel::Warning => {
                            model::NotificationLevel::Warning
                        }
                        wit_notifications::NotificationLevel::Error => {
                            model::NotificationLevel::Error
                        }
                    },
                    title: notification.title,
                    body: notification.body,
                    actions: notification
                        .actions
                        .into_iter()
                        .map(|action| model::NotificationAction {
                            id: action.id,
                            label: action.label,
                        })
                        .collect(),
                    auto_dismiss: notification.auto_dismiss,
                };
                self.count_lifted_text(
                    notification.owner.len()
                        + notification.title.len()
                        + notification.body.as_ref().map_or(0, String::len)
                        + notification
                            .actions
                            .iter()
                            .map(|action| action.id.len() + action.label.len())
                            .sum::<usize>(),
                )?;
                if let Err(error) = self.permit(model::CAPABILITY_NOTIFICATIONS) {
                    return Ok(Err(error.into()));
                }
                let title = notification.title.clone();
                let result = self.api.notify(notification);
                if let Err(error) = &result
                    && error.code == model::HostErrorCode::LimitExceeded
                {
                    // Spec decision 22: notifications past the caps go to the log instead.
                    log::info!(
                        target: "fyx_ui",
                        "{}: notification not shown ({}): {}",
                        self.extension_id,
                        error.message,
                        $crate::conversions::sanitize_title(&title)
                    );
                }
                Ok(result.map_err(Into::into))
            }

            fn dismiss_notification(&mut self, id: u64) -> wasmtime::Result<()> {
                self.api.dismiss_notification(id);
                Ok(())
            }

            fn progress_start(
                &mut self,
                options: wit_notifications::ProgressOptions,
            ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
                let options = model::ProgressOptions {
                    owner: options.owner,
                    title: options.title,
                    location: match options.location {
                        wit_notifications::ProgressLocation::StatusBar => {
                            model::ProgressLocation::StatusBar
                        }
                        wit_notifications::ProgressLocation::Panel => {
                            model::ProgressLocation::Panel
                        }
                    },
                    cancellable: options.cancellable,
                };
                self.count_lifted_text(options.owner.len() + options.title.len())?;
                if let Err(error) = self.permit(model::CAPABILITY_NOTIFICATIONS) {
                    return Ok(Err(error.into()));
                }
                Ok(self.api.progress_start(options).map_err(Into::into))
            }

            fn progress_update(
                &mut self,
                id: u64,
                value: wit_notifications::ProgressValue,
                message: Option<String>,
            ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
                self.count_lifted_text(message.as_ref().map_or(0, String::len))?;
                let value = match value {
                    wit_notifications::ProgressValue::Indeterminate => {
                        model::ProgressValue::Indeterminate
                    }
                    wit_notifications::ProgressValue::Percent(percent) => {
                        model::ProgressValue::Percent(percent)
                    }
                };
                Ok(self
                    .api
                    .progress_update(id, value, message)
                    .map_err(Into::into))
            }

            fn progress_end(&mut self, id: u64) -> wasmtime::Result<()> {
                self.api.progress_end(id);
                Ok(())
            }
        }
    };
}

/// A notification title as it goes into the log: one line, control and format characters
/// escaped.
pub(crate) fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .take(80)
        .flat_map(|character| {
            if character.is_control() || matches!(character, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}') {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

/// The conversions of the copied-forward types. `props: { … }` and `kinds: { … }` are the extra
/// `match` arms of the version's `props` and `surface-kind` variants; they map the case or return
/// `Err` with [`unsupported`].
macro_rules! conversions {
    (props: { $($props:tt)* } kinds: { $($kinds:tt)* }) => {
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

        $crate::conversions::icons! {
            ArrowUp ArrowDown ArrowLeft ArrowRight ChevronUp ChevronDown ChevronLeft ChevronRight
            Expand Collapse Menu Ellipsis ListTree ListTodo Table FileTree Plus Dash Check
            CheckDouble Close Search Filter Refresh Undo Copy Pencil Trash Replace Download Send
            Share Link Pin Unpin Play Pause Stop Maximize Minimize Split File FileCode FileDiff
            FileText Folder FolderOpen Code Terminal Diff Regex CaseSensitive WholeWord Hash AtSign
            GitBranch GitCommit PullRequest Info Warning Error Help Bell BellDot Circle Dot Star
            StarFilled Bookmark ThumbsUp ThumbsDown TodoPending TodoProgress TodoComplete Spinner
            Blank Settings Clock Debug Eye EyeOff Lock Unlock Person People Chat Envelope Book
            Notepad Image Globe Server Database Keyboard Box Archive Sparkle Bolt Flame Power Blocks
        }

        impl TryFrom<wit::Props> for model::Props {
            type Error = model::HostError;

            fn try_from(value: wit::Props) -> Result<Self, model::HostError> {
                Ok(match value {
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
                    $($props)*
                })
            }
        }

        impl TryFrom<wit::Node> for model::Node {
            type Error = model::HostError;

            fn try_from(value: wit::Node) -> Result<Self, model::HostError> {
                Ok(Self {
                    id: value.id,
                    parent: value.parent,
                    props: value.props.try_into()?,
                })
            }
        }

        impl TryFrom<wit::Patch> for model::Patch {
            type Error = model::HostError;

            fn try_from(value: wit::Patch) -> Result<Self, model::HostError> {
                Ok(match value {
                    wit::Patch::SetProps((id, props)) => Self::SetProps(id, props.try_into()?),
                    wit::Patch::Insert(op) => Self::Insert(model::InsertOp {
                        node: op.node.try_into()?,
                        before: op.before,
                    }),
                    wit::Patch::Remove(id) => Self::Remove(id),
                    wit::Patch::InvalidateRows(op) => {
                        Self::InvalidateRows(model::InvalidateRowsOp {
                            target: op.target,
                            rows: op.rows.map(|rows| model::RowRange {
                                start: rows.start,
                                count: rows.count,
                            }),
                        })
                    }
                })
            }
        }

        impl TryFrom<wit::Update> for model::Update {
            type Error = model::HostError;

            fn try_from(value: wit::Update) -> Result<Self, model::HostError> {
                Ok(match value {
                    wit::Update::Unchanged => Self::Unchanged,
                    wit::Update::Replace(nodes) => Self::Replace(
                        nodes
                            .into_iter()
                            .map(TryInto::try_into)
                            .collect::<Result<_, _>>()?,
                    ),
                    wit::Update::Patches(patches) => Self::Patches(
                        patches
                            .into_iter()
                            .map(TryInto::try_into)
                            .collect::<Result<_, _>>()?,
                    ),
                })
            }
        }

        impl TryFrom<wit::ListRow> for model::ListRow {
            type Error = model::HostError;

            fn try_from(value: wit::ListRow) -> Result<Self, model::HostError> {
                Ok(Self {
                    index: value.index,
                    indent: value.indent,
                    nodes: value
                        .nodes
                        .into_iter()
                        .map(TryInto::try_into)
                        .collect::<Result<_, _>>()?,
                })
            }
        }

        impl TryFrom<wit::Surface> for model::Surface {
            type Error = model::HostError;

            fn try_from(value: wit::Surface) -> Result<Self, model::HostError> {
                Ok(Self {
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
                        $($kinds)*
                    },
                })
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
                    model::UiEvent::NotificationAction(event) => {
                        Self::NotificationAction(wit::NotificationActionEvent {
                            notification: event.notification,
                            action: event.action,
                        })
                    }
                    model::UiEvent::NotificationClosed(id, reason) => Self::NotificationClosed((
                        id,
                        match reason {
                            model::NotificationCloseReason::User => {
                                wit::NotificationCloseReason::User
                            }
                            model::NotificationCloseReason::Expired => {
                                wit::NotificationCloseReason::Expired
                            }
                            model::NotificationCloseReason::WindowClosed => {
                                wit::NotificationCloseReason::WindowClosed
                            }
                        },
                    )),
                    model::UiEvent::ProgressCancelRequested(id) => {
                        Self::ProgressCancelRequested(id)
                    }
                }
            }
        }
    };
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

pub(crate) use {conversions, host_imports, icons, notification_imports};

/// A list conversion that stops at the first value the model cannot hold.
pub(crate) fn try_collect<T, U>(values: Vec<T>) -> Result<Vec<U>, model::HostError>
where
    U: TryFrom<T, Error = model::HostError>,
{
    values.into_iter().map(U::try_from).collect()
}
