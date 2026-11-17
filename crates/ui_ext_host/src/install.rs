//! Installing a UI extension from a folder: `fyx_ui: install dev UI extension`.
//!
//! The user picks a folder; its `extension.toml` must have a `[fyx_ui]` table and the component
//! it names must be built. A confirmation lists the capabilities the extension asks for. The
//! folder is then linked into the installed-extensions directory as `<id>`, the way Zed links a
//! dev extension, so the registry's scan loads it now and on every start, and the extension
//! store lists it.

use std::{path::Path, sync::Arc};

use anyhow::{Context as _, Result, bail};
use extension_host::ExtensionStore;
use fs::{Fs, RemoveOptions};
use gpui::{Context, PathPromptOptions, PromptLevel, Window, actions};
use ui_ext_wasm::{CAPABILITY_PANEL, CAPABILITY_STATUS_BAR, UiManifest};
use workspace::{Toast, Workspace, notifications::NotificationId};

use crate::{UiExtensionRegistry, consent::ConsentRecord};

actions!(
    fyx_ui,
    [
        /// Installs a UI extension from a folder whose extension.toml has a [fyx_ui] table.
        InstallDevUiExtension,
    ]
);

/// What each capability lets the extension do, for the install confirmation.
pub fn describe_capabilities(manifest: &UiManifest) -> Vec<String> {
    describe_capability_ids(&manifest.capabilities)
}

pub fn describe_capability_ids(capabilities: &[String]) -> Vec<String> {
    if capabilities.is_empty() {
        return vec!["none".into()];
    }
    capabilities
        .iter()
        .map(|capability| match capability.as_str() {
            CAPABILITY_PANEL => format!("{capability}: add dock panels"),
            CAPABILITY_STATUS_BAR => format!("{capability}: add status bar items"),
            other => format!("{other}: unknown to this version"),
        })
        .collect()
}

/// Characters `cmd` treats specially. `Fs::create_symlink` makes a junction with
/// `cmd /C mklink /J <link> <target>` on Windows, and Rust quotes an argument only when it has a
/// space, tab or quote, so a path with one of these could run a command. Ids cannot contain them
/// (`ui_ext_wasm::validate_id`); the folder the user picked and the installed directory can.
const CMD_SPECIAL: &[char] = &['&', '|', '^', '<', '>', '%', '!', '"'];

/// The first character of `path` that `cmd` would interpret, if any.
pub fn cmd_special_char(path: &Path) -> Option<char> {
    path.to_string_lossy()
        .chars()
        .find(|char| CMD_SPECIAL.contains(char))
}

/// The link goes directly inside `installed_dir`, named after the id; never elsewhere, whatever
/// the id. `UiManifest::read` already validates the id, so this is a second line.
fn link_target(installed_dir: &Path, extension_id: &str) -> Result<std::path::PathBuf> {
    let target = installed_dir.join(extension_id);
    if target.parent() != Some(installed_dir)
        || target.file_name().and_then(|name| name.to_str()) != Some(extension_id)
    {
        bail!(
            "extension id {extension_id:?} does not name a folder inside the installed directory"
        );
    }
    Ok(target)
}

/// Links `source` into `installed_dir` as the extension's id, after checking that it is a UI
/// extension with a built component. A previous dev install (a link) of the same id is replaced;
/// a regular install of it is left alone.
pub async fn install_dev_ui_extension(
    source: &Path,
    installed_dir: &Path,
    fs: &dyn Fs,
) -> Result<Arc<str>> {
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
    let target = link_target(installed_dir, &extension_id)?;
    // The link is made through `cmd` on Windows (see `CMD_SPECIAL`); check on every OS, so an
    // extension folder behaves the same everywhere.
    for path in [source, target.as_path()] {
        if let Some(char) = cmd_special_char(path) {
            bail!(
                "{} contains {char:?}, which this install cannot link safely; move or rename \
                 the folder",
                path.display()
            );
        }
    }
    fs.create_dir(installed_dir).await?;
    if let Some(metadata) = fs.metadata(&target).await? {
        if !metadata.is_symlink {
            bail!(
                "{extension_id} is already installed (not as a dev extension); uninstall it first"
            );
        }
        fs.remove_file(
            &target,
            RemoveOptions {
                recursive: false,
                ignore_if_not_exists: true,
            },
        )
        .await?;
    }
    fs.create_symlink(&target, source.to_path_buf())
        .await
        .with_context(|| {
            format!(
                "linking {} into {}",
                source.display(),
                installed_dir.display()
            )
        })?;
    Ok(extension_id)
}

/// The action: pick a folder, confirm with its capabilities, install, load.
pub(crate) fn prompt_and_install(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(store) = ExtensionStore::try_global(cx) else {
        return;
    };
    let installed_dir = store.read(cx).installed_dir.clone();
    let fs = workspace.app_state().fs.clone();
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Install UI Extension".into()),
    });
    cx.spawn_in(window, async move |workspace, cx| {
        let result: Result<Option<Arc<str>>> = async {
            let Some(source) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                return Ok(None);
            };
            let Some((extension_id, manifest)) = UiManifest::read(fs.as_ref(), &source).await?
            else {
                bail!(
                    "{} is not a UI extension: its extension.toml has no [fyx_ui] table",
                    source.display()
                );
            };
            let detail = format!(
                "It asks for:\n{}\n\nComponent: {}\nFolder: {}",
                describe_capabilities(&manifest).join("\n"),
                manifest.component,
                source.display()
            );
            let answer = workspace.update_in(cx, |_, window, cx| {
                window.prompt(
                    PromptLevel::Info,
                    &format!("Install the UI extension {extension_id}?"),
                    Some(&detail),
                    &["Install", "Cancel"],
                    cx,
                )
            })?;
            if answer.await != Ok(0) {
                return Ok(None);
            }
            let extension_id =
                install_dev_ui_extension(&source, &installed_dir, fs.as_ref()).await?;
            // The user just confirmed these capabilities for this folder.
            let link = installed_dir.join(extension_id.as_ref());
            let record = ConsentRecord::current(
                &link,
                &manifest.capabilities,
                &link.join(&manifest.component),
                true,
                fs.as_ref(),
                &cx.background_executor(),
            )
            .await?;
            cx.update(|_, cx| {
                if let Some(registry) = UiExtensionRegistry::global(cx) {
                    registry.update(cx, |registry, cx| {
                        registry.grant_consent(&extension_id, record, fs.clone(), cx);
                        registry.scan_installed(installed_dir.clone(), fs.clone(), cx)
                    });
                }
            })?;
            Ok(Some(extension_id))
        }
        .await;
        let message = match result {
            Ok(None) => return,
            Ok(Some(extension_id)) => format!("Installed the UI extension {extension_id}"),
            Err(error) => format!("Could not install the UI extension: {error:#}"),
        };
        workspace
            .update(cx, |workspace, cx| {
                workspace.show_toast(
                    Toast::new(NotificationId::named("fyx-ui-install".into()), message),
                    cx,
                )
            })
            .ok();
    })
    .detach();
}
