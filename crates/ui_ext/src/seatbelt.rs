//! Plan 7.5 (macOS): the Seatbelt profile a `process:spawn` child runs under. Pure text, so it is
//! built and tested on every OS; `ui_ext_wasm` runs it with `/usr/bin/sandbox-exec -p` on macOS
//! only.
//!
//! Unlike `crates/sandbox` (the agent's terminal tool), which lets a command read everything and
//! only confines writes, this profile denies by default:
//!
//! - **Reads:** the system and toolchain locations ([`SYSTEM_READ_PATHS`], the toolchain homes
//!   the caller found) and the paths in [`SpawnSandbox::readable`] (the program's worktree, the
//!   program itself). Metadata (`stat`, `realpath`) is allowed everywhere, so tools that walk up
//!   to `/` still work; file contents and directory listings outside are denied.
//! - **Writes:** only [`SpawnSandbox::writable`] (the worktree, a per-spawn temporary directory)
//!   and the standard device nodes.
//! - **Network:** none, or only to Fyx's loopback proxy ([`SpawnNetwork::Proxy`]), which allows
//!   the extension's declared `[fyx_ui.network] domains` and nothing else.
//! - **Mach services:** the allowlist of `crates/sandbox`, which leaves out LaunchServices and
//!   launchd, so a child cannot start a process outside the sandbox.
//!
//! Paths must be absolute and canonical: Seatbelt matches the resolved path (`/private/var`, not
//! `/var`).

use std::path::{Path, PathBuf};

/// Where a sandboxed child may connect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnNetwork {
    /// No network at all.
    Blocked,
    /// Only Fyx's HTTP(S) proxy on `localhost:<port>`; the proxy enforces the host list.
    Proxy { port: u16 },
    /// Any host: for programs whose hosts are not declared (an opted-in MCP server's sandbox
    /// confines its files only).
    Open,
}

/// What one spawned program may touch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSandbox {
    /// Read-only subtrees besides [`SYSTEM_READ_PATHS`].
    pub readable: Vec<PathBuf>,
    /// Read-write subtrees.
    pub writable: Vec<PathBuf>,
    /// Read-only inside [`SpawnSandbox::writable`]: what runs later outside the sandbox
    /// (`.git/hooks`, editor tasks, `.envrc`).
    pub protected: Vec<PathBuf>,
    pub network: SpawnNetwork,
}

/// System locations every program may read (and execute from): the OS, the developer tools and
/// the package managers' prefixes. `/private/etc` and `/private/var/db` hold resolver, timezone
/// and certificate files; `/private/var/select` the selected shell.
pub const SYSTEM_READ_PATHS: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/System",
    "/Library/Apple",
    "/Library/Developer",
    "/Library/Frameworks",
    "/Library/Preferences",
    "/Applications/Xcode.app",
    "/opt/homebrew",
    "/opt/local",
    "/nix",
    "/private/etc",
    "/private/var/db/timezone",
    "/private/var/db/dyld",
    "/private/var/select",
    "/dev",
];

/// Toolchain directories under the home directory that a program may read when they exist:
/// version managers and language toolchains keep their binaries and libraries there. Dotfiles
/// with secrets (`.ssh`, `.aws`, `.netrc`, `.config/gh`, …) are not on the list.
pub const TOOLCHAIN_HOME_DIRS: &[&str] = &[
    ".cargo",
    ".rustup",
    ".nvm",
    ".volta",
    ".fnm",
    ".bun",
    ".deno",
    ".pyenv",
    ".rbenv",
    ".asdf",
    ".local/share/mise",
    ".sdkman",
    "go",
    ".gitconfig",
    ".config/git",
];

