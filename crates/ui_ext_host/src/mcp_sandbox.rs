//! Plan 7.5: the opt-in macOS sandbox for stdio MCP servers (`fyx.mcp_sandbox`). Installed into
//! `context_server::fyx_sandbox`; the profile comes from `ui_ext::seatbelt`.
//!
//! An opted-in server reads and writes its project folder and its own temporary folder, reads the
//! rest of the home folder and the system, and reaches any host: its writes are confined, its
//! reads and network are not. Servers are not declared like extensions, so a stricter default
//! would break most of them; per-server entries add writable or readable folders, or turn the
//! sandbox on or off for one server.

use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use collections::HashMap;
use gpui::App;
use settings::{RegisterSetting, Settings, SettingsStore};
use ui_ext::seatbelt::{SpawnNetwork, SpawnSandbox};

/// Project paths a server may not write even inside its project: they run later outside the
/// sandbox (Git hooks and config, editor tasks and settings, direnv).
const PROTECTED_IN_PROJECT: &[&str] = &[
    ".git/hooks",
    ".git/config",
    ".zed",
    ".fyx",
    ".vscode",
    ".idea",
    ".envrc",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerSandbox {
    pub enabled: Option<bool>,
    pub writable: Vec<String>,
    pub readable: Vec<String>,
}

/// `fyx.mcp_sandbox`.
#[derive(Clone, Debug, Default, PartialEq, Eq, RegisterSetting)]
pub struct McpSandboxSettings {
    pub enabled: bool,
    pub servers: HashMap<String, ServerSandbox>,
}

impl Settings for McpSandboxSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let Some(sandbox) = content
            .fyx
            .as_ref()
            .and_then(|fyx| fyx.mcp_sandbox.as_ref())
        else {
            return Self::default();
        };
        Self {
            enabled: sandbox.enabled.unwrap_or(false),
            servers: sandbox
                .servers
                .iter()
                .flatten()
                .map(|(name, server)| {
                    (
                        name.clone(),
                        ServerSandbox {
                            enabled: server.enabled,
                            writable: server.writable.clone().unwrap_or_default(),
                            readable: server.readable.clone().unwrap_or_default(),
                        },
                    )
                })
                .collect(),
        }
    }
}

impl McpSandboxSettings {
    /// Whether `server` starts sandboxed: its own entry, else the global switch.
    pub fn applies_to(&self, server: &str) -> bool {
        self.servers
            .get(server)
            .and_then(|server| server.enabled)
            .unwrap_or(self.enabled)
    }
}

/// `~/x` under `home`; other paths must be absolute. Anything else is dropped.
fn expand(path: &str, home: &Path) -> Option<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    if path == "~" {
        return Some(home.to_path_buf());
    }
    let path = PathBuf::from(path);
    path.is_absolute().then_some(path)
}

/// What `server`, started in `project` (canonical, when it has one), may touch, or `None` when it
/// starts unsandboxed. `home` and `temp` (its own temporary folder) are canonical; `canonical`
/// resolves the listed folders, which are dropped when missing.
pub fn plan(
    server: &str,
    project: Option<&Path>,
    settings: &McpSandboxSettings,
    home: &Path,
    temp: &Path,
    canonical: impl Fn(&Path) -> Option<PathBuf>,
) -> Option<SpawnSandbox> {
    if !settings.applies_to(server) {
        return None;
    }
    let entry = settings.servers.get(server);
    let listed = |paths: Option<&Vec<String>>| -> Vec<PathBuf> {
        paths
            .into_iter()
            .flatten()
            .filter_map(|path| expand(path, home))
            .filter_map(|path| canonical(&path))
            .collect()
    };
    let mut readable = vec![home.to_path_buf()];
    readable.extend(listed(entry.map(|entry| &entry.readable)));
    let mut writable: Vec<PathBuf> = project.map(Path::to_path_buf).into_iter().collect();
    writable.push(temp.to_path_buf());
    writable.extend(listed(entry.map(|entry| &entry.writable)));
    Some(SpawnSandbox {
        readable,
        writable,
        protected: project
            .into_iter()
            .flat_map(|project| {
                PROTECTED_IN_PROJECT
                    .iter()
                    .map(move |relative| project.join(relative))
            })
            .collect(),
        network: SpawnNetwork::Open,
    })
}

/// The settings the wrapper reads: it runs off the main thread, without an `App`.
static CURRENT: RwLock<Option<McpSandboxSettings>> = RwLock::new(None);

fn store(settings: McpSandboxSettings) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(settings);
}

