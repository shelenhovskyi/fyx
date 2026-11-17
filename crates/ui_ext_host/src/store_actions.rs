//! Commands over the install store (plan 1.1), until the Extensions page gets its Fyx tab:
//! install a `.fyxext` file, roll back, pin or unpin, uninstall.
//!
//! Installing and rolling back show the capabilities and record consent for exactly that version
//! (`consent`), since the user has just seen what it asks for.

use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, bail};
use fs::Fs;
use gpui::{AsyncWindowContext, Context, PathPromptOptions, PromptLevel, Window, actions};
use workspace::{Toast, Workspace, notifications::NotificationId};

use crate::{
    UiExtensionRegistry,
    consent::ConsentRecord,
    install::describe_capability_ids,
    install_store::{InstallStore, StoreState},
};

actions!(
    fyx_ui,
    [
        /// Installs or updates a UI extension from a .fyxext package.
        InstallUiExtensionFromFile,
        /// Switches a UI extension from the install store back to its previous version.
        RollBackUiExtension,
        /// Pins a UI extension from the install store to its version, or unpins it.
        TogglePinUiExtension,
        /// Removes a UI extension from the install store.
        UninstallUiExtension,
        /// Loads a UI extension again on a fresh instance, after a failure or to pick up changes.
        RestartUiExtension,
    ]
);

pub(crate) fn register(workspace: &mut Workspace) {
    workspace.register_action(|workspace, _: &InstallUiExtensionFromFile, window, cx| {
        install_from_file(workspace, window, cx)
    });
    workspace.register_action(|workspace, _: &RollBackUiExtension, window, cx| {
        run(workspace, window, cx, StoreCommand::RollBack)
    });
    workspace.register_action(|workspace, _: &TogglePinUiExtension, window, cx| {
        run(workspace, window, cx, StoreCommand::TogglePin)
    });
    workspace.register_action(|workspace, _: &UninstallUiExtension, window, cx| {
        run(workspace, window, cx, StoreCommand::Uninstall)
    });
    workspace.register_action(|_, _: &RestartUiExtension, window, cx| prompt_restart(window, cx));
}

/// Picks a loaded extension and restarts it.
fn prompt_restart(window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(registry) = UiExtensionRegistry::global(cx) else {
        return;
    };
    let mut ids: Vec<Arc<str>> = registry
        .read(cx)
        .surfaces()
        .iter()
        .map(|surface| surface.extension_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids.truncate(MAX_PICKER_CHOICES);
    if ids.is_empty() {
        return;
    }
    let mut answers: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();
    answers.push("Cancel");
    let answer = window.prompt(
        PromptLevel::Info,
        "Restart which UI extension?",
        Some("It loads again on a fresh instance and its panels and status items remount."),
        &answers,
        cx,
    );
    cx.spawn(async move |_, cx| {
        let Ok(choice) = answer.await else {
            return;
        };
        if let Some(id) = ids.get(choice) {
            registry.update(cx, |registry, cx| registry.restart_extension(id, cx));
        }
    })
    .detach();
}

fn store(cx: &gpui::App) -> Option<InstallStore> {
    UiExtensionRegistry::global(cx)?
        .read(cx)
        .install_store()
        .cloned()
}

/// Records consent for the version now active and rescans, so it loads.
async fn grant_and_rescan(
    store: &InstallStore,
    extension_id: &str,
    fs: Arc<dyn Fs>,
    cx: &mut AsyncWindowContext,
) -> Result<()> {
    let state = store.state(fs.as_ref()).await?;
    if let Some(entry) = state.extensions.get(extension_id) {
        let dir = store.version_dir(extension_id, &entry.active);
        if let Some((_, manifest)) = ui_ext_wasm::UiManifest::read(fs.as_ref(), &dir).await? {
            let record = ConsentRecord::current(
                &dir,
                &manifest.capabilities,
                &dir.join(&manifest.component),
                false,
                fs.as_ref(),
                &cx.background_executor(),
            )
            .await?;
            cx.update(|_, cx| {
                if let Some(registry) = UiExtensionRegistry::global(cx) {
                    registry.update(cx, |registry, cx| {
                        registry.grant_consent(extension_id, record, fs.clone(), cx)
                    });
                }
            })?;
        }
    }
    cx.update(|_, cx| {
        if let Some(registry) = UiExtensionRegistry::global(cx) {
            registry.update(cx, |registry, cx| registry.rescan(cx));
        }
    })?;
    Ok(())
}

fn toast(workspace: &mut Workspace, message: String, cx: &mut Context<Workspace>) {
    workspace.show_toast(
        Toast::new(NotificationId::named("fyx-ui-store".into()), message),
        cx,
    );
}

fn install_from_file(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(store) = store(cx) else {
        return;
    };
    let fs = workspace.app_state().fs.clone();
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Install UI Extension Package".into()),
    });
    cx.spawn_in(window, async move |workspace, cx| {
        let result: Result<Option<String>> = async {
            let Some(package): Option<PathBuf> =
                paths.await??.and_then(|paths| paths.into_iter().next())
            else {
                return Ok(None);
            };
            let info = store.inspect_package(&package, fs.as_ref()).await?;
            let state = store.state(fs.as_ref()).await?;
            let current = state
                .extensions
                .get(info.extension_id.as_ref())
                .map(|entry| format!("\nInstalled now: {}", entry.active))
                .unwrap_or_default();
            let detail = format!(
                "It asks for:\n{}\n\nUnsigned, not from the registry.\nPackage: {}{current}",
                describe_capability_ids(&info.manifest.capabilities).join("\n"),
                package.display(),
            );
            let answer = workspace.update_in(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &format!("Install {} {}?", info.extension_id, info.version),
                    Some(&detail),
                    &["Install", "Cancel"],
                    cx,
                )
            })?;
            if answer.await != Ok(0) {
                return Ok(None);
            }
            let installed = store.install_package(&package, fs.as_ref()).await?;
            grant_and_rescan(&store, &installed.extension_id, fs.clone(), cx).await?;
            Ok(Some(match installed.previous {
                Some(previous) => format!(
                    "Installed {} {} (previous: {previous})",
                    installed.extension_id, installed.version
                ),
                None => format!("Installed {} {}", installed.extension_id, installed.version),
            }))
        }
        .await;
        let message = match result {
            Ok(None) => return,
            Ok(Some(message)) => message,
            Err(error) => format!("Could not install the package: {error:#}"),
        };
        workspace
            .update(cx, |workspace, cx| toast(workspace, message, cx))
            .ok();
    })
    .detach();
}

