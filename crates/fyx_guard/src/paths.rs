//! [`fyx_guard_core::paths`] and the resolver over a project's file system ([`Fs`]): the file a
//! path really names.

use std::path::Path;

use fs::Fs;

pub use fyx_guard_core::paths::*;

/// Resolves `path` through `fs` (in tests, a fake file system with symlinks).
pub async fn resolve(fs: &dyn Fs, path: &Path) -> ResolvedPath {
    for ancestor in ancestors(path) {
        if let Ok(real) = fs.canonicalize(&ancestor).await {
            return resolved(path, Some((&ancestor, &real)));
        }
    }
    resolved(path, None)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use fs::FakeFs;
    use gpui::TestAppContext;
    use serde_json::json;
    use util::path;

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
}
