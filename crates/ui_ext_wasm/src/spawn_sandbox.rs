//! Plan 7.5: on macOS a `process:spawn` child runs under Seatbelt (`/usr/bin/sandbox-exec -p`)
//! with the profile of `ui_ext::seatbelt`:
//!
//! - it reads the system, its toolchain, its own directory and its worktree, nothing else of the
//!   home directory or the other worktrees;
//! - it writes its worktree (but not [`PROTECTED_IN_WORKTREE`]) and a temporary directory of its
//!   own (`TMPDIR`), removed when it exits;
//! - it reaches the network only through a loopback proxy that allows the extension's declared
//!   `[fyx_ui.network] domains`, and only while `network:fetch` is granted; revoking it kills
//!   the program. Loopback domains and ports are not passed on: the proxy matches host names.
//!
//! Windows and Linux keep the job object and process group of plan 4.7 for now. What is pure
//! (the plan, the host list) builds and is tested on every OS; the rest is macOS only.

// What only macOS runs is built and tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::Path;

use ui_ext::network::DomainList;
use ui_ext::seatbelt::{SpawnNetwork, SpawnSandbox};

/// Worktree paths a sandboxed program may not write: they run later outside the sandbox (Git
/// hooks and config, editor tasks and settings, direnv), as for `workspace:edit`.
pub(crate) const PROTECTED_IN_WORKTREE: &[&str] = &[
    ".git/hooks",
    ".git/config",
    ".zed",
    ".fyx",
    ".vscode",
    ".idea",
    ".envrc",
];

/// What `program` (canonical) started in `worktree_root` (canonical) may touch, with `temp` as
/// its own temporary directory.
pub(crate) fn plan(
    program: &Path,
    worktree_root: &Path,
    temp: &Path,
    network: SpawnNetwork,
) -> SpawnSandbox {
    let readable = program
        .parent()
        .filter(|directory| !directory.starts_with(worktree_root))
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    SpawnSandbox {
        readable,
        writable: vec![worktree_root.to_path_buf(), temp.to_path_buf()],
        protected: PROTECTED_IN_WORKTREE
            .iter()
            .map(|relative| worktree_root.join(relative))
            .collect(),
        network,
    }
}

/// The host names the proxy allows: the declared domains without ports, loopback left out.
pub(crate) fn proxy_hosts(domains: &DomainList) -> Vec<String> {
    let mut hosts: Vec<String> = domains
        .texts()
        .into_iter()
        .filter_map(|text| {
            let host = match text.rsplit_once(':') {
                Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
                _ => text.as_str(),
            };
            let loopback = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1";
            (!loopback && http_proxy::HostPattern::parse(host).is_ok()).then(|| host.to_string())
        })
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// The project's egress list (W5's audit §3.3): `.fyx/egress.json` in the worktree root,
/// `{"hosts": ["api.github.com", "*.crates.io"]}`. `None` without the file: no project list. A
/// file that does not parse is an empty list, so nothing is allowed (fail closed). Sandboxed
/// programs cannot write `.fyx`.
pub(crate) fn project_egress(worktree_root: &Path) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Egress {
        hosts: Vec<String>,
    }
    let path = worktree_root.join(".fyx").join("egress.json");
    let text = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<Egress>(&text) {
        Ok(egress) => Some(egress.hosts),
        Err(error) => {
            log::warn!("{}: {error}; no host is allowed", path.display());
            Some(Vec::new())
        }
    }
}

/// `hosts` narrowed to the project's list: a declared host stays when a project entry covers it,
/// and a project entry stays when a declared wildcard covers it. No project list: `hosts`.
pub(crate) fn narrowed(hosts: Vec<String>, project: Option<Vec<String>>) -> Vec<String> {
    let Some(project) = project else {
        return hosts;
    };
    let parse = |text: &String| {
        http_proxy::HostPattern::parse(text)
            .ok()
            .map(|pattern| (text.clone(), pattern))
    };
    let declared: Vec<_> = hosts.iter().filter_map(parse).collect();
    let listed: Vec<_> = project.iter().filter_map(parse).collect();
    let mut kept = Vec::new();
    for (declared_text, declared_pattern) in &declared {
        for (listed_text, listed_pattern) in &listed {
            if listed_pattern.covers(declared_pattern) {
                kept.push(declared_text.clone());
            } else if declared_pattern.covers(listed_pattern) {
                kept.push(listed_text.clone());
            }
        }
    }
    kept.sort();
    kept.dedup();
    kept
}

