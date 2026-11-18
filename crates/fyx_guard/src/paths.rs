//! The file a path really names. Security and review decisions must not depend on how an agent
//! spelled a path: a committed symlink, `src/../.env`, `\\?\C:\repo\.env`, `.env.` or `.env::$DATA`
//! all name a file whose typed path matches no pattern. Shared by `fyx_guard` and `agent_review`, so
//! the shield and the review rules agree on what a path is.
//!
//! Match patterns against every path in [`ResolvedPath::candidates`]: a protection applies when any
//! candidate matches it, and a loosening (auto-accept) only when all of them do.

use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use fs::Fs;
use util::paths::{SanitizedPath, normalize_path};

/// A path as typed, and as the file system resolves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    /// The typed path without a `\\?\` prefix, with `.` and `..` resolved lexically.
    pub lexical: PathBuf,
    /// [`Self::lexical`] with Windows' silent renames undone: trailing dots and spaces dropped from
    /// each component, and an NTFS stream suffix (`::$DATA`, `:name`) dropped.
    pub alias_free: PathBuf,
    /// The real file: the deepest existing ancestor canonicalized (symlinks, 8.3 short names and the
    /// on-disk case resolved), the rest of the path appended. `None` when no ancestor exists.
    pub canonical: Option<PathBuf>,
}

impl ResolvedPath {
    /// Every distinct spelling of the file, the typed one first.
    pub fn candidates(&self) -> Vec<&Path> {
        let mut candidates = vec![self.lexical.as_path()];
        for path in [Some(&self.alias_free), self.canonical.as_ref()]
            .into_iter()
            .flatten()
        {
            if !candidates.contains(&path.as_path()) {
                candidates.push(path);
            }
        }
        candidates
    }

    /// Whether the typed path uses a spelling Windows silently rewrites.
    pub fn has_windows_alias(&self) -> bool {
        self.alias_free != self.lexical
    }
}

/// The typed path without a `\\?\` prefix, with `.` and `..` resolved lexically.
pub fn lexical(path: &Path) -> PathBuf {
    normalize_path(&without_verbatim_prefix(path))
}

/// `path` without a `\\?\` or `\\?\UNC\` prefix. Unlike [`SanitizedPath`], also when the path holds
/// `..`: what matters is which file a pattern must cover, not whether that spelling opens.
fn without_verbatim_prefix(path: &Path) -> PathBuf {
    if cfg!(windows)
        && let Some(text) = path.to_str()
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\")
            && rest.as_bytes().get(1) == Some(&b':')
        {
            return PathBuf::from(rest);
        }
    }
    SanitizedPath::new(path).as_path().to_path_buf()
}

/// Whether Windows opens a different name than `name`: it drops trailing dots and spaces, and a
/// `:` starts an NTFS stream name (`file::$DATA` is `file` itself).
pub fn is_windows_alias(name: &str) -> bool {
    name.contains(':') || (name.ends_with(['.', ' ']) && name != "." && name != "..")
}

