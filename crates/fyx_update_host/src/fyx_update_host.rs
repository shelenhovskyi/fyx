//! Fyx self-update in the app (plan 6.2, `docs/fyx/platform/self-update.md`): the opt-in
//! `fyx.updates.check` setting (off by default), the "fyx: check for updates" action and the
//! notification that offers a new version with "Install". Nothing is downloaded before "Install",
//! and every check goes through `fyx_update`'s signature, channel, freshness and downgrade checks.
//!
//! Without a release key (`fyx_update::RELEASE_KEYS` is empty until the owner makes one), a check
//! fetches nothing and says that updates are not set up in this build. Swapping the downloaded
//! build in on restart (the install plan and the Windows helper) is the next step; until then
//! "Install" downloads and verifies the build and shows where it is.

mod check;

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fyx_update::{Artifact, Offer};
use gpui::{App, AppContext as _, actions};
use release_channel::{AppVersion, ReleaseChannel};
use semver::Version;
use settings::{RegisterSetting, Settings, SettingsStore};
use workspace::notifications::{
    NotificationId, show_app_notification, simple_message_notification::MessageNotification,
};

pub use check::{Build, CheckOutcome, current_target};

actions!(
    fyx,
    [
        /// Checks once whether a new Fyx release is out, even when `fyx.updates.check` is off.
        CheckForUpdates
    ]
);

/// How long after start the first automatic check runs, and how often after that.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// `fyx.updates` in settings (schema in `settings_content::FyxUpdatesSettingsContent`).
#[derive(Clone, Debug, RegisterSetting)]
pub struct FyxUpdateSettings {
    pub check: bool,
}

impl Settings for FyxUpdateSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            check: content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.updates.as_ref())
                .and_then(|updates| updates.check)
                .unwrap_or(false),
        }
    }
}

/// Whether automatic checks are on; off when no settings are loaded.
pub fn automatic_checks_enabled(cx: &App) -> bool {
    cx.has_global::<SettingsStore>() && FyxUpdateSettings::get_global(cx).check
}

/// Registers the action and starts the daily check, which runs only while the setting is on.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &CheckForUpdates, cx| run_check(true, cx));
    cx.spawn(async move |cx| {
        cx.background_executor().timer(FIRST_CHECK_DELAY).await;
        loop {
            cx.update(|cx| {
                if automatic_checks_enabled(cx) {
                    run_check(false, cx);
                }
            });
            cx.background_executor().timer(CHECK_INTERVAL).await;
        }
    })
    .detach();
}

fn updates_dir() -> PathBuf {
    paths::data_dir().join("updates")
}

fn this_build(cx: &App) -> Build {
    Build {
        channel: ReleaseChannel::global(cx).dev_name().to_owned(),
        version: AppVersion::global(cx),
        target: current_target(),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// One check. `manual` checks report every outcome; automatic ones only report a new version.
fn run_check(manual: bool, cx: &mut App) {
    let http = cx.http_client();
    let build = this_build(cx);
    let task = cx.background_spawn(async move {
        check::check(
            http.as_ref(),
            check::releases_base(),
            fyx_update::RELEASE_KEYS,
            &build,
            &updates_dir(),
            unix_now(),
        )
        .await
    });
    cx.spawn(async move |cx| {
        let outcome = task.await;
        cx.update(|cx| match outcome {
            Ok(CheckOutcome::Checked(Offer::Update {
                version,
                artifact,
                notes_url,
            })) => notify_update(version, artifact, notes_url, cx),
            Ok(CheckOutcome::Checked(Offer::NoBuildForTarget(version))) => notify(
                format!("Fyx {version} is out, but it has no build for this computer yet."),
                cx,
            ),
            Ok(CheckOutcome::Checked(Offer::UpToDate)) if manual => {
                notify(format!("Fyx {} is up to date.", AppVersion::global(cx)), cx)
            }
            Ok(CheckOutcome::NotConfigured) if manual => notify(
                "Updates are not set up in this build yet: it has no Fyx release key, so no \
                 release could be verified. Nothing was downloaded."
                    .to_owned(),
                cx,
            ),
            Ok(_) => {}
            Err(error) if manual => notify(format!("Could not check for updates: {error:#}"), cx),
            Err(error) => log::warn!("fyx update check failed: {error:#}"),
        });
    })
    .detach();
}

struct UpdateNotification;

fn notify(message: String, cx: &mut App) {
    show_app_notification(
        NotificationId::unique::<UpdateNotification>(),
        cx,
        move |cx| {
            let message = message.clone();
            cx.new(|cx| MessageNotification::new(message, cx))
        },
    );
}

fn notify_update(version: Version, artifact: Artifact, notes_url: String, cx: &mut App) {
    show_app_notification(
        NotificationId::unique::<UpdateNotification>(),
        cx,
        move |cx| {
            let artifact = artifact.clone();
            let notes_url = notes_url.clone();
            let version = version.clone();
            cx.new(|cx| {
                MessageNotification::new(format!("Fyx {version} is available."), cx)
                    .primary_message("Install")
                    .primary_on_click(move |_, cx| {
                        install(version.clone(), artifact.clone(), cx);
                    })
                    .secondary_message("What's New")
                    .secondary_on_click(move |_, cx| cx.open_url(&notes_url))
            })
        },
    );
}

/// Downloads the build and checks it against the signed manifest. Swapping it in on restart is
/// not wired up yet, so this says where the verified file is.
fn install(version: Version, artifact: Artifact, cx: &mut App) {
    let http = cx.http_client();
    let task = cx.background_spawn(async move {
        check::download(http.as_ref(), &artifact, &updates_dir()).await
    });
    cx.spawn(async move |cx| {
        let downloaded = task.await;
        cx.update(|cx| match downloaded {
            Ok(path) => {
                show_app_notification(NotificationId::unique::<UpdateNotification>(), cx, {
                    move |cx| {
                        let path = path.clone();
                        cx.new(|cx| {
                            MessageNotification::new(
                                format!(
                                    "Fyx {version} is downloaded and matches the signed release. \
                                     Installing it on restart is not built yet: install it from \
                                     {}.",
                                    path.display()
                                ),
                                cx,
                            )
                            .primary_message("Show File")
                            .primary_on_click(move |_, cx| cx.reveal_path(&path))
                        })
                    }
                });
            }
            Err(error) => notify(format!("Could not download Fyx {version}: {error:#}"), cx),
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use gpui::UpdateGlobal as _;

    use super::*;

    #[gpui::test]
    fn automatic_checks_are_opt_in(cx: &mut App) {
        assert!(!automatic_checks_enabled(cx), "off without settings");
        let store = SettingsStore::test(cx);
        cx.set_global(store);
        assert!(!automatic_checks_enabled(cx), "off by default");
        SettingsStore::update_global(cx, |store, cx| {
            store
                .set_user_settings(r#"{ "fyx": { "updates": { "check": true } } }"#, cx)
                .result()
                .expect("valid settings");
        });
        assert!(automatic_checks_enabled(cx));
    }

    #[test]
    fn this_build_names_a_target_triple() {
        let target = current_target();
        assert!(target.split('-').count() >= 3, "{target}");
    }
}
