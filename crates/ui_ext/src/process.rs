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

/// The only environment names `[fyx_ui.process] env` may declare (integration-6 N1): locale,
/// terminal and output, logging, and a few runtime switches that load nothing. Everything else is
/// refused, because the names that load code or move a program's configuration (`HOME`,
/// `XDG_CONFIG_HOME`, `ZDOTDIR`, `GOFLAGS`, `CLASSPATH`, `GCONV_PATH`, ...) have no end, and
/// Fyx's own `FYX_*` and `ZED_*` names change what Fyx loads. Matched in ASCII upper case; `LC_*`
/// is allowed too. Ask for an addition rather than widening it per extension.
pub const ALLOWED_ENV_NAMES: &[&str] = &[
    // Locale and time.
    "LANG",
    "LANGUAGE",
    "TZ",
    // Terminal and output.
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "COLUMNS",
    "LINES",
    "CI",
    // Logging and runtime switches.
    "RUST_LOG",
    "RUST_BACKTRACE",
    "NODE_ENV",
    "PYTHONUNBUFFERED",
    "PYTHONIOENCODING",
    "PYTHONUTF8",
];
const ALLOWED_ENV_PREFIXES: &[&str] = &["LC_"];

/// Environment names never passed to a program, even when the manifest declares them: they load
/// code into it, name another program for it to run, or choose where it finds programs and
/// configuration (design.md §9, "Environment"; integration-5 S6). Matched in ASCII upper case.
/// [`ALLOWED_ENV_NAMES`] already leaves them out; this list gives them a clearer error.
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
    /// 0.4 (design wit-0.4 §C.2): the author's one line under the item at install, required
    /// there and refused below 0.4.
    #[serde(default)]
    pub reason: Option<String>,
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

/// Whether the environment name `name` may be declared and passed: in [`ALLOWED_ENV_NAMES`] (or
/// `LC_*`) and not [`is_forbidden_env`].
pub fn is_allowed_env(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    (ALLOWED_ENV_NAMES.contains(&upper.as_str())
        || ALLOWED_ENV_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix)))
        && !is_forbidden_env(name)
}

