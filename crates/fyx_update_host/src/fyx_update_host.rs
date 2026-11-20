//! Fyx self-update in the app (plan 6.2, `docs/fyx/platform/self-update.md`): the opt-in
//! `fyx.updates.check` setting (off by default), the "fyx: check for updates" action and the
//! notification that offers a new version with "Install". Nothing is downloaded before "Install",
//! and every check goes through `fyx_update`'s signature, channel, freshness and downgrade checks.
//!
//! Without a release key (`fyx_update::RELEASE_KEYS` is empty until the owner makes one), a check
//! fetches nothing and says that updates are not set up in this build.
//!
//! "Install" downloads and verifies the build and unpacks it beside the installed app; "Restart to
//! Update" swaps it in with `fyx_update`'s rename plan (on Windows a helper does it after Fyx
//! exits) and restarts. A new version that twice fails to stay up for a minute is rolled back
//! on the next start; "fyx: roll back update" does it on request. Dev builds never swap anything.

mod check;
mod install;

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
        CheckForUpdates,
        /// Restores the version Fyx had before its last update.
        RollBackUpdate
    ]
);

/// How long after start the first automatic check runs, and how often after that.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long a new version must stay up to count as started.
const STARTED_AFTER: Duration = Duration::from_secs(60);

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
    cx.on_action(|_: &RollBackUpdate, cx| roll_back(cx));
    watch_start(cx);
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

/// Downloads the build, checks it against the signed manifest and unpacks it beside the
/// installed app; then offers "Restart to Update".
fn install(version: Version, artifact: Artifact, cx: &mut App) {
    let layout = match current_layout(cx) {
        Ok(layout) => layout,
        Err(error) => {
            notify(
                format!("Fyx {version} cannot be installed here: {error:#}"),
                cx,
            );
            return;
        }
    };
    let http = cx.http_client();
    let staging = layout.clone();
    let task = cx.background_spawn(async move {
        let path = check::download(http.as_ref(), &artifact, &updates_dir()).await?;
        let bytes = std::fs::read(&path)
            .map_err(|error| anyhow::anyhow!("reading {}: {error}", path.display()))?;
        install::stage(bytes, artifact.kind, &staging).await?;
        std::fs::remove_file(&path)
            .map_err(|error| anyhow::anyhow!("removing {}: {error}", path.display()))
    });
    cx.spawn(async move |cx| {
        let staged = task.await;
        cx.update(|cx| match staged {
            Ok(()) => {
                show_app_notification(NotificationId::unique::<UpdateNotification>(), cx, {
                    move |cx| {
                        let version = version.clone();
                        let layout = layout.clone();
                        cx.new(|cx| {
                            MessageNotification::new(
                                format!(
                                    "Fyx {version} is downloaded and matches the signed release."
                                ),
                                cx,
                            )
                            .primary_message("Restart to Update")
                            .primary_on_click(move |_, cx| {
                                restart_into(Swap::Install(version.clone()), &layout, cx)
                            })
                        })
                    }
                });
            }
            Err(error) => notify(format!("Could not install Fyx {version}: {error:#}"), cx),
        });
    })
    .detach();
}

/// The installed app this Fyx runs from, with its staged and previous folders. Refused for dev
/// builds, which run from a build folder.
fn current_layout(cx: &App) -> anyhow::Result<fyx_update::Layout> {
    if ReleaseChannel::global(cx) == ReleaseChannel::Dev {
        anyhow::bail!("dev builds are never replaced by an update");
    }
    let exe = std::env::current_exe()?;
    let installed = install::installed_location(&exe, std::env::consts::OS)
        .ok_or_else(|| anyhow::anyhow!("{} is not inside an installed Fyx", exe.display()))?;
    Ok(fyx_update::Layout::beside(&installed))
}

enum Swap {
    Install(Version),
    RollBack,
}

