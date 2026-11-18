//! The pure part of `process:spawn` for fyx:ui 0.3 (docs/fyx/wit-0.3/design.md §9, plan 4.7,
//! spec decision 24): the manifest's program list, the limits, the request and outcome types,
//! the environment rules and the checks the import makes at once. No I/O, so the Apache harness
//! (`fyx_ext_harness`) makes the same checks as the host. Resolution on `PATH` outside the
//! worktrees, the filtered environment, job objects and process groups are the host's, in
//! `ui_ext_wasm::process`.

use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;

use crate::v0_2::{HostError, HostErrorCode};

pub const CAPABILITY_PROCESS: &str = "process:spawn";
/// Processes per extension, counted from the import until `process-exited` (or `spawn-failed`)
/// is delivered (decision 24).
pub const MAX_PROCESSES: usize = 4;
pub const MAX_ARGS: usize = 256;
pub const MAX_ARG_BYTES: usize = 128 * 1024;
pub const MAX_ENV_PAIRS: usize = 32;
pub const MAX_STDIN_WRITE_BYTES: usize = 1 << 20;
/// Bytes waiting to be written to one process's stdin.
pub const MAX_STDIN_QUEUED_BYTES: usize = 4 << 20;
/// The most bytes of one `process-output` event.
pub const MAX_OUTPUT_CHUNK_BYTES: usize = 64 * 1024;
/// A kill sends the polite signal first, then the hard one after this (Unix).
pub const KILL_GRACE: Duration = Duration::from_secs(2);

/// Environment names never passed to a program, even when the manifest declares them: they load
/// code into it, name another program for it to run, or choose where it finds programs and
/// configuration (design.md §9, "Environment"; integration-5 S6). Matched in ASCII upper case.
const FORBIDDEN_ENV_NAMES: &[&str] = &[
    // Interpreters and runtimes.
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHONSTARTUP",
    "PYTHONPATH",
    "PYTHONHOME",
    "PYTHONUSERBASE",
    "PERL5OPT",
    "PERL5LIB",
    "PERLLIB",
    "RUBYOPT",
    "RUBYLIB",
    "LUA_PATH",
    "LUA_CPATH",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "DOTNET_STARTUP_HOOKS",
    "PSMODULEPATH",
    // Shells.
    "BASH_ENV",
    "ENV",
    "PROMPT_COMMAND",
    // Programs other programs start.
    "EDITOR",
    "VISUAL",
    "PAGER",
    "GIT_EDITOR",
    "GIT_SEQUENCE_EDITOR",
    "GIT_PAGER",
    "GIT_EXTERNAL_DIFF",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_PROXY_COMMAND",
    "GIT_EXEC_PATH",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTDOC",
    // Where git finds its repository, hooks and configuration.
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_TEMPLATE_DIR",
    // Where programs are found: the host passes its own, filtered (design.md §9).
    "PATH",
    "PATHEXT",
    "COMSPEC",
];
const FORBIDDEN_ENV_PREFIXES: &[&str] = &[
    "LD_",
    "DYLD_",
    "GIT_CONFIG_",
    "NPM_CONFIG_",
    "YARN_",
    "CARGO_",
    "COR_",
    "CORECLR_",
];

/// One `[[fyx_ui.processes]]` entry.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    /// What `request.program` names; without `path`, resolved on `PATH` outside the worktrees.
    pub name: String,
    /// Argument patterns the program may be run with. One per position: a literal, `*` within it
    /// for any text, or `**` as the last pattern for any rest. None: no arguments at all (an
    /// open program needs `args = ["**"]`, which the consent prompt shows; integration-5 S6).
    #[serde(default)]
    pub args: Vec<String>,
    /// A worktree-relative program, which needs a trusted worktree.
    #[serde(default)]
    pub path: Option<String>,
}

/// The manifest's program list and passed-through environment names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessPolicy {
    pub programs: Vec<ProcessSpec>,
    pub env: BTreeSet<String>,
}

/// Whether the environment name `name` is one that loads code into a program.
pub fn is_forbidden_env(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    FORBIDDEN_ENV_NAMES.contains(&upper.as_str())
        || FORBIDDEN_ENV_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
}

/// A name for `[fyx_ui.process] env`: `[A-Za-z0-9_]+`, and not [`is_forbidden_env`]. The loader
/// refuses a manifest that declares a forbidden one.
pub fn check_env_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(format!("environment name {name:?} must be [A-Za-z0-9_]+"));
    }
    if is_forbidden_env(name) {
        return Err(format!(
            "environment name {name} loads code into the program and is never passed"
        ));
    }
    Ok(())
}