/// A name for `[fyx_ui.process] env`: `[A-Za-z0-9_]+` and [`is_allowed_env`]. The loader refuses
/// a manifest that declares any other.
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
    if !is_allowed_env(name) {
        return Err(format!(
            "environment name {name} is not one Fyx passes to programs: only {} and LC_*",
            ALLOWED_ENV_NAMES.join(", ")
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

/// Programs that run whatever code they are given: shells, interpreters and launchers (owner's
/// Q2, 2026-11-19). The manifest may name them, with `args` patterns like any program, but a grant
/// of one means "any program", and the consent prompt says so ([`RUNS_ANY_CODE_PROMPT`]). Names
/// are compared case-insensitively, without a Windows executable extension and without a version
/// suffix (`python3.12`, `node22`, `lua-5.4`; integration-6 Q2).
pub const RUNS_ANY_CODE: &[&str] = &[
    // Shells.
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "mksh",
    "csh",
    "tcsh",
    "nu",
    "elvish",
    "xonsh",
    "pwsh",
    "powershell",
    "cmd",
    "osascript",
    "wscript",
    "cscript",
    "mshta",
    // Interpreters and runtimes.
    "node",
    "nodejs",
    "deno",
    "bun",
    "npx",
    "bunx",
    "python",
    "python2",
    "python3",
    "py",
    "pypy",
    "pypy3",
    "perl",
    "ruby",
    "irb",
    "php",
    "lua",
    "luajit",
    "tclsh",
    "wish",
    "rscript",
    "julia",
    "java",
    "jshell",
    "dotnet",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    // Programs that run another program.
    "env",
    "xargs",
    "sudo",
    "doas",
    "runas",
    "nohup",
    "nice",
    "time",
    "timeout",
    "watch",
    "busybox",
    "start",
    "wsl",
    "ssh",
    "rundll32",
    "regsvr32",
    "msiexec",
    "schtasks",
    "docker",
    "podman",
    // Package managers and build or task runners: they run the scripts a project names.
    "npm",
    "pnpm",
    "yarn",
    "cargo",
    "rustup",
    "go",
    "make",
    "gmake",
    "nmake",
    "cmake",
    "ninja",
    "just",
    "rake",
    "bundle",
    "gem",
    "composer",
    "gradle",
    "gradlew",
    "mvn",
    "mvnw",
    "ant",
    "uv",
    "uvx",
    "pip",
    "pipx",
    "poetry",
    "conda",
    "mamba",
];

/// Git subcommands that run a command they are given or that configuration names (integration-6
/// Q2). Any arguments after them count, so `["config", "**"]` is "any program".
const GIT_RUNS_ANY_CODE: &[&str] = &[
    "config",
    "bisect",
    "submodule",
    "rebase",
    "filter-branch",
    "difftool",
    "mergetool",
    "daemon",
];

/// What the consent prompt says about a program in [`RUNS_ANY_CODE`] (owner's Q2).
pub const RUNS_ANY_CODE_PROMPT: &str = "can run any program on your computer";

/// Whether granting `spec` means "any program" (owner's Q2): a program in [`RUNS_ANY_CODE`], or
/// `git` with options before the subcommand that can set its configuration or repository (`-c`,
/// `--config-env`, `--exec-path`, `--git-dir`, or a wildcard), since a config value can name a
/// command git runs, or with a subcommand in `GIT_RUNS_ANY_CODE` (or a wildcard for it).
pub fn runs_any_code(spec: &ProcessSpec) -> bool {
    let base = spec
        .path
        .as_deref()
        .unwrap_or(&spec.name)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let program = [".exe", ".cmd", ".bat", ".com", ".ps1"]
        .iter()
        .find_map(|extension| base.strip_suffix(extension))
        .unwrap_or(&base);
    let unversioned = without_version(program);
    if RUNS_ANY_CODE.contains(&program) || RUNS_ANY_CODE.contains(&unversioned) {
        return true;
    }
    if program != "git" {
        return false;
    }
    // Git's own options come before the subcommand: one of them that sets configuration or the
    // repository, or a wildcard there (which could be one), makes a config value name a command
    // git runs. `-C` and `--work-tree` take a value; repository discovery inside it finds only a
    // `.git`, which `workspace:edit` never writes.
    let mut patterns = spec.args.iter();
    while let Some(pattern) = patterns.next() {
        if pattern.contains('*') {
            return true;
        }
        if !pattern.starts_with('-') {
            return GIT_RUNS_ANY_CODE.contains(&pattern.as_str());
        }
        if pattern.starts_with("-c")
            || pattern.starts_with("--config-env")
            || pattern.starts_with("--exec-path")
            || pattern.starts_with("--git-dir")
        {
            return true;
        }
        if pattern == "-C" || pattern == "--work-tree" || pattern == "--namespace" {
            patterns.next();
        }
    }
    false
}

/// `program` without a trailing version: `python3.12` and `python-3.12` are `python`, `node22`
/// is `node`. A name that is only digits stays as it is.
fn without_version(program: &str) -> &str {
    let trimmed = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    let trimmed = trimmed.trim_end_matches(['-', '_']);
    if trimmed.is_empty() || trimmed.len() == program.len() {
        program
    } else {
        trimmed
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
    Other(String),
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
        if !policy.env.contains(name) || !is_allowed_env(name) {
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

    /// Owner's Q2: interpreters and shells may be named, and count as "any program".
    #[test]
    fn interpreters_and_shells_run_any_code() {
        let spec = |name: &str, args: &[&str]| ProcessSpec {
            name: name.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            path: None,
            reason: None,
        };
        for name in [
            "bash", "Python3", "node.exe", "PWSH.EXE", "cmd", "env", "npx.cmd",
        ] {
            assert!(runs_any_code(&spec(name, &["**"])), "{name}");
            assert!(runs_any_code(&spec(name, &[])), "{name} without arguments");
        }
        let mut local = spec("lint", &[]);
        local.path = Some("tools/python.exe".into());
        assert!(runs_any_code(&local));
        for name in ["rustfmt", "eslint", "prettier", "hostname", "pythonic"] {
            assert!(!runs_any_code(&spec(name, &["**"])), "{name}");
        }
        assert!(!runs_any_code(&spec("git", &["status", "--porcelain"])));
        assert!(!runs_any_code(&spec("git", &["log", "**"])));
        assert!(runs_any_code(&spec("git", &["**"])));
        assert!(runs_any_code(&spec("git", &["-c", "*", "**"])));
        assert!(runs_any_code(&spec("git", &["*"])));
        // integration-6 Q2: versioned names, more runners, and git subcommands that run commands.
        for name in [
            "python3.12",
            "python3.12.exe",
            "node22",
            "lua-5.4",
            "ruby3.2",
            "wsl.exe",
            "npm.cmd",
            "pnpm",
            "yarn",
            "cargo",
            "make",
            "uv",
            "uvx",
            "ssh",
            "rundll32",
        ] {
            assert!(runs_any_code(&spec(name, &["**"])), "{name}");
        }
        for name in ["rustfmt2", "tsc", "clang-format-18", "7z"] {
            assert!(!runs_any_code(&spec(name, &["**"])), "{name}");
        }
        for args in [
            &["config", "**"][..],
            &["bisect", "run", "**"],
            &["submodule", "foreach", "**"],
            &["-C", "sub", "rebase", "**"],
            &["--git-dir=tmp/x", "status"],
            &["--no-pager", "*"],
        ] {
            assert!(runs_any_code(&spec("git", args)), "{args:?}");
        }
        assert!(!runs_any_code(&spec("git", &["-C", "sub", "status", "**"])));
        assert!(!runs_any_code(&spec("git", &["--no-pager", "diff", "**"])));
        assert_eq!(RUNS_ANY_CODE_PROMPT, "can run any program on your computer");
    }

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
                    reason: None,
                },
                ProcessSpec {
                    name: "echo".into(),
                    args: strings(&["**"]),
                    path: None,
                    reason: None,
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
            // integration-6 N1: homes, config dirs and other loaders are not allowed.
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
            "APPDATA",
            "ZDOTDIR",
            "GOFLAGS",
            "CLASSPATH",
            "GCONV_PATH",
            "FYX_UI_DEV_EXTENSIONS",
            "ZED_STATELESS",
            "MY_TOOL_SETTING",
        ] {
            assert!(check_env_name(forbidden).is_err(), "{forbidden}");
        }
        for allowed in ["NO_COLOR", "lc_all", "LC_MESSAGES", "RUST_LOG", "TZ"] {
            assert_eq!(check_env_name(allowed), Ok(()), "{allowed}");
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