pub(crate) fn init(cx: &mut App) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let read = |cx: &App| {
        if cx.has_global::<SettingsStore>() {
            McpSandboxSettings::get_global(cx).clone()
        } else {
            McpSandboxSettings::default()
        }
    };
    store(read(cx));
    cx.observe_global::<SettingsStore>(move |cx| store(read(cx)))
        .detach();
    #[cfg(target_os = "macos")]
    context_server::fyx_sandbox::set_stdio_command_wrapper(Some(std::sync::Arc::new(macos::wrap)));
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::process::Command;

    use anyhow::{Context as _, Result};
    use context_server::fyx_sandbox::StdioServer;

    use super::{CURRENT, PoisonError};

    pub(super) fn wrap(command: Command, server: StdioServer<'_>) -> Result<Command> {
        let settings = CURRENT
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .unwrap_or_default();
        if !settings.applies_to(server.id) {
            return Ok(command);
        }
        let canonical = |path: &std::path::Path| std::fs::canonicalize(path).ok();
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .and_then(|home| canonical(&home))
            .context("fyx: no home folder for the MCP server sandbox")?;
        let project = server.working_directory.and_then(canonical);
        let temp = std::env::temp_dir().join(format!(
            "fyx-mcp-{}-{}",
            std::process::id(),
            server
                .id
                .replace(|character: char| !character.is_ascii_alphanumeric(), "_")
        ));
        std::fs::create_dir_all(&temp)
            .with_context(|| format!("fyx: the MCP server's folder {}", temp.display()))?;
        let temp = canonical(&temp).context("fyx: the MCP server's folder is gone")?;
        let Some(plan) = super::plan(
            server.id,
            project.as_deref(),
            &settings,
            &home,
            &temp,
            canonical,
        ) else {
            return Ok(command);
        };
        let profile = ui_ext::seatbelt::profile(&plan, &[]);
        let mut sandboxed = Command::new("/usr/bin/sandbox-exec");
        sandboxed
            .arg("-p")
            .arg(profile)
            .arg(command.get_program())
            .args(command.get_args());
        for (name, value) in command.get_envs() {
            match value {
                Some(value) => sandboxed.env(name, value),
                None => sandboxed.env_remove(name),
            };
        }
        sandboxed.env("TMPDIR", OsString::from(temp.as_os_str()));
        if let Some(directory) = command.get_current_dir() {
            sandboxed.current_dir(directory);
        }
        log::info!("fyx: MCP server {} starts sandboxed", server.id);
        Ok(sandboxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(enabled: bool) -> McpSandboxSettings {
        McpSandboxSettings {
            enabled,
            servers: HashMap::default(),
        }
    }

    fn plan_for(server: &str, settings: &McpSandboxSettings) -> Option<SpawnSandbox> {
        plan(
            server,
            Some(Path::new("/work/project")),
            settings,
            Path::new("/home/me"),
            Path::new("/tmp/fyx-mcp-1-files"),
            |path| Some(path.to_path_buf()),
        )
    }

    #[test]
    fn off_by_default_and_on_with_the_switch() {
        assert_eq!(plan_for("files", &McpSandboxSettings::default()), None);
        let sandbox = plan_for("files", &settings(true)).expect("the switch is on");
        assert_eq!(sandbox.readable, [PathBuf::from("/home/me")]);
        assert_eq!(
            sandbox.writable,
            [
                PathBuf::from("/work/project"),
                PathBuf::from("/tmp/fyx-mcp-1-files")
            ]
        );
        assert!(
            sandbox
                .protected
                .contains(&PathBuf::from("/work/project/.git/hooks"))
        );
        assert_eq!(sandbox.network, SpawnNetwork::Open);
    }

    #[test]
    fn a_server_entry_overrides_the_switch_and_adds_folders() {
        let mut on = settings(true);
        on.servers.insert(
            "github".into(),
            ServerSandbox {
                enabled: Some(false),
                ..ServerSandbox::default()
            },
        );
        // Absolute on every OS (`/opt/data` is not absolute on Windows).
        let data = std::env::temp_dir().join("fyx-mcp-data");
        on.servers.insert(
            "files".into(),
            ServerSandbox {
                enabled: None,
                writable: vec!["~/.npm".into(), "relative".into()],
                readable: vec![data.to_string_lossy().into_owned()],
            },
        );
        assert_eq!(plan_for("github", &on), None);
        let files = plan_for("files", &on).expect("files follows the switch");
        assert!(files.writable.contains(&Path::new("/home/me").join(".npm")));
        assert!(!files.writable.iter().any(|path| path.ends_with("relative")));
        assert!(files.readable.contains(&data));

        let mut off = settings(false);
        off.servers.insert(
            "files".into(),
            ServerSandbox {
                enabled: Some(true),
                ..ServerSandbox::default()
            },
        );
        assert!(plan_for("files", &off).is_some());
        assert_eq!(plan_for("other", &off), None);
    }

    #[test]
    fn missing_folders_are_dropped_and_a_server_without_a_project_writes_only_its_temp() {
        let mut on = settings(true);
        on.servers.insert(
            "files".into(),
            ServerSandbox {
                enabled: None,
                writable: vec!["~/gone".into()],
                readable: Vec::new(),
            },
        );
        let sandbox = plan(
            "files",
            None,
            &on,
            Path::new("/home/me"),
            Path::new("/tmp/t"),
            |path| (!path.ends_with("gone")).then(|| path.to_path_buf()),
        )
        .expect("the switch is on");
        assert_eq!(sandbox.writable, [PathBuf::from("/tmp/t")]);
        assert!(sandbox.protected.is_empty());
    }

    #[test]
    fn the_profile_confines_writes_and_opens_the_network() {
        let sandbox = plan_for("files", &settings(true)).expect("the switch is on");
        let profile = ui_ext::seatbelt::profile(&sandbox, &[]);
        assert!(profile.contains("(allow network*)"));
        assert!(profile.contains("(subpath \"/work/project\")"));
        assert!(profile.contains("(deny file-write*"));
    }
}
