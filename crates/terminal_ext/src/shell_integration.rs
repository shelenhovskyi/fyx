//! Shell integration with a nonce (plan 5.17) for zsh, bash, fish and PowerShell.
//!
//! Fyx's scripts (`assets/fyx/shell/`, built in) are written to a dir of the user's
//! ([`set_shell_integration`]) and loaded without touching the user's files: zsh through `ZDOTDIR`
//! (Fyx's dotfiles source the user's own), bash with `--init-file`, fish through an
//! `XDG_DATA_DIRS` entry (`fish/vendor_conf.d`), PowerShell with `-NoExit -Command`. Each script
//! marks prompts and commands with OSC 633 and signs the command line with the terminal's nonce
//! ([`NONCE_ENV`]), which it takes out of the environment before the user's first command.
//!
//! [`ShellIntegration::inject`] runs where `TerminalBuilder` resolves the shell, only for local
//! interactive terminals, and only when the shell's arguments are ones it understands (a login or
//! interactive flag); any other arguments are left alone and the terminal has no integration.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gpui::{App, Global};

/// The terminal's nonce, for the script; removed from the environment by the script.
pub const NONCE_ENV: &str = "FYX_SHELL_NONCE";
/// zsh: the user's `ZDOTDIR` (or `HOME`), which Fyx's dotfiles source from.
const USER_ZDOTDIR_ENV: &str = "FYX_USER_ZDOTDIR";
/// zsh: the user had a `ZDOTDIR`; otherwise the script unsets it after startup.
const ZDOTDIR_WAS_SET_ENV: &str = "FYX_ZDOTDIR_WAS_SET";
/// bash: `--init-file` replaces the login files, so the script sources them itself.
const BASH_LOGIN_ENV: &str = "FYX_BASH_LOGIN";
/// fish: the `XDG_DATA_DIRS` entry the script takes out again.
const FISH_DATA_DIR_ENV: &str = "FYX_FISH_DATA_DIR";
/// The XDG base-dir spec's default for an unset `XDG_DATA_DIRS`.
const DEFAULT_XDG_DATA_DIRS: &str = "/usr/local/share:/usr/share";

const ZSH_DIR: &str = "zsh";
const BASH_SCRIPT: &str = "fyx.bash";
const POWERSHELL_SCRIPT: &str = "fyx.ps1";

/// The scripts, by path under the integration dir.
const FILES: &[(&str, &str)] = &[
    (
        "zsh/.zshenv",
        include_str!("../../../assets/fyx/shell/zsh/.zshenv"),
    ),
    (
        "zsh/.zprofile",
        include_str!("../../../assets/fyx/shell/zsh/.zprofile"),
    ),
    (
        "zsh/.zshrc",
        include_str!("../../../assets/fyx/shell/zsh/.zshrc"),
    ),
    (
        "zsh/.zlogin",
        include_str!("../../../assets/fyx/shell/zsh/.zlogin"),
    ),
    (
        "zsh/fyx-integration.zsh",
        include_str!("../../../assets/fyx/shell/zsh/fyx-integration.zsh"),
    ),
    (
        BASH_SCRIPT,
        include_str!("../../../assets/fyx/shell/fyx.bash"),
    ),
    (
        "fish/vendor_conf.d/fyx.fish",
        include_str!("../../../assets/fyx/shell/fish/vendor_conf.d/fyx.fish"),
    ),
    (
        POWERSHELL_SCRIPT,
        include_str!("../../../assets/fyx/shell/fyx.ps1"),
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    PowerShell,
}

/// The shell a program path names, by its file name.
pub fn shell_kind(program: &str) -> Option<ShellKind> {
    let name = program.rsplit(['/', '\\']).next()?.to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    match name {
        "zsh" => Some(ShellKind::Zsh),
        "bash" => Some(ShellKind::Bash),
        "fish" => Some(ShellKind::Fish),
        "pwsh" | "powershell" => Some(ShellKind::PowerShell),
        _ => None,
    }
}

/// Where the scripts live; set by `terminal_host` while the setting is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellIntegration {
    dir: PathBuf,
}

struct GlobalShellIntegration(Option<ShellIntegration>);

impl Global for GlobalShellIntegration {}

/// `None` turns shell integration off for terminals opened from now on.
pub fn set_shell_integration(dir: Option<PathBuf>, cx: &mut App) {
    cx.set_global(GlobalShellIntegration(
        dir.map(|dir| ShellIntegration { dir }),
    ));
}

/// For `TerminalBuilder::new`, on the main thread.
pub fn shell_integration(cx: &App) -> Option<ShellIntegration> {
    cx.try_global::<GlobalShellIntegration>()?.0.clone()
}

/// Dirs whose scripts this process wrote, so each is written once.
static WRITTEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

