//! Account profiles in the extension runtime (`fyx.accounts`, design wit-0.4 §C.6). The host sets
//! its [`AccountDirs`] once ([`set_account_dirs`]): account profiles are user settings, one set
//! per Fyx process.
//!
//! - A terminal an extension creates gets `${account.<tool>}` from the account chosen for its
//!   project's tool (resolved here, host-side; the extension sees only the profile's name).
//! - The user's own tools' config-dir variables are refused in its env, like the presets'
//!   ([`ui_ext::terminal::ACCOUNT_DIR_ENV_NAMES`]).
//! - Account dirs are protected: no process or terminal starts in one, and no proposal,
//!   diagnostic, context file or absolute path names a file in one.
//!
//! Nothing here reads, lists or checks a dir's contents.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
};

use ui_ext::terminal::{AccountDirs, in_account_dirs};

static ACCOUNT_DIRS: RwLock<Option<Arc<dyn AccountDirs>>> = RwLock::new(None);

/// The host's account profiles, for every extension from now on; `None` removes them.
pub fn set_account_dirs(dirs: Option<Arc<dyn AccountDirs>>) {
    *ACCOUNT_DIRS.write().unwrap_or_else(PoisonError::into_inner) = dirs;
}

fn dirs() -> Option<Arc<dyn AccountDirs>> {
    ACCOUNT_DIRS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Whether `path`, absolute, is in an account profile's dir.
pub(crate) fn in_account_dir(path: &Path) -> bool {
    dirs().is_some_and(|dirs| in_account_dirs(path, &dirs.dirs()))
}

/// Whether `name` is the config-dir variable of one of the user's `fyx.accounts.tools`.
pub(crate) fn is_user_tool_env(name: &str) -> bool {
    dirs().is_some_and(|dirs| {
        dirs.env_names()
            .iter()
            .any(|env| env.trim().eq_ignore_ascii_case(name))
    })
}

/// `${account.<tool>}` for a terminal starting in `cwd`. The runtime resolves `cwd` canonically;
/// the host gets it as users write paths, without Windows' `\\?\` prefix.
pub(crate) fn account_dir(tool: &str, cwd: &Path) -> Option<PathBuf> {
    dirs()?.account_dir(tool, &without_verbatim_prefix(cwd))
}

fn without_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Profiles over a fixed list: `(tool, dir, project folder)`, and the tools' variables.
    pub(crate) struct FakeAccounts {
        pub(crate) profiles: Vec<(&'static str, PathBuf, PathBuf)>,
        pub(crate) env_names: Vec<&'static str>,
    }

    impl AccountDirs for FakeAccounts {
        fn dirs(&self) -> Vec<PathBuf> {
            self.profiles
                .iter()
                .map(|(_, dir, _)| dir.clone())
                .collect()
        }
        fn env_names(&self) -> Vec<String> {
            self.env_names.iter().map(|name| name.to_string()).collect()
        }
        fn account_dir(&self, tool: &str, cwd: &Path) -> Option<PathBuf> {
            self.profiles
                .iter()
                .find(|(profile_tool, _, project)| {
                    *profile_tool == tool && cwd.starts_with(project)
                })
                .map(|(_, dir, _)| dir.clone())
        }
    }

    #[test]
    fn dirs_names_and_choices() {
        let temp = std::env::temp_dir().join(format!("fyx-accounts-{}", std::process::id()));
        let work = temp.join(".claude-work");
        let project = temp.join("api");
        std::fs::create_dir_all(work.join("projects")).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        assert!(!in_account_dir(&work));
        set_account_dirs(Some(Arc::new(FakeAccounts {
            profiles: vec![("claude", work.clone(), project.clone())],
            env_names: vec!["AIDER_HOME"],
        })));
        assert!(in_account_dir(&work));
        assert!(in_account_dir(&work.join("projects").join("x.jsonl")));
        assert!(!in_account_dir(&temp.join(".claude-work2")));
        assert!(!in_account_dir(&project));
        // `..` that ends inside, and (on Windows) another letter case.
        assert!(in_account_dir(
            &project.join("..").join(".claude-work").join("projects")
        ));
        if cfg!(windows) {
            let upper = PathBuf::from(work.to_string_lossy().to_uppercase());
            assert!(in_account_dir(&upper));
        }
        assert!(is_user_tool_env("aider_home"));
        assert!(!is_user_tool_env("HOME"));
        assert_eq!(account_dir("claude", &project), Some(work.clone()));
        assert_eq!(account_dir("codex", &project), None);
        assert_eq!(account_dir("claude", &temp), None);
        set_account_dirs(None);
        assert!(!in_account_dir(&work));
        std::fs::remove_dir_all(&temp).ok();
    }
}
