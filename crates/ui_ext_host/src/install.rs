//! Dev installs (plan 1.2, spec decision 6): `fyx_ui: install dev UI extension` picks a folder,
//! shows the capabilities its `extension.toml` asks for, and adds the folder to
//! `fyx.extensions.dev_paths` in the user's settings. The registry loads every folder there on each
//! start and reloads it when its component is rebuilt; `FYX_UI_DEV_EXTENSIONS` stays as an override
//! for scripts. `fyx_ui: remove dev UI extension` takes a folder off the list.
//!
//! Nothing is linked or copied: the folder stays where it is. (Earlier builds linked it into the
//! installed-extensions directory with `cmd /C mklink /J`; such links still load.)

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, bail};
use fs::Fs;
use gpui::{App, Context, PathPromptOptions, PromptLevel, Window, actions};
use settings::{RegisterSetting, Settings};
use ui_ext_wasm::{CAPABILITY_PANEL, CAPABILITY_STATUS_BAR, UiManifest, permissions::tier_of};
use workspace::{Toast, Workspace, notifications::NotificationId};

use crate::{UiExtensionRegistry, consent::ConsentRecord};

actions!(
    fyx_ui,
    [
        /// Adds a folder whose extension.toml has a [fyx_ui] table to the dev UI extensions.
        InstallDevUiExtension,
        /// Takes a folder off the dev UI extensions.
        RemoveDevUiExtension,
    ]
);

/// `fyx.extensions` in settings (schema in `settings_content::FyxExtensionsSettingsContent`,
/// default in `assets/settings/fyx.json`).
#[derive(Clone, Debug, Default, RegisterSetting)]
pub struct FyxExtensionsSettings {
    pub dev_paths: Vec<PathBuf>,
    pub developer_mode: bool,
    pub hidden_status_items: Vec<String>,
    pub registry_url: String,
}

impl Settings for FyxExtensionsSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            dev_paths: content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.extensions.as_ref())
                .and_then(|extensions| extensions.dev_paths.as_ref())
                .map(|paths| paths.iter().map(PathBuf::from).collect())
                .unwrap_or_default(),
            developer_mode: content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.extensions.as_ref())
                .and_then(|extensions| extensions.developer_mode)
                .unwrap_or(false),
            hidden_status_items: content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.extensions.as_ref())
                .and_then(|extensions| extensions.hidden_status_items.clone())
                .unwrap_or_default(),
            registry_url: content
                .fyx
                .as_ref()
                .and_then(|fyx| fyx.extensions.as_ref())
                .and_then(|extensions| extensions.registry_url.clone())
                .unwrap_or_default(),
        }
    }
}

/// The dev paths in settings, or none before settings are set up.
pub fn dev_paths_setting(cx: &App) -> Vec<PathBuf> {
    if cx.has_global::<settings::SettingsStore>() {
        FyxExtensionsSettings::get_global(cx).dev_paths.clone()
    } else {
        Vec::new()
    }
}

/// Whether `fyx.extensions.hidden_status_items` names this status item (triage-3, UX F22).
pub fn status_item_hidden(key: &str, cx: &App) -> bool {
    cx.has_global::<settings::SettingsStore>()
        && FyxExtensionsSettings::get_global(cx)
            .hidden_status_items
            .iter()
            .any(|hidden| hidden == key)
}

/// Adds a status item to `fyx.extensions.hidden_status_items`, or with `None` empties it.
pub fn set_status_item_hidden(key: Option<String>, cx: &App) {
    let fs = <dyn fs::Fs>::global(cx);
    settings::update_settings_file(fs, cx, move |settings, _| {
        let hidden = settings
            .fyx
            .get_or_insert_default()
            .extensions
            .get_or_insert_default()
            .hidden_status_items
            .get_or_insert_default();
        match &key {
            Some(key) if !hidden.contains(key) => hidden.push(key.clone()),
            Some(_) => {}
            None => hidden.clear(),
        }
    });
}

/// `fyx.extensions.developer_mode`, or false before settings are set up.
pub fn developer_mode_setting(cx: &App) -> bool {
    cx.has_global::<settings::SettingsStore>()
        && FyxExtensionsSettings::get_global(cx).developer_mode
}