/// A double-quoted Seatbelt string: backslashes and quotes escaped. `None` for a path that is not
/// UTF-8 or holds a control character, which the caller leaves out (and so denies).
fn quoted(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if !path.is_absolute() && !text.starts_with('/') || text.chars().any(char::is_control) {
        return None;
    }
    Some(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn subpaths<'a>(paths: impl IntoIterator<Item = &'a Path>) -> String {
    paths
        .into_iter()
        .filter_map(quoted)
        .map(|path| format!("\n    (subpath {path})"))
        .collect()
}

const BASE: &str = r#"(version 1)
; Fyx process:spawn (plan 7.5): deny by default.
(deny default)

(allow process-exec* process-fork)
(allow signal (target same-sandbox))
(allow sysctl-read)
; stat and realpath anywhere; contents and listings only below.
(allow file-read-metadata)
(allow file-read-data (literal "/"))

; Mach services: the allowlist of crates/sandbox (no LaunchServices or launchd).
(allow mach-lookup
    (global-name "com.apple.system.opendirectoryd.libinfo")
    (global-name "com.apple.system.opendirectoryd.membership")
    (global-name "com.apple.system.DirectoryService.libinfo_v1")
    (global-name "com.apple.bsd.dirhelper")
    (global-name "com.apple.cfprefsd.daemon")
    (global-name "com.apple.cfprefsd.agent")
    (local-name "com.apple.cfprefsd.agent")
    (global-name "com.apple.logd")
    (global-name "com.apple.logd.events")
    (global-name "com.apple.system.logger")
    (global-name "com.apple.diagnosticd")
    (global-name "com.apple.system.notification_center")
    (global-name "com.apple.analyticsd")
    (global-name "com.apple.analyticsd.messagetracer")
    (global-name "com.apple.PowerManagement.control")
    (global-name "com.apple.dt.automationmode.reader"))

; Standard streams and device nodes.
(allow file-read* file-write*
    (literal "/dev/null")
    (literal "/dev/zero")
    (literal "/dev/tty")
    (literal "/dev/stdin")
    (literal "/dev/stdout")
    (literal "/dev/stderr")
    (subpath "/dev/fd"))
(allow file-read* (literal "/dev/random") (literal "/dev/urandom"))
(allow file-ioctl (subpath "/dev/fd"))
"#;

const NETWORK_SERVICES: &str = r#"
; DNS, network configuration and TLS trust, only with the proxy.
(allow mach-lookup
    (global-name "com.apple.SystemConfiguration.configd")
    (global-name "com.apple.SystemConfiguration.DNSConfiguration")
    (global-name "com.apple.networkd")
    (global-name "com.apple.SecurityServer")
    (global-name "com.apple.trustd")
    (global-name "com.apple.trustd.agent")
    (global-name "com.apple.ocspd"))
"#;

/// The profile for `sandbox`, with `toolchain` (existing, canonical toolchain directories)
/// readable too.
pub fn profile(sandbox: &SpawnSandbox, toolchain: &[PathBuf]) -> String {
    let mut profile = BASE.to_string();
    let readable = SYSTEM_READ_PATHS
        .iter()
        .map(Path::new)
        .chain(toolchain.iter().map(PathBuf::as_path))
        .chain(sandbox.readable.iter().map(PathBuf::as_path))
        .chain(sandbox.writable.iter().map(PathBuf::as_path));
    profile.push_str(&format!(
        "\n; Readable.\n(allow file-read*{})\n",
        subpaths(readable)
    ));
    let writable = subpaths(sandbox.writable.iter().map(PathBuf::as_path));
    if !writable.is_empty() {
        profile.push_str(&format!("; Writable.\n(allow file-write*{writable})\n"));
    }
    // A later rule wins: these stay read-only inside the writable trees.
    let protected = subpaths(sandbox.protected.iter().map(PathBuf::as_path));
    if !protected.is_empty() {
        profile.push_str(&format!("; Protected.\n(deny file-write*{protected})\n"));
    }
    match sandbox.network {
        SpawnNetwork::Blocked => {}
        SpawnNetwork::Open => {
            profile.push_str("
(allow network*)
");
            profile.push_str(NETWORK_SERVICES);
        }
        SpawnNetwork::Proxy { port } => {
            // Seatbelt takes only `*` or `localhost` as the host of a `remote tcp` rule.
            profile.push_str(&format!(
                "\n(allow network-outbound (remote tcp \"localhost:{port}\"))\n\
                 (allow network-bind (local ip \"localhost:*\"))\n"
            ));
            profile.push_str(NETWORK_SERVICES);
        }
    }
    profile
}

/// The toolchain directories under `home` ([`TOOLCHAIN_HOME_DIRS`]), plus, for each `PATH`
/// entry under `home`, its first component there (`~/.foo/bin` makes `~/.foo` readable), so a
/// toolchain not on the list still runs. `exists` and `canonical` are the file system's, passed
/// in so this is testable anywhere. `home` itself never becomes readable.
pub fn toolchain_paths(
    home: &Path,
    path_entries: &[PathBuf],
    canonical: impl Fn(&Path) -> Option<PathBuf>,
) -> Vec<PathBuf> {
    let Some(home) = canonical(home) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = TOOLCHAIN_HOME_DIRS
        .iter()
        .filter_map(|dir| canonical(&home.join(dir)))
        .collect();
    for entry in path_entries {
        let Some(entry) = canonical(entry) else {
            continue;
        };
        let Ok(relative) = entry.strip_prefix(&home) else {
            continue;
        };
        if let Some(first) = relative.components().next() {
            found.push(home.join(first));
        }
    }
    found.retain(|path| path != &home && path.starts_with(&home));
    found.sort();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(network: SpawnNetwork) -> SpawnSandbox {
        SpawnSandbox {
            readable: vec![PathBuf::from("/Applications/Tool.app/Contents/MacOS")],
            writable: vec![
                PathBuf::from("/Users/me/project"),
                PathBuf::from("/private/var/folders/xy/T/fyx-spawn-1"),
            ],
            protected: vec![PathBuf::from("/Users/me/project/.git/hooks")],
            network,
        }
    }

    #[test]
    fn open_network_allows_any_host() {
        let profile = profile(&sandbox(SpawnNetwork::Open), &[]);
        assert!(profile.contains("(allow network*)"));
        assert!(profile.contains("com.apple.trustd"));
        assert!(!profile.contains("localhost:"));
    }

    #[test]
    fn denies_by_default_and_lists_what_it_allows() {
        let profile = profile(
            &sandbox(SpawnNetwork::Blocked),
            &[PathBuf::from("/Users/me/.cargo")],
        );
        assert!(profile.starts_with("(version 1)"));
        assert!(profile.contains("(deny default)"));
        // No blanket rules.
        for blanket in [
            "(allow file-read*)",
            "(allow file-write*)",
            "(allow network*)",
            "(allow mach-lookup)",
        ] {
            assert!(!profile.contains(blanket), "{blanket}");
        }
        let reads = profile
            .split("; Readable.")
            .nth(1)
            .and_then(|rest| rest.split("; Writable.").next())
            .unwrap_or_default();
        for path in [
            "/usr",
            "/Users/me/.cargo",
            "/Applications/Tool.app/Contents/MacOS",
            "/Users/me/project",
        ] {
            assert!(reads.contains(&format!("(subpath \"{path}\")")), "{path}");
        }
        assert!(!reads.contains("(subpath \"/Users/me\")"));
        let writes = profile
            .split("; Writable.")
            .nth(1)
            .and_then(|rest| rest.split("; Protected.").next())
            .unwrap_or_default();
        assert!(writes.contains("(subpath \"/Users/me/project\")"));
        assert!(writes.contains("fyx-spawn-1"));
        assert!(!writes.contains("/usr"));
        assert!(profile.ends_with(
            "; Protected.\n(deny file-write*\n    (subpath \"/Users/me/project/.git/hooks\"))\n"
        ));
        assert!(!profile.contains("network-outbound"));
        assert!(!profile.contains("com.apple.trustd"));
        assert!(!profile.contains("com.apple.coreservices.launchservicesd"));
    }

    #[test]
    fn the_proxy_is_the_only_way_out() {
        let profile = profile(&sandbox(SpawnNetwork::Proxy { port: 49152 }), &[]);
        assert!(profile.contains("(allow network-outbound (remote tcp \"localhost:49152\"))"));
        assert_eq!(profile.matches("network-outbound").count(), 1);
        assert!(profile.contains("com.apple.trustd"));
    }

    #[test]
    fn paths_are_quoted_and_odd_ones_left_out() {
        let odd = SpawnSandbox {
            readable: vec![
                PathBuf::from("/tmp/a\"b\\c"),
                PathBuf::from("relative/dir"),
                PathBuf::from("/tmp/line\nbreak"),
            ],
            writable: Vec::new(),
            protected: Vec::new(),
            network: SpawnNetwork::Blocked,
        };
        let profile = profile(&odd, &[]);
        assert!(profile.contains(r#"(subpath "/tmp/a\"b\\c")"#));
        assert!(!profile.contains("relative/dir"));
        assert!(!profile.contains("line\nbreak"));
        // Nothing writable: no write rule beyond the device nodes.
        assert!(!profile.contains("; Writable."));
    }

    #[test]
    fn toolchains_under_home_but_never_home() {
        let home = PathBuf::from("/Users/me");
        let existing = [
            "/Users/me",
            "/Users/me/.cargo",
            "/Users/me/.cargo/bin",
            "/Users/me/.odd-tool/bin",
            "/Users/me/bin",
            "/usr/bin",
        ];
        let canonical = |path: &Path| {
            existing
                .iter()
                .any(|known| Path::new(known) == path)
                .then(|| path.to_path_buf())
        };
        let found = toolchain_paths(
            &home,
            &[
                PathBuf::from("/Users/me/.cargo/bin"),
                PathBuf::from("/Users/me/.odd-tool/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/Users/me"),
            ],
            canonical,
        );
        assert_eq!(
            found,
            [
                PathBuf::from("/Users/me/.cargo"),
                PathBuf::from("/Users/me/.odd-tool"),
            ]
        );
    }
}