/// Swaps the staged (install) or previous (rollback) version in and restarts into it. On Windows
/// the running exe cannot be moved, so a helper does the renames after Fyx exits.
fn restart_into(swap: Swap, layout: &fyx_update::Layout, cx: &mut App) {
    let (steps, staged_needed) = match &swap {
        Swap::Install(_) => (fyx_update::install_plan(layout), true),
        Swap::RollBack => (fyx_update::rollback_plan(layout), false),
    };
    let prepared = install::check_ready(layout, staged_needed).and_then(|()| match &swap {
        Swap::Install(version) => install::write_pending(
            &updates_dir(),
            &install::Pending {
                version: version.clone(),
                installed: layout.installed.clone(),
                failed_starts: 0,
            },
        ),
        Swap::RollBack => install::mark_started(&updates_dir()),
    });
    if let Err(error) = prepared {
        notify(format!("Could not update Fyx: {error:#}"), cx);
        return;
    }
    if cfg!(windows) {
        match spawn_windows_helper(&steps, layout) {
            Ok(()) => cx.quit(),
            Err(error) => {
                forget_pending();
                notify(format!("Could not start the update helper: {error:#}"), cx);
            }
        }
        return;
    }
    match install::run_steps(&steps) {
        Ok(()) => cx.restart(),
        Err(error) => {
            forget_pending();
            notify(
                format!("Could not update Fyx, nothing changed: {error:#}"),
                cx,
            );
        }
    }
}

fn forget_pending() {
    if let Err(error) = install::mark_started(&updates_dir()) {
        log::error!("fyx update: {error:#}");
    }
}

/// Writes the helper script into the updates folder and starts it, detached, with this process's
/// id, so it waits for Fyx to exit, runs `steps` and starts Fyx again.
fn spawn_windows_helper(
    steps: &[fyx_update::Step],
    layout: &fyx_update::Layout,
) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let relaunch = layout.installed.join(exe.strip_prefix(&layout.installed)?);
    let dir = updates_dir();
    std::fs::create_dir_all(&dir)?;
    let script = install::windows_helper_script(
        std::process::id(),
        steps,
        Some(&relaunch),
        &dir.join("helper.log"),
    );
    let script_path = dir.join("helper.ps1");
    std::fs::write(&script_path, script)?;
    util::command::new_command("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .spawn()?;
    Ok(())
}

/// At start: counts a start of a just-installed version, rolls back one that failed to start too
/// often, and clears the record once the new version has been up for [`STARTED_AFTER`].
fn watch_start(cx: &mut App) {
    let running = AppVersion::global(cx);
    let decision = match install::on_start(&updates_dir(), &running) {
        Ok(decision) => decision,
        Err(error) => {
            log::error!("fyx update: {error:#}");
            return;
        }
    };
    match decision {
        install::StartDecision::Nothing => {}
        install::StartDecision::Watch(pending) => {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(STARTED_AFTER).await;
                forget_pending();
                cx.update(|cx| notify(format!("Fyx is updated to {}.", pending.version), cx));
            })
            .detach();
        }
        install::StartDecision::RollBack(pending) => {
            let layout = fyx_update::Layout::beside(&pending.installed);
            notify(
                format!(
                    "Fyx {} did not start properly twice, so the previous version is restored.",
                    pending.version
                ),
                cx,
            );
            restart_into(Swap::RollBack, &layout, cx);
        }
        install::StartDecision::NotInstalled(pending) => notify(
            format!(
                "Fyx {} was not installed; this is still {running}. See {} for details.",
                pending.version,
                updates_dir().join("helper.log").display()
            ),
            cx,
        ),
    }
}

/// "fyx: roll back update": restores the previous version if one is kept.
fn roll_back(cx: &mut App) {
    match current_layout(cx) {
        Ok(layout) if layout.previous.is_dir() => restart_into(Swap::RollBack, &layout, cx),
        Ok(_) => notify(
            "There is no previous version of Fyx to go back to.".to_owned(),
            cx,
        ),
        Err(error) => notify(format!("Cannot roll back here: {error:#}"), cx),
    }
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