/// What each capability lets the extension do, for the install confirmation.
pub fn describe_capabilities(manifest: &UiManifest) -> Vec<String> {
    describe_capability_ids(&manifest.capabilities)
}

/// What a capability lets an extension do, in plain words (triage-3, UX F17). The manifest
/// name follows in brackets for authors.
pub fn describe_capability_ids(capabilities: &[String]) -> Vec<String> {
    if capabilities.is_empty() {
        return vec!["Nothing beyond its own UI".into()];
    }
    capabilities
        .iter()
        .map(|capability| match capability.as_str() {
            CAPABILITY_PANEL => format!("Adds panels to your docks ({capability})"),
            CAPABILITY_STATUS_BAR => format!("Adds items to the status bar ({capability})"),
            "ui:modal" => format!("Opens dialogs after you click or press Enter ({capability})"),
            "workspace:read-buffers" => {
                format!("Reads your open files (asks once, here; you can revoke it) ({capability})")
            }
            "workspace:read-files" => format!(
                "Reads files in your projects (asks once, here; you can revoke it) ({capability})"
            ),
            "workspace:diagnostics" => format!(
                "Reads errors and warnings (asks once, here; you can revoke it) ({capability})"
            ),
            "network:fetch" => format!(
                "Connects to the internet, to the hosts below (asks the first time) ({capability})"
            ),
            "process:spawn" => {
                format!("Runs the programs below (asks the first time) ({capability})")
            }
            "workspace:edit" => {
                format!("Edits files in your projects (asks the first time) ({capability})")
            }
            "agent:observe" => {
                format!("Watches agent sessions (asks the first time) ({capability})")
            }
            other if other.starts_with("network:fetch@") => {
                format!("Connects to {}", &other["network:fetch@".len()..])
            }
            other if other.starts_with("process:spawn@") => {
                format!("Runs {}", &other["process:spawn@".len()..])
            }
            other => match tier_of(other) {
                Some(_) => format!("Not used by this version of Fyx ({other})"),
                None => format!("Unknown to this version of Fyx ({other})"),
            },
        })
        .collect()
}

/// A capability as a short chip label.
pub fn capability_chip_label(capability: &str) -> String {
    match capability {
        CAPABILITY_PANEL => "Panels".into(),
        CAPABILITY_STATUS_BAR => "Status Bar Items".into(),
        "ui:modal" => "Dialogs".into(),
        "workspace:read-buffers" => "Reads Open Files".into(),
        "workspace:read-files" => "Reads Project Files".into(),
        "workspace:diagnostics" => "Reads Diagnostics".into(),
        "network:fetch" => "Network".into(),
        "process:spawn" => "Runs Programs".into(),
        "workspace:edit" => "Edits Files".into(),
        "agent:observe" => "Watches Agents".into(),
        other => match other.split_once('@') {
            Some(("network:fetch", domain)) => domain.to_string(),
            Some(("process:spawn", program)) => format!("Runs {program}"),
            _ => other.to_string(),
        },
    }
}

/// Checks that `source` is a UI extension with a built component: its `extension.toml` has a
/// `[fyx_ui]` table with a valid id, a supported API version and a component path inside the
/// folder (`UiManifest::read`), and the component exists.
pub async fn check_dev_folder(source: &Path, fs: &dyn Fs) -> Result<(Arc<str>, UiManifest)> {
    let Some((extension_id, manifest)) = UiManifest::read(fs, source).await? else {
        bail!(
            "{} is not a UI extension: its extension.toml has no [fyx_ui] table",
            source.display()
        );
    };
    let component = source.join(&manifest.component);
    if fs.metadata(&component).await?.is_none() {
        bail!(
            "{extension_id}: the component {} is not built yet",
            component.display()
        );
    }
    Ok((extension_id, manifest))
}

fn toast(workspace: &mut Workspace, message: String, cx: &mut Context<Workspace>) {
    workspace.show_toast(
        Toast::new(NotificationId::named("fyx-ui-install".into()), message),
        cx,
    );
}

