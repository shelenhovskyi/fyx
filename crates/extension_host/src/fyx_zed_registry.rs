//! fyx: Zed's extension registry (themes, languages, language servers) stays reachable while the
//! rest of Zed's cloud is off.
//!
//! Fyx points `server_url` nowhere, and upstream derives the registry address from it. The
//! `fyx.extensions.zed_registry_url` setting gives the registry its own base URL instead;
//! `assets/settings/fyx.json` sets it to `https://api.zed.dev`. Empty falls back to upstream's
//! address from `server_url`.

use anyhow::{Context as _, Result, ensure};
use url::Url;

/// The registry URL for `path` and `query` under `fyx.extensions.zed_registry_url`, or `None`
/// when the setting is empty or missing and the URL comes from `server_url`.
pub(crate) fn registry_url(
    base_url: Option<&str>,
    path: &str,
    query: &[(&str, &str)],
) -> Option<Result<Url>> {
    let base_url = base_url
        .map(str::trim)
        .filter(|base_url| !base_url.is_empty())?;
    Some(build_registry_url(base_url, path, query))
}

fn build_registry_url(base_url: &str, path: &str, query: &[(&str, &str)]) -> Result<Url> {
    let base_url = base_url.trim_end_matches('/');
    let mut url = Url::parse(&format!("{base_url}{path}"))
        .with_context(|| format!("fyx.extensions.zed_registry_url is not a URL: {base_url}"))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    ensure!(
        matches!(url.scheme(), "https" | "http"),
        "fyx.extensions.zed_registry_url must be an http(s):// URL: {base_url}"
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_missing_setting_falls_back_to_server_url() {
        assert!(registry_url(None, "/extensions", &[]).is_none());
        assert!(registry_url(Some(""), "/extensions", &[]).is_none());
        assert!(registry_url(Some("  "), "/extensions", &[]).is_none());
    }

    #[test]
    fn builds_path_and_query_under_the_base() -> Result<()> {
        let url = registry_url(
            Some("https://api.zed.dev/"),
            "/extensions/updates",
            &[("ids", "html,toml"), ("max_schema_version", "1")],
        )
        .context("the setting is set")??;
        assert_eq!(
            url.as_str(),
            "https://api.zed.dev/extensions/updates?ids=html%2Ctoml&max_schema_version=1"
        );

        let url = registry_url(
            Some("http://localhost:8080"),
            "/extensions/html/download",
            &[],
        )
        .context("the setting is set")??;
        assert_eq!(
            url.as_str(),
            "http://localhost:8080/extensions/html/download"
        );
        Ok(())
    }

    #[test]
    fn rejects_other_schemes_and_garbage() {
        let file = registry_url(Some("file:///registry"), "/extensions", &[]);
        assert!(matches!(file, Some(Err(_))));
        let garbage = registry_url(Some("not a url"), "/extensions", &[]);
        assert!(matches!(garbage, Some(Err(_))));
    }
}