/// `path` with every component Windows rewrites ([`is_windows_alias`]) rewritten the same way.
/// Applied on every OS: it only adds spellings to check, and a repository is shared across OSes.
pub fn alias_free(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => match name.to_str() {
                Some(text) if is_windows_alias(text) => {
                    let stem = text.split(':').next().unwrap_or_default();
                    let stem = stem.trim_end_matches(['.', ' ']);
                    if stem.is_empty() {
                        result.push(name);
                    } else {
                        result.push(OsString::from(stem));
                    }
                }
                _ => result.push(name),
            },
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Resolves `path` through `fs` (in tests, a fake file system with symlinks).
pub async fn resolve(fs: &dyn Fs, path: &Path) -> ResolvedPath {
    let (lexical, alias_free) = spellings(path);
    let typed = without_verbatim_prefix(path);
    let mut canonical = None;
    for ancestor in typed.ancestors() {
        if ancestor.as_os_str().is_empty() {
            break;
        }
        if let Ok(real) = fs.canonicalize(ancestor).await {
            canonical = Some(rejoin(&real, &typed, ancestor));
            break;
        }
    }
    ResolvedPath {
        lexical,
        alias_free,
        canonical,
    }
}

/// [`resolve`] on the real disk, blocking; for callers without an async context.
pub fn resolve_blocking(path: &Path) -> ResolvedPath {
    let (lexical, alias_free) = spellings(path);
    let typed = without_verbatim_prefix(path);
    let canonical = typed
        .ancestors()
        .take_while(|ancestor| !ancestor.as_os_str().is_empty())
        .find_map(|ancestor| {
            std::fs::canonicalize(ancestor)
                .ok()
                .map(|real| rejoin(&real, &typed, ancestor))
        });
    ResolvedPath {
        lexical,
        alias_free,
        canonical,
    }
}

/// The spellings of `path` without asking the file system ([`ResolvedPath::canonical`] is `None`).
pub fn unresolved(path: &Path) -> ResolvedPath {
    let (lexical, alias_free) = spellings(path);
    ResolvedPath {
        lexical,
        alias_free,
        canonical: None,
    }
}

fn spellings(path: &Path) -> (PathBuf, PathBuf) {
    let lexical = lexical(path);
    let alias_free = alias_free(&lexical);
    (lexical, alias_free)
}

/// `real` (the canonical `ancestor` of `typed`) with the rest of `typed` appended. Nothing past
/// `ancestor` exists, so it holds no symlinks and its `..` resolve lexically.
fn rejoin(real: &Path, typed: &Path, ancestor: &Path) -> PathBuf {
    let real = without_verbatim_prefix(real);
    let rest = typed.strip_prefix(ancestor).unwrap_or(Path::new(""));
    alias_free(&normalize_path(&real.join(rest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs::FakeFs;
    use gpui::TestAppContext;
    use serde_json::json;
    use util::path;

    #[test]
    fn windows_aliases_are_rewritten_as_windows_does() {
        assert!(is_windows_alias(".env."));
        assert!(is_windows_alias(".env "));
        assert!(is_windows_alias(".env::$DATA"));
        assert!(is_windows_alias("notes.txt:hidden"));
        assert!(!is_windows_alias(".env"));
        assert!(!is_windows_alias(".."));
        assert_eq!(
            alias_free(Path::new("repo/.claude./settings.json::$DATA")),
            PathBuf::from("repo/.claude/settings.json")
        );
        assert_eq!(
            alias_free(Path::new("repo/.env .. ")),
            PathBuf::from("repo/.env")
        );
        // Nothing left to keep: the name stays as it is.
        assert_eq!(alias_free(Path::new("repo/...")), PathBuf::from("repo/..."));
    }

    #[test]
    fn lexical_drops_dots_and_the_verbatim_prefix() {
        assert_eq!(
            lexical(Path::new("/repo/src/../secrets/./key.pem")),
            PathBuf::from("/repo/secrets/key.pem")
        );
        #[cfg(windows)]
        assert_eq!(
            lexical(Path::new(r"\\?\C:\repo\src\..\.env")),
            PathBuf::from(r"C:\repo\.env")
        );
    }

    #[gpui::test]
    async fn symlinks_resolve_to_the_real_file(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            path!("/outside"),
            json!({ "config": { "settings.json": "{}" } }),
        )
        .await;
        fs.insert_tree(path!("/repo"), json!({ "src": { "a.rs": "" } }))
            .await;
        fs.create_symlink(
            path!("/repo/link").as_ref(),
            path!("/outside/config").into(),
        )
        .await
        .unwrap();

        // An existing file behind a symlinked directory.
        let resolved = resolve(fs.as_ref(), path!("/repo/link/settings.json").as_ref()).await;
        assert_eq!(
            resolved.canonical.as_deref(),
            Some(Path::new(path!("/outside/config/settings.json")))
        );
        assert_eq!(
            resolved.candidates(),
            [
                Path::new(path!("/repo/link/settings.json")),
                Path::new(path!("/outside/config/settings.json"))
            ]
        );
        // A file that does not exist yet, behind the same link, with `..` in the new part.
        let resolved = resolve(
            fs.as_ref(),
            path!("/repo/link/new/../created.json").as_ref(),
        )
        .await;
        assert_eq!(
            resolved.canonical.as_deref(),
            Some(Path::new(path!("/outside/config/created.json")))
        );
        // A plain path resolves to itself.
        let resolved = resolve(fs.as_ref(), path!("/repo/src/a.rs").as_ref()).await;
        assert_eq!(resolved.candidates(), [Path::new(path!("/repo/src/a.rs"))]);
    }

    #[gpui::test]
    async fn aliases_of_a_new_file_keep_their_windows_meaning(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(path!("/repo"), json!({ "a.txt": "" })).await;
        let resolved = resolve(fs.as_ref(), path!("/repo/.env.").as_ref()).await;
        assert!(resolved.has_windows_alias());
        assert_eq!(resolved.alias_free, PathBuf::from(path!("/repo/.env")));
        assert_eq!(
            resolved.canonical.as_deref(),
            Some(Path::new(path!("/repo/.env")))
        );
    }

    #[test]
    fn resolves_on_the_real_disk() {
        let dir = std::env::temp_dir().join(format!("fyx-guard-paths-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let resolved = resolve_blocking(&dir.join("sub").join("..").join("new.txt"));
        let real_dir = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(resolved.canonical, Some(lexical(&real_dir).join("new.txt")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