/// The action: pick a folder, confirm with its capabilities, add it to the dev paths.
pub(crate) fn prompt_and_install(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let fs = workspace.app_state().fs.clone();
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Install Dev UI Extension".into()),
    });
    cx.spawn_in(window, async move |workspace, cx| {
        let result: Result<Option<crate::store_actions::Outcome>> = async {
            let Some(source) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                return Ok(None);
            };
            let source = fs.canonicalize(&source).await.unwrap_or(source);
            let (extension_id, manifest) = check_dev_folder(&source, fs.as_ref()).await?;
            let detail = format!(
                "It asks for:\n{}\n\nComponent: {}\nFolder: {}\n\nFyx loads it on every start and \
                 reloads it when the component is rebuilt.",
                describe_capabilities(&manifest).join("\n"),
                manifest.component,
                source.display()
            );
            let answer = workspace.update_in(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &format!("Install the dev UI extension {extension_id}?"),
                    Some(&detail),
                    &["Install", "Cancel"],
                    cx,
                )
            })?;
            if answer.await != Ok(0) {
                return Ok(None);
            }
            // The user just confirmed these capabilities for this folder.
            let record = ConsentRecord::current(
                &source,
                &manifest.consent_items(),
                &source.join(&manifest.component),
                true,
                fs.as_ref(),
                cx.background_executor(),
            )
            .await?;
            let waiter = cx.update(|_, cx| crate::store_actions::expect_load(&extension_id, cx))?;
            cx.update(|_, cx| {
                if let Some(registry) = UiExtensionRegistry::global(cx) {
                    registry.update(cx, |registry, cx| {
                        registry.grant_consent(&extension_id, record, fs.clone(), cx)
                    });
                }
                // The registry follows the setting.
                let path = source.to_string_lossy().into_owned();
                settings::update_settings_file(fs.clone(), cx, move |settings, _| {
                    let paths = settings
                        .fyx
                        .get_or_insert_default()
                        .extensions
                        .get_or_insert_default()
                        .dev_paths
                        .get_or_insert_default();
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                });
            })?;
            // The settings change starts the load; report after it has an outcome.
            let loaded = crate::store_actions::load_outcome(waiter, cx).await;
            Ok(Some(crate::store_actions::installed_outcome(
                format!("Installed the dev UI extension {extension_id}"),
                loaded,
            )))
        }
        .await;
        let outcome = match result {
            Ok(None) => return,
            Ok(Some(outcome)) => outcome,
            Err(error) => crate::store_actions::Outcome {
                message: format!("Could not install the dev UI extension: {error:#}"),
                failed: true,
            },
        };
        workspace
            .update(cx, |workspace, cx| {
                crate::store_actions::show_outcome(workspace, outcome, cx)
            })
            .ok();
    })
    .detach();
}

/// Most folders the removal prompt offers; the prompt is a stopgap until the Extensions page tab.
const MAX_PICKER_CHOICES: usize = 8;

/// The action: pick one of the dev paths and take it off the list.
pub(crate) fn prompt_and_remove(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let fs = workspace.app_state().fs.clone();
    let paths: Vec<PathBuf> = dev_paths_setting(cx)
        .into_iter()
        .take(MAX_PICKER_CHOICES)
        .collect();
    if paths.is_empty() {
        toast(
            workspace,
            "No dev UI extension is installed (fyx.extensions.dev_paths is empty)".into(),
            cx,
        );
        return;
    }
    let names: Vec<String> = paths
        .iter()
        .map(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        })
        .collect();
    let detail = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut answers: Vec<&str> = names.iter().map(String::as_str).collect();
    answers.push("Cancel");
    let answer = window.prompt(
        PromptLevel::Info,
        "Remove which dev UI extension?",
        Some(&detail),
        &answers,
        cx,
    );
    cx.spawn_in(window, async move |workspace, cx| {
        let Ok(choice) = answer.await else {
            return;
        };
        let Some(path) = paths.get(choice) else {
            return;
        };
        let removed = path.to_string_lossy().into_owned();
        workspace
            .update(cx, |workspace, cx| {
                settings::update_settings_file(fs.clone(), cx, {
                    let removed = removed.clone();
                    move |settings, _| {
                        if let Some(paths) = settings
                            .fyx
                            .as_mut()
                            .and_then(|fyx| fyx.extensions.as_mut())
                            .and_then(|extensions| extensions.dev_paths.as_mut())
                        {
                            paths.retain(|path| *path != removed);
                        }
                    }
                });
                toast(
                    workspace,
                    format!("Removed the dev UI extension {removed}"),
                    cx,
                );
            })
            .ok();
    })
    .detach();
}