/// Whether `args` fit `patterns` (see [`ProcessSpec::args`]). No patterns: no arguments.
pub fn args_match(patterns: &[String], args: &[String]) -> bool {
    let rest = patterns.last().is_some_and(|last| last == "**");
    let fixed = if rest {
        &patterns[..patterns.len() - 1]
    } else {
        patterns
    };
    if args.len() < fixed.len() || (!rest && args.len() != fixed.len()) {
        return false;
    }
    fixed
        .iter()
        .zip(args)
        .all(|(pattern, arg)| glob(pattern.as_bytes(), arg.as_bytes()))
}

/// `*` matches any run of bytes; everything else matches itself.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|skip| glob(rest, &text[skip..])),
        Some((byte, rest)) => text
            .split_first()
            .is_some_and(|(first, text)| first == byte && glob(rest, text)),
    }
}

/// A file or directory in a worktree: the WIT `file-ref`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRef {
    /// `None`: the one worktree that has this path.
    pub worktree: Option<u64>,
    /// Relative, `/`-separated, without empty, `.` or `..` components.
    pub path: String,
}

impl FileRef {
    /// The path rule, checked at the import.
    pub fn check(&self) -> Result<(), HostError> {
        if self.path.is_empty() {
            return Ok(());
        }
        let bad = self.path.starts_with('/')
            || self.path.contains('\\')
            || self.path.contains(':')
            || self
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..");
        if bad {
            return Err(invalid(format!(
                "path {:?} must be relative, `/`-separated, without `.` or `..`",
                self.path
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnRequest {
    pub program: String,
    pub args: Vec<String>,
    /// `None`: the root of the first worktree.
    pub cwd: Option<FileRef>,
    pub env: Vec<(String, String)>,
    pub stdin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub process: u64,
    pub stream: OutputStream,
    /// At most [`MAX_OUTPUT_CHUNK_BYTES`].
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessExit {
    pub process: u64,
    /// `None` when it ended by a signal or was killed.
    pub code: Option<i32>,
    /// The host killed it.
    pub killed: bool,
}

/// The WIT `spawn-error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnError {
    Denied,
    /// Not declared, not on `PATH` outside the worktrees, or a script the host does not run.
    NotFound(String),
    LimitExceeded,
    Io(String),
}

fn invalid(message: impl Into<String>) -> HostError {
    HostError {
        code: HostErrorCode::InvalidArgument,
        message: message.into(),
    }
}

/// What the import decides at once.
#[derive(Debug, PartialEq, Eq)]
pub enum SpawnCheck<'a> {
    /// Accepted: start it after the call with this entry.
    Spawn(&'a ProcessSpec),
    /// Accepted as an id whose outcome is `spawn-failed(not-found)`: the program is not in the
    /// manifest (design.md §9 tests).
    NotDeclared,
}

/// The import's checks (design.md §9, "Semantics", "Limits"): the argument and environment caps,
/// the declared environment names, the argument patterns and the `cwd` path rule.
pub fn check_spawn<'a>(
    request: &SpawnRequest,
    policy: &'a ProcessPolicy,
) -> Result<SpawnCheck<'a>, HostError> {
    if request.args.len() > MAX_ARGS {
        return Err(invalid(format!("at most {MAX_ARGS} arguments")));
    }
    let arg_bytes: usize = request.args.iter().map(String::len).sum();
    if arg_bytes > MAX_ARG_BYTES {
        return Err(invalid(format!(
            "the arguments are over {MAX_ARG_BYTES} bytes"
        )));
    }
    if request.args.iter().any(|arg| arg.contains('\0')) {
        return Err(invalid("an argument contains NUL"));
    }
    if request.env.len() > MAX_ENV_PAIRS {
        return Err(invalid(format!(
            "at most {MAX_ENV_PAIRS} environment pairs"
        )));
    }
    for (name, value) in &request.env {
        if !policy.env.contains(name) || is_forbidden_env(name) {
            return Err(invalid(format!(
                "environment name {name} is not in [fyx_ui.process] env"
            )));
        }
        if value.contains('\0') {
            return Err(invalid(format!("the value of {name} contains NUL")));
        }
    }
    if let Some(cwd) = &request.cwd {
        cwd.check()?;
    }
    let Some(spec) = policy
        .programs
        .iter()
        .find(|spec| spec.name == request.program)
    else {
        return Ok(SpawnCheck::NotDeclared);
    };
    if !args_match(&spec.args, &request.args) {
        return Err(invalid(format!(
            "the arguments do not match the patterns declared for {}",
            spec.name
        )));
    }
    Ok(SpawnCheck::Spawn(spec))
}

/// The per-extension process cap ([`MAX_PROCESSES`]) and the ids.
#[derive(Debug, Default)]
pub struct ProcessCounters {
    next_id: u64,
    live: BTreeSet<u64>,
}

impl ProcessCounters {
    pub fn start(&mut self) -> Result<u64, HostError> {
        if self.live.len() >= MAX_PROCESSES {
            return Err(HostError {
                code: HostErrorCode::LimitExceeded,
                message: format!("at most {MAX_PROCESSES} processes per extension"),
            });
        }
        self.next_id += 1;
        self.live.insert(self.next_id);
        Ok(self.next_id)
    }

    pub fn is_live(&self, id: u64) -> bool {
        self.live.contains(&id)
    }

    /// Its `process-exited` or `spawn-failed` was delivered, or the call that made it failed.
    pub fn finished(&mut self, id: u64) {
        self.live.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn policy() -> ProcessPolicy {
        ProcessPolicy {
            programs: vec![
                ProcessSpec {
                    name: "git".into(),
                    args: strings(&["status", "--porcelain=*", "**"]),
                    path: None,
                },
                ProcessSpec {
                    name: "echo".into(),
                    args: strings(&["**"]),
                    path: None,
                },
            ],
            env: ["NO_COLOR".to_string()].into(),
        }
    }

    fn request(program: &str, args: &[&str]) -> SpawnRequest {
        SpawnRequest {
            program: program.into(),
            args: strings(args),
            cwd: None,
            env: Vec::new(),
            stdin: false,
        }
    }

    #[test]
    fn argument_patterns() {
        let patterns = strings(&["status", "--porcelain=*", "**"]);
        assert!(args_match(
            &patterns,
            &strings(&["status", "--porcelain=v2"])
        ));
        assert!(args_match(
            &patterns,
            &strings(&["status", "--porcelain=v2", "a", "b"])
        ));
        assert!(!args_match(&patterns, &strings(&["status"])));
        assert!(!args_match(
            &patterns,
            &strings(&["-c", "core.sshCommand=x", "status"])
        ));
        let exact = strings(&["--version"]);
        assert!(args_match(&exact, &strings(&["--version"])));
        assert!(!args_match(&exact, &strings(&["--version", "x"])));
        // No patterns: no arguments (integration-5 S6); `**` alone: any.
        assert!(!args_match(&[], &strings(&["-c", "alias.x=!cmd", "x"])));
        assert!(args_match(&[], &[]));
        assert!(args_match(
            &strings(&["**"]),
            &strings(&["anything", "at", "all"])
        ));
    }

    #[test]
    fn environment_names() {
        for forbidden in [
            "LD_PRELOAD",
            "ld_library_path",
            "DYLD_INSERT_LIBRARIES",
            "NODE_OPTIONS",
            "GIT_CONFIG_COUNT",
            "GIT_SSH_COMMAND",
            "BASH_ENV",
        ] {
            assert!(check_env_name(forbidden).is_err(), "{forbidden}");
        }
        for bad in ["", "A-B", "A B"] {
            assert!(check_env_name(bad).is_err(), "{bad:?}");
        }
        assert_eq!(check_env_name("NO_COLOR"), Ok(()));
    }

    #[test]
    fn the_import_checks() {
        let policy = policy();
        assert!(matches!(
            check_spawn(&request("git", &["status", "--porcelain=v2"]), &policy),
            Ok(SpawnCheck::Spawn(spec)) if spec.name == "git"
        ));
        assert_eq!(
            check_spawn(&request("rm", &["-rf"]), &policy),
            Ok(SpawnCheck::NotDeclared)
        );
        assert_eq!(
            check_spawn(&request("git", &["push"]), &policy)
                .unwrap_err()
                .code,
            HostErrorCode::InvalidArgument
        );
        let mut env = request("echo", &[]);
        env.env.push(("NO_COLOR".into(), "1".into()));
        assert!(check_spawn(&env, &policy).is_ok());
        env.env.push(("HOME".into(), "/tmp".into()));
        assert_eq!(
            check_spawn(&env, &policy).unwrap_err().code,
            HostErrorCode::InvalidArgument
        );
        let many = request("echo", &vec!["a"; MAX_ARGS + 1]);
        assert!(check_spawn(&many, &policy).is_err());
        let big = SpawnRequest {
            args: vec!["a".repeat(MAX_ARG_BYTES + 1)],
            ..request("echo", &[])
        };
        assert!(check_spawn(&big, &policy).is_err());
        for path in ["../up", "/abs", "a//b", "a\\b", "C:x", "./a"] {
            let bad = SpawnRequest {
                cwd: Some(FileRef {
                    worktree: None,
                    path: path.into(),
                }),
                ..request("echo", &[])
            };
            assert!(check_spawn(&bad, &policy).is_err(), "{path}");
        }
    }

    #[test]
    fn counters() {
        let mut counters = ProcessCounters::default();
        for id in 1..=MAX_PROCESSES as u64 {
            assert_eq!(counters.start(), Ok(id));
        }
        assert_eq!(
            counters.start().unwrap_err().code,
            HostErrorCode::LimitExceeded
        );
        counters.finished(2);
        assert_eq!(counters.start(), Ok(MAX_PROCESSES as u64 + 1));
        assert!(!counters.is_live(2));
    }
}