fn env_value<S: BuildHasher>(env: &HashMap<String, String, S>, name: &str) -> Option<String> {
    env.get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .filter(|value| !value.is_empty())
}

fn flags_are(args: Option<&[String]>, allowed: &[&str]) -> bool {
    args.unwrap_or_default().iter().all(|arg| {
        allowed
            .iter()
            .any(|allowed| arg.eq_ignore_ascii_case(allowed))
    })
}

impl ShellIntegration {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes the scripts once per process (again if one went missing), off the main thread.
    fn ensure_files(&self) -> std::io::Result<()> {
        let mut written = WRITTEN.lock().unwrap_or_else(|poison| poison.into_inner());
        let complete = FILES.iter().all(|(path, _)| self.dir.join(path).is_file());
        if complete && written.contains(&self.dir) {
            return Ok(());
        }
        for (path, contents) in FILES {
            let path = self.dir.join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // A Windows checkout may have CRLF in the built-in copy; bash and zsh need LF.
            let contents = contents.replace("\r\n", "\n");
            if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
                std::fs::write(&path, contents)?;
            }
        }
        written.push(self.dir.clone());
        Ok(())
    }

    fn script(&self, path: &str) -> String {
        let path = self.dir.join(path).to_string_lossy().into_owned();
        // Git Bash on Windows reads `C:/...`; PowerShell takes either.
        if cfg!(windows) {
            path.replace('\\', "/")
        } else {
            path
        }
    }

    /// Loads Fyx's script into the shell `program` will start (`None`: the platform's default
    /// shell, as alacritty picks it) and gives it `nonce`. Returns whether it did; if not,
    /// `program` and `env` are as they were.
    pub fn inject<S: BuildHasher>(
        &self,
        program: &mut Option<(String, Vec<String>)>,
        env: &mut HashMap<String, String, S>,
        nonce: &str,
    ) -> bool {
        let (name, args) = match program.as_ref() {
            Some((name, args)) => (name.clone(), Some(args.clone())),
            None => match env_value(env, "SHELL") {
                Some(shell) => (shell, None),
                None => return false,
            },
        };
        let Some(kind) = shell_kind(&name) else {
            return false;
        };
        let login_flags = ["-l", "--login", "-i", "--interactive"];
        let understood = match kind {
            ShellKind::Zsh | ShellKind::Fish | ShellKind::Bash => {
                flags_are(args.as_deref(), &login_flags)
            }
            ShellKind::PowerShell => {
                flags_are(args.as_deref(), &["-NoLogo", "-nol", "-Login", "-l"])
            }
        };
        // Alacritty starts the default shell through `login` on macOS; only env reaches it there.
        let default_needs_args = program.is_none() && cfg!(target_os = "macos");
        if !understood || (default_needs_args && kind == ShellKind::Bash) {
            return false;
        }
        if let Err(error) = self.ensure_files() {
            log::warn!(
                "fyx shell integration: cannot write {}: {error}",
                self.dir.display()
            );
            return false;
        }
        let mut new_args = None;
        match kind {
            ShellKind::Zsh => {
                let user = env_value(env, "ZDOTDIR");
                if user.is_some() {
                    env.insert(ZDOTDIR_WAS_SET_ENV.to_owned(), "1".to_owned());
                }
                let user = user.or_else(|| env_value(env, "HOME")).unwrap_or_default();
                env.insert(USER_ZDOTDIR_ENV.to_owned(), user);
                env.insert("ZDOTDIR".to_owned(), self.script(ZSH_DIR));
            }
            ShellKind::Fish => {
                let dir = self.dir.to_string_lossy().into_owned();
                let rest = env_value(env, "XDG_DATA_DIRS")
                    .unwrap_or_else(|| DEFAULT_XDG_DATA_DIRS.to_owned());
                env.insert("XDG_DATA_DIRS".to_owned(), format!("{dir}:{rest}"));
                env.insert(FISH_DATA_DIR_ENV.to_owned(), dir);
            }
            ShellKind::Bash => {
                if args
                    .unwrap_or_default()
                    .iter()
                    .any(|arg| arg == "-l" || arg == "--login")
                {
                    env.insert(BASH_LOGIN_ENV.to_owned(), "1".to_owned());
                }
                new_args = Some(vec![
                    "--init-file".to_owned(),
                    self.script(BASH_SCRIPT),
                    "-i".to_owned(),
                ]);
            }
            ShellKind::PowerShell => {
                let script = self.script(POWERSHELL_SCRIPT).replace('\'', "''");
                let mut args = args.unwrap_or_default();
                args.extend([
                    "-NoExit".to_owned(),
                    "-Command".to_owned(),
                    format!(". '{script}'"),
                ]);
                new_args = Some(args);
            }
        }
        if let Some(args) = new_args {
            *program = Some((name, args));
        }
        env.insert(NONCE_ENV.to_owned(), nonce.to_owned());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integration(name: &str) -> (ShellIntegration, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fyx-shell-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        (ShellIntegration { dir: dir.clone() }, dir)
    }

    fn shell(program: &str, args: &[&str]) -> Option<(String, Vec<String>)> {
        Some((
            program.to_owned(),
            args.iter().map(|arg| (*arg).to_owned()).collect(),
        ))
    }

    #[test]
    fn shells_are_known_by_their_file_name() {
        assert_eq!(shell_kind("/bin/zsh"), Some(ShellKind::Zsh));
        assert_eq!(
            shell_kind(r"C:\Program Files\Git\bin\bash.exe"),
            Some(ShellKind::Bash)
        );
        assert_eq!(shell_kind("PWSH.EXE"), Some(ShellKind::PowerShell));
        assert_eq!(
            shell_kind(r"C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe"),
            Some(ShellKind::PowerShell)
        );
        assert_eq!(shell_kind("/usr/bin/fish"), Some(ShellKind::Fish));
        assert_eq!(shell_kind("cmd.exe"), None);
        assert_eq!(shell_kind("wsl.exe"), None);
        assert_eq!(shell_kind("/bin/bash-wrapper"), None);
    }

    #[test]
    fn each_shell_gets_its_script_and_the_nonce() {
        let (integration, dir) = integration("each");

        let mut env = HashMap::from([("HOME".to_owned(), "/home/u".to_owned())]);
        let mut program = shell("/bin/zsh", &["-l"]);
        assert!(integration.inject(&mut program, &mut env, "n1"));
        assert_eq!(program, shell("/bin/zsh", &["-l"]), "zsh: env only");
        assert_eq!(env[NONCE_ENV], "n1");
        assert_eq!(env[USER_ZDOTDIR_ENV], "/home/u");
        assert!(!env.contains_key(ZDOTDIR_WAS_SET_ENV) || std::env::var("ZDOTDIR").is_ok());
        assert!(Path::new(&env["ZDOTDIR"]).join(".zshrc").is_file());

        let mut env = HashMap::<String, String>::default();
        let mut program = shell("/usr/bin/bash", &["--login"]);
        assert!(integration.inject(&mut program, &mut env, "n2"));
        let (_, args) = program.unwrap();
        assert_eq!(args[0], "--init-file");
        assert!(Path::new(&args[1]).is_file());
        let script = std::fs::read_to_string(&args[1]).unwrap();
        assert!(!script.contains('\r'), "LF only");
        assert_eq!(env[BASH_LOGIN_ENV], "1");

        let mut env = HashMap::from([("XDG_DATA_DIRS".to_owned(), "/opt/share".to_owned())]);
        let mut program = shell("fish", &[]);
        assert!(integration.inject(&mut program, &mut env, "n3"));
        let dir_text = dir.to_string_lossy().into_owned();
        assert_eq!(env["XDG_DATA_DIRS"], format!("{dir_text}:/opt/share"));
        assert_eq!(env[FISH_DATA_DIR_ENV], dir_text);
        assert!(dir.join("fish/vendor_conf.d/fyx.fish").is_file());

        let mut env = HashMap::<String, String>::default();
        let mut program = shell(r"C:\Program Files\PowerShell\7\pwsh.exe", &["-NoLogo"]);
        assert!(integration.inject(&mut program, &mut env, "n4"));
        let (_, args) = program.unwrap();
        assert_eq!(&args[..3], ["-NoLogo", "-NoExit", "-Command"]);
        assert!(args[3].starts_with(". '") && args[3].ends_with("fyx.ps1'"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn other_arguments_and_other_shells_are_left_alone() {
        let (integration, dir) = integration("alone");
        for program in [
            shell("/bin/bash", &["-c", "make"]),
            shell("/bin/zsh", &["-f"]),
            shell("pwsh", &["-NoProfile", "-File", "x.ps1"]),
            shell("cmd.exe", &[]),
            shell("nu", &[]),
        ] {
            let mut env = HashMap::<String, String>::default();
            let mut changed = program.clone();
            assert!(!integration.inject(&mut changed, &mut env, "n"));
            assert_eq!(changed, program);
            assert!(env.is_empty());
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_powershell_path_with_a_quote_stays_one_argument() {
        let dir = std::env::temp_dir().join(format!("fyx-shell-o'neil-{}", std::process::id()));
        let integration = ShellIntegration { dir: dir.clone() };
        let mut program = shell("pwsh", &[]);
        assert!(integration.inject(&mut program, &mut HashMap::<String, String>::default(), "n"));
        let (_, args) = program.unwrap();
        assert!(args[2].contains("o''neil"), "{}", args[2]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
