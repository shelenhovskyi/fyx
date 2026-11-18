//! `fyx:ui/notifications` (design.md, section M) for the 0.2 and 0.3 worlds, which declare the
//! same interface in two generated modules. The checks are `ApiState`'s, which mirror the host.

/// `impl wit_notifications::Host for State`, where `wit_notifications` and `wit_host` are the
/// world's generated modules.
macro_rules! notification_imports {
    () => {
        impl wit_notifications::Host for State {
            fn notify(
                &mut self,
                notification: wit_notifications::Notification,
            ) -> wasmtime::Result<Result<u64, wit_host::HostError>> {
                use ui_ext::v0_2::notifications as notify;
                let notification = notify::Notification {
                    owner: notification.owner,
                    level: match notification.level {
                        wit_notifications::NotificationLevel::Info => {
                            notify::NotificationLevel::Info
                        }
                        wit_notifications::NotificationLevel::Success => {
                            notify::NotificationLevel::Success
                        }
                        wit_notifications::NotificationLevel::Warning => {
                            notify::NotificationLevel::Warning
                        }
                        wit_notifications::NotificationLevel::Error => {
                            notify::NotificationLevel::Error
                        }
                    },
                    title: notification.title,
                    body: notification.body,
                    actions: notification
                        .actions
                        .into_iter()
                        .map(|action| notify::NotificationAction {
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
                let title = notification.title.clone();
                let result = self.api.notify(notification);
                if let Err(error) = &result
                    && error.code == ui_ext::v0_2::HostErrorCode::LimitExceeded
                {
                    // Spec decision 22: past the caps the host logs the title instead.
                    self.write_log(
                        ui_ext::LogLevel::Info,
                        format!(
                            "fyx-ext-harness: notification not shown ({}): {title:?}",
                            error.message
                        ),
                    )?;
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
                use ui_ext::v0_2::notifications as notify;
                let options = notify::ProgressOptions {
                    owner: options.owner,
                    title: options.title,
                    location: match options.location {
                        wit_notifications::ProgressLocation::StatusBar => {
                            notify::ProgressLocation::StatusBar
                        }
                        wit_notifications::ProgressLocation::Panel => {
                            notify::ProgressLocation::Panel
                        }
                    },
                    cancellable: options.cancellable,
                };
                self.count_lifted_text(options.owner.len() + options.title.len())?;
                Ok(self.api.progress_start(options).map_err(Into::into))
            }

            fn progress_update(
                &mut self,
                id: u64,
                value: wit_notifications::ProgressValue,
                message: Option<String>,
            ) -> wasmtime::Result<Result<(), wit_host::HostError>> {
                use ui_ext::v0_2::notifications as notify;
                self.count_lifted_text(message.as_ref().map_or(0, String::len))?;
                let value = match value {
                    wit_notifications::ProgressValue::Indeterminate => {
                        notify::ProgressValue::Indeterminate
                    }
                    wit_notifications::ProgressValue::Percent(percent) => {
                        notify::ProgressValue::Percent(percent)
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

pub(crate) use notification_imports;