#[derive(Clone, Copy)]
enum StoreCommand {
    RollBack,
    TogglePin,
    Uninstall,
}

impl StoreCommand {
    fn applies(self, state: &StoreState, id: &str) -> bool {
        match self {
            StoreCommand::RollBack => state.extensions[id].previous.is_some(),
            StoreCommand::TogglePin | StoreCommand::Uninstall => true,
        }
    }

    fn title(self) -> &'static str {
        match self {
            StoreCommand::RollBack => "Roll back which UI extension?",
            StoreCommand::TogglePin => "Pin or unpin which UI extension?",
            StoreCommand::Uninstall => "Uninstall which UI extension?",
        }
    }
}

/// Most extensions a picker offers; the prompt is a stopgap until the Extensions page tab.
const MAX_PICKER_CHOICES: usize = 8;

fn run(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
    command: StoreCommand,
) {
    let Some(store) = store(cx) else {
        return;
    };
    let fs = workspace.app_state().fs.clone();
    cx.spawn_in(window, async move |workspace, cx| {
        let result: Result<Option<String>> = async {
            let state = store.state(fs.as_ref()).await?;
            let ids: Vec<String> = state
                .extensions
                .keys()
                .filter(|id| command.applies(&state, id))
                .take(MAX_PICKER_CHOICES)
                .cloned()
                .collect();
            if ids.is_empty() {
                bail!(match command {
                    StoreCommand::RollBack => "no installed UI extension has a previous version",
                    _ => "no UI extension is installed from a package",
                });
            }
            let describe = |id: &String| {
                let entry = &state.extensions[id];
                match command {
                    StoreCommand::RollBack => format!(
                        "{id}: {} → {}",
                        entry.active,
                        entry
                            .previous
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_default()
                    ),
                    StoreCommand::TogglePin => format!(
                        "{id} {}: {}",
                        entry.active,
                        if entry.pinned { "pinned" } else { "not pinned" }
                    ),
                    StoreCommand::Uninstall => format!("{id} {}", entry.active),
                }
            };
            let detail = ids.iter().map(describe).collect::<Vec<_>>().join("\n");
            let mut answers: Vec<&str> = ids.iter().map(String::as_str).collect();
            answers.push("Cancel");
            let answer = workspace.update_in(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    command.title(),
                    Some(&detail),
                    &answers,
                    cx,
                )
            })?;
            let Ok(choice) = answer.await else {
                return Ok(None);
            };
            let Some(id) = ids.get(choice) else {
                return Ok(None);
            };
            let message = match command {
                StoreCommand::RollBack => {
                    let version = store.rollback(id, fs.as_ref()).await?;
                    grant_and_rescan(&store, id, fs.clone(), cx).await?;
                    format!("Rolled {id} back to {version}")
                }
                StoreCommand::TogglePin => {
                    let pinned = !state.extensions[id].pinned;
                    store.set_pinned(id, pinned, fs.as_ref()).await?;
                    if pinned {
                        format!("Pinned {id} to {}", state.extensions[id].active)
                    } else {
                        format!("Unpinned {id}")
                    }
                }
                StoreCommand::Uninstall => {
                    store.uninstall(id, fs.as_ref()).await?;
                    cx.update(|_, cx| {
                        if let Some(registry) = UiExtensionRegistry::global(cx) {
                            registry.update(cx, |registry, cx| registry.rescan(cx));
                        }
                    })?;
                    format!("Uninstalled {id}")
                }
            };
            Ok(Some(message))
        }
        .await;
        let message = match result {
            Ok(None) => return,
            Ok(Some(message)) => message,
            Err(error) => format!("{error:#}"),
        };
        workspace
            .update(cx, |workspace, cx| toast(workspace, message, cx))
            .ok();
    })
    .detach();
}