#[cfg(target_os = "macos")]
pub(crate) use macos::sandboxed;

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use futures::StreamExt as _;
    use ui_ext::seatbelt::{self, SpawnNetwork};

    /// The command to spawn instead, and what must live until the program exits.
    pub(crate) struct Sandboxed {
        pub(crate) program: PathBuf,
        /// Before the program's own arguments.
        pub(crate) args: Vec<OsString>,
        /// Added to the program's environment.
        pub(crate) env: Vec<(OsString, OsString)>,
        pub(crate) networked: bool,
        temp: PathBuf,
        proxy: Option<http_proxy::ProxyHandle>,
    }

    impl Drop for Sandboxed {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.temp) {
                log::debug!("{}: {error}", self.temp.display());
            }
            // Its drop joins the listener thread; not on the caller's thread.
            if let Some(proxy) = self.proxy.take() {
                std::thread::spawn(move || drop(proxy));
            }
        }
    }

    fn variable<'a>(environment: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
        environment
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// Wraps `program` for `sandbox-exec`. `hosts` empty: no network.
    pub(crate) fn sandboxed(
        extension_id: &str,
        id: u64,
        program: &Path,
        worktree_root: &Path,
        hosts: Vec<String>,
        environment: &[(OsString, OsString)],
    ) -> Result<Sandboxed, String> {
        let canonical = |path: &Path| std::fs::canonicalize(path).ok();
        let program = canonical(program).ok_or("the program is gone")?;
        let worktree_root = canonical(worktree_root).ok_or("the worktree is gone")?;
        let temp = std::env::temp_dir().join(format!(
            "fyx-spawn-{}-{id}-{}",
            std::process::id(),
            extension_id.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
        ));
        std::fs::create_dir_all(&temp).map_err(|error| format!("{}: {error}", temp.display()))?;
        let temp = canonical(&temp).ok_or("the temporary directory is gone")?;
        let mut sandboxed = Sandboxed {
            program: PathBuf::from("/usr/bin/sandbox-exec"),
            args: Vec::new(),
            env: vec![("TMPDIR".into(), temp.clone().into_os_string())],
            networked: false,
            temp,
            proxy: None,
        };
        let network = if hosts.is_empty() {
            SpawnNetwork::Blocked
        } else {
            let (events, mut receiver) = futures::channel::mpsc::unbounded();
            let proxy = http_proxy::ProxyHandle::spawn(http_proxy::ProxyConfig {
                allowlist: http_proxy::Allowlist::from_patterns(
                    hosts
                        .iter()
                        .filter_map(|host| http_proxy::HostPattern::parse(host).ok()),
                ),
                upstream: None,
                events,
            })
            .map_err(|error| format!("the network proxy: {error:#}"))?;
            let port = proxy.port();
            let extension_id = extension_id.to_string();
            std::thread::spawn(move || {
                futures::executor::block_on(async move {
                    while let Some(event) = receiver.next().await {
                        if let http_proxy::ProxyEvent::RequestAttempt { host, outcome, .. } = event
                        {
                            log::info!("{extension_id}: program {id} to {host}: {outcome:?}");
                        }
                    }
                })
            });
            let address = OsString::from(format!("http://127.0.0.1:{port}"));
            for name in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                sandboxed.env.push((name.into(), address.clone()));
            }
            sandboxed.proxy = Some(proxy);
            sandboxed.networked = true;
            SpawnNetwork::Proxy { port }
        };
        let plan = super::plan(&program, &worktree_root, &sandboxed.temp, network);
        let path_entries: Vec<PathBuf> = variable(environment, "PATH")
            .map(|path| std::env::split_paths(path).collect())
            .unwrap_or_default();
        let toolchain = variable(environment, "HOME")
            .map(|home| seatbelt::toolchain_paths(Path::new(home), &path_entries, canonical))
            .unwrap_or_default();
        sandboxed.args = vec![
            "-p".into(),
            seatbelt::profile(&plan, &toolchain).into(),
            program.into_os_string(),
        ];
        Ok(sandboxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn plan_confines_to_the_worktree() {
        let root = PathBuf::from("/Users/me/project");
        let temp = PathBuf::from("/private/var/folders/t/fyx-spawn-1");
        let plan = plan(
            Path::new("/opt/homebrew/bin/rg"),
            &root,
            &temp,
            SpawnNetwork::Blocked,
        );
        assert_eq!(plan.readable, [PathBuf::from("/opt/homebrew/bin")]);
        assert_eq!(plan.writable, [root.clone(), temp.clone()]);
        assert!(plan.protected.contains(&root.join(".git/hooks")));
        assert!(plan.protected.contains(&root.join(".zed")));
        // A program in the worktree adds nothing readable.
        let inside = super::plan(&root.join("bin/tool"), &root, &temp, SpawnNetwork::Blocked);
        assert!(inside.readable.is_empty());
    }

    #[test]
    fn proxy_hosts_drop_ports_and_loopback() {
        let domains = DomainList::parse(&[
            "api.github.com".into(),
            "*.example.org".into(),
            "example.com:8443".into(),
            "localhost:3000".into(),
        ])
        .expect("valid domains");
        assert_eq!(
            proxy_hosts(&domains),
            ["*.example.org", "api.github.com", "example.com"]
        );
        assert!(proxy_hosts(&DomainList::default()).is_empty());
    }

    #[test]
    fn the_project_list_narrows_the_declared_hosts() {
        let strings = |list: &[&str]| list.iter().map(|host| host.to_string()).collect();
        let declared: Vec<String> = strings(&["*.github.com", "crates.io", "example.com"]);
        assert_eq!(narrowed(declared.clone(), None), declared);
        assert_eq!(
            narrowed(
                declared.clone(),
                Some(strings(&["api.github.com", "*.crates.io", "crates.io"]))
            ),
            ["api.github.com", "crates.io"]
        );
        assert!(narrowed(declared, Some(Vec::new())).is_empty());
    }

    #[test]
    fn the_project_list_is_read_from_the_worktree() {
        let root = tempfile::tempdir().expect("root");
        assert_eq!(project_egress(root.path()), None);
        std::fs::create_dir_all(root.path().join(".fyx")).expect(".fyx");
        let file = root.path().join(".fyx/egress.json");
        std::fs::write(&file, r#"{"hosts": ["api.github.com"]}"#).expect("write");
        assert_eq!(
            project_egress(root.path()),
            Some(vec!["api.github.com".to_string()])
        );
        std::fs::write(&file, "not json").expect("write");
        assert_eq!(project_egress(root.path()), Some(Vec::new()));
    }
}

/// The real sandbox (plan 7.5's done-when), macOS only: the owner runs these on the Mac.
#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::Output;

    fn environment() -> Vec<(OsString, OsString)> {
        std::env::vars_os()
            .filter(|(name, _)| name == "PATH" || name == "HOME")
            .collect()
    }

    /// Runs `program args` sandboxed in `worktree`, with `hosts` for the proxy.
    fn run(worktree: &Path, hosts: Vec<String>, program: &str, args: &[&str]) -> Output {
        let sandboxed = super::sandboxed(
            "test-extension",
            1,
            Path::new(program),
            worktree,
            hosts,
            &environment(),
        )
        .expect("sandboxed");
        std::process::Command::new(&sandboxed.program)
            .args(&sandboxed.args)
            .args(args)
            .current_dir(worktree)
            .envs(environment())
            .envs(sandboxed.env.iter().cloned())
            .output()
            .expect("sandbox-exec runs")
    }

    #[test]
    fn reads_and_writes_only_its_worktree() {
        let worktree = tempfile::tempdir().expect("worktree");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(worktree.path().join("inside.txt"), "inside").expect("write");
        std::fs::write(outside.path().join("secret.txt"), "secret").expect("write");
        std::fs::create_dir_all(worktree.path().join(".git/hooks")).expect("hooks");

        let inside = run(worktree.path(), Vec::new(), "/bin/cat", &["inside.txt"]);
        assert!(inside.status.success(), "{inside:?}");
        assert_eq!(inside.stdout, b"inside");

        let secret = outside.path().join("secret.txt");
        let denied = run(
            worktree.path(),
            Vec::new(),
            "/bin/cat",
            &[secret.to_str().expect("utf-8")],
        );
        assert!(!denied.status.success(), "{denied:?}");
        assert!(denied.stdout.is_empty());

        let written = run(worktree.path(), Vec::new(), "/usr/bin/touch", &["made.txt"]);
        assert!(written.status.success(), "{written:?}");
        assert!(worktree.path().join("made.txt").exists());

        let hook = run(
            worktree.path(),
            Vec::new(),
            "/usr/bin/touch",
            &[".git/hooks/pre-commit"],
        );
        assert!(!hook.status.success(), "{hook:?}");
        assert!(!worktree.path().join(".git/hooks/pre-commit").exists());

        let planted = outside.path().join("planted.txt");
        let elsewhere = run(
            worktree.path(),
            Vec::new(),
            "/usr/bin/touch",
            &[planted.to_str().expect("utf-8")],
        );
        assert!(!elsewhere.status.success(), "{elsewhere:?}");
        assert!(!planted.exists());
    }

    #[test]
    fn no_network_without_declared_hosts() {
        let worktree = tempfile::tempdir().expect("worktree");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let port = listener.local_addr().expect("address").port().to_string();
        // `nc -z` exits 0 when it connects.
        let blocked = run(
            worktree.path(),
            Vec::new(),
            "/usr/bin/nc",
            &["-z", "-w", "2", "127.0.0.1", &port],
        );
        assert!(!blocked.status.success(), "{blocked:?}");
    }

    #[test]
    fn the_proxy_refuses_an_undeclared_host() {
        let worktree = tempfile::tempdir().expect("worktree");
        // curl goes through HTTPS_PROXY; the proxy answers 403 for a host off the list.
        let refused = run(
            worktree.path(),
            vec!["example.com".into()],
            "/usr/bin/curl",
            &[
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--max-time",
                "5",
                "https://example.org/",
            ],
        );
        assert!(!refused.status.success(), "{refused:?}");
    }
}
