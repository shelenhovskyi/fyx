//! The update check and download, apart from the UI so tests can run them against a fake server
//! and a test key. Everything here goes through `fyx_update`'s checks.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use futures::AsyncReadExt as _;
use fyx_update::{Artifact, DetachedSignature, Offer, RELEASES_BASE, ReleaseKey};
use http_client::{AsyncBody, HttpClient};
use semver::Version;

/// The most bytes read for a manifest or its signature.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
/// The file in the updates folder that keeps the newest version a verified manifest showed.
const HIGHEST_SEEN_FILE: &str = "highest-seen";

/// What this build is, for the manifest checks.
#[derive(Clone, Debug)]
pub struct Build {
    /// `stable`, `preview`, `nightly` or `dev`.
    pub channel: String,
    pub version: Version,
    /// A Rust target triple, as the manifest names artifacts.
    pub target: String,
}

/// The result of one check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// This build has no release key, so no manifest could ever be trusted. Nothing was fetched.
    NotConfigured,
    Checked(Offer),
}

/// The target triple of this build, as the release workflow names artifacts.
pub fn current_target() -> String {
    let os = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "windows" => "pc-windows-msvc",
        "linux" => "unknown-linux-gnu",
        other => other,
    };
    format!("{}-{os}", std::env::consts::ARCH)
}

/// Fetches `fyx-<channel>.json` and its signature from `base`, verifies them with `keys` and
/// returns the offer for `build`. With no keys it returns [`CheckOutcome::NotConfigured`] before
/// any request. `updates_dir` keeps the newest version seen, so a replayed older manifest never
/// offers a downgrade.
pub async fn check(
    http: &dyn HttpClient,
    base: &str,
    keys: &[ReleaseKey],
    build: &Build,
    updates_dir: &Path,
    now: u64,
) -> Result<CheckOutcome> {
    if keys.is_empty() {
        return Ok(CheckOutcome::NotConfigured);
    }
    let manifest_url = format!("{base}/fyx-{}.json", build.channel);
    let manifest_bytes = fetch(http, &manifest_url, MAX_MANIFEST_BYTES).await?;
    let signature_bytes = fetch(http, &format!("{manifest_url}.sig"), MAX_MANIFEST_BYTES).await?;
    let signature = DetachedSignature::parse(
        std::str::from_utf8(&signature_bytes).context("the signature file is not UTF-8")?,
    )?;
    let manifest =
        fyx_update::verify_manifest(&manifest_bytes, &signature, keys, &build.channel, now)?;
    let highest_seen = read_highest_seen(updates_dir);
    let offer = fyx_update::offer(
        &manifest,
        &build.version,
        highest_seen.as_ref(),
        &build.target,
    );
    if highest_seen
        .as_ref()
        .is_none_or(|seen| manifest.version > *seen)
    {
        write_highest_seen(updates_dir, &manifest.version)?;
    }
    Ok(CheckOutcome::Checked(offer))
}

/// The release base URL: GitHub's latest release of `shelenhovskyi/fyx`.
pub fn releases_base() -> &'static str {
    RELEASES_BASE
}

/// Downloads `artifact` into `updates_dir`, checks its size and SHA-256 against the signed
/// manifest, and returns the file's path. A download that fails a check is not kept.
pub async fn download(
    http: &dyn HttpClient,
    artifact: &Artifact,
    updates_dir: &Path,
) -> Result<PathBuf> {
    let bytes = fetch(http, &artifact.url, artifact.size.saturating_add(1)).await?;
    fyx_update::check_artifact(&bytes, artifact)?;
    std::fs::create_dir_all(updates_dir)
        .with_context(|| format!("creating {}", updates_dir.display()))?;
    let path = updates_dir.join(file_name(&artifact.url));
    std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// The last segment of `url`, if it is a plain file name; a fixed name otherwise.
fn file_name(url: &str) -> String {
    let name = url
        .split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or_default();
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-+".contains(character));
    if plain {
        name.to_owned()
    } else {
        "fyx-update.download".to_owned()
    }
}

async fn fetch(http: &dyn HttpClient, url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut response = http
        .get(url, AsyncBody::default(), true)
        .await
        .with_context(|| format!("fetching {url}"))?;
    if !response.status().is_success() {
        bail!("{url} answered {}", response.status());
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .take(limit)
        .read_to_end(&mut bytes)
        .await
        .with_context(|| format!("reading {url}"))?;
    Ok(bytes)
}

fn read_highest_seen(updates_dir: &Path) -> Option<Version> {
    let text = std::fs::read_to_string(updates_dir.join(HIGHEST_SEEN_FILE)).ok()?;
    Version::parse(text.trim()).ok()
}

fn write_highest_seen(updates_dir: &Path, version: &Version) -> Result<()> {
    std::fs::create_dir_all(updates_dir)
        .with_context(|| format!("creating {}", updates_dir.display()))?;
    std::fs::write(updates_dir.join(HIGHEST_SEEN_FILE), version.to_string())
        .context("saving the newest version seen")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use fyx_update::{ArtifactKind, MANIFEST_SCHEMA, Manifest};
    use http_client::{FakeHttpClient, Response};
    use ring::signature::{Ed25519KeyPair, KeyPair as _};
    use sha2::{Digest as _, Sha256};

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const BASE: &str = "https://releases.test/download";
    const APP: &[u8] = b"the 1.0.1 app";

    /// The same throwaway key as `fyx_update`'s tests: seed `[7; 32]`, never a release key.
    fn key_pair() -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("a test key")
    }

    fn test_keys() -> Vec<ReleaseKey> {
        let mut ed25519 = [0; 32];
        ed25519.copy_from_slice(key_pair().public_key().as_ref());
        vec![ReleaseKey {
            key_id: "fyx-release-test",
            ed25519,
        }]
    }

    fn build(version: &str) -> Build {
        Build {
            channel: "stable".into(),
            version: Version::parse(version).expect("a version"),
            target: "x86_64-pc-windows-msvc".into(),
        }
    }

    fn artifact() -> Artifact {
        Artifact {
            target: "x86_64-pc-windows-msvc".into(),
            kind: ArtifactKind::WindowsZip,
            url: format!("{BASE}/Fyx-x86_64-windows.zip"),
            size: APP.len() as u64,
            sha256: hex::encode(Sha256::digest(APP)),
        }
    }

    /// A fake release server: the files by URL, and every URL asked for.
    fn server(version: &str) -> (Arc<http_client::HttpClientWithUrl>, Arc<Mutex<Vec<String>>>) {
        let manifest = Manifest {
            schema: MANIFEST_SCHEMA.into(),
            channel: "stable".into(),
            version: Version::parse(version).expect("a version"),
            published_at: NOW - 60,
            notes_url: "https://github.com/shelenhovskyi/fyx/releases/tag/v1.0.1".into(),
            artifacts: vec![artifact()],
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("json");
        let signature = format!(
            "fyx-release-test {}\n",
            base64::engine::general_purpose::STANDARD.encode(key_pair().sign(&manifest_bytes))
        );
        let files: HashMap<String, Vec<u8>> = [
            (format!("{BASE}/fyx-stable.json"), manifest_bytes),
            (
                format!("{BASE}/fyx-stable.json.sig"),
                signature.into_bytes(),
            ),
            (format!("{BASE}/Fyx-x86_64-windows.zip"), APP.to_vec()),
        ]
        .into();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let http = FakeHttpClient::create(move |request| {
            let url = request.uri().to_string();
            seen.lock().expect("lock").push(url.clone());
            let body = files.get(&url).cloned();
            async move {
                Ok(match body {
                    Some(bytes) => Response::builder().status(200).body(bytes.into())?,
                    None => Response::builder().status(404).body(AsyncBody::default())?,
                })
            }
        });
        (http, requests)
    }

    #[gpui::test]
    async fn without_a_key_nothing_is_fetched() {
        let (http, requests) = server("1.0.1");
        let dir = tempfile::tempdir().expect("a temp dir");
        let outcome = check(http.as_ref(), BASE, &[], &build("1.0.0"), dir.path(), NOW)
            .await
            .expect("a check");
        assert_eq!(outcome, CheckOutcome::NotConfigured);
        assert!(requests.lock().expect("lock").is_empty());
        assert!(fyx_update::RELEASE_KEYS.is_empty(), "no release key yet");
    }

    #[gpui::test]
    async fn a_signed_newer_release_is_offered_and_downloads_verified() {
        let (http, _) = server("1.0.1");
        let dir = tempfile::tempdir().expect("a temp dir");
        let outcome = check(
            http.as_ref(),
            BASE,
            &test_keys(),
            &build("1.0.0"),
            dir.path(),
            NOW,
        )
        .await
        .expect("a check");
        let CheckOutcome::Checked(Offer::Update {
            version, artifact, ..
        }) = outcome
        else {
            panic!("expected an update, got {outcome:?}");
        };
        assert_eq!(version, Version::new(1, 0, 1));
        let path = download(http.as_ref(), &artifact, dir.path())
            .await
            .expect("a verified download");
        assert_eq!(std::fs::read(&path).expect("the file"), APP);
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("Fyx-x86_64-windows.zip")
        );

        let tampered = Artifact {
            sha256: hex::encode(Sha256::digest(b"something else")),
            ..artifact
        };
        std::fs::remove_file(&path).expect("remove");
        assert!(
            download(http.as_ref(), &tampered, dir.path())
                .await
                .is_err()
        );
        assert!(!path.exists(), "a download that fails its hash is not kept");
    }

    #[gpui::test]
    async fn an_older_manifest_after_a_newer_one_is_no_downgrade() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (newer, _) = server("1.0.2");
        check(
            newer.as_ref(),
            BASE,
            &test_keys(),
            &build("1.0.0"),
            dir.path(),
            NOW,
        )
        .await
        .expect("a check");
        let (replayed, _) = server("1.0.1");
        let outcome = check(
            replayed.as_ref(),
            BASE,
            &test_keys(),
            &build("1.0.0"),
            dir.path(),
            NOW,
        )
        .await
        .expect("a check");
        assert_eq!(outcome, CheckOutcome::Checked(Offer::UpToDate));
    }

    #[gpui::test]
    async fn a_manifest_signed_by_another_key_is_refused() {
        let (http, _) = server("1.0.1");
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut other = test_keys();
        other[0].ed25519 = [1; 32];
        assert!(
            check(
                http.as_ref(),
                BASE,
                &other,
                &build("1.0.0"),
                dir.path(),
                NOW
            )
            .await
            .is_err()
        );
    }

    #[test]
    fn download_names_are_plain_file_names() {
        assert_eq!(file_name("https://x.test/a/Fyx-1.zip?x=1"), "Fyx-1.zip");
        assert_eq!(file_name("https://x.test/a/..%2f.."), "fyx-update.download");
        assert_eq!(file_name("https://x.test/a/"), "fyx-update.download");
        assert_eq!(file_name("https://x.test/.hidden"), "fyx-update.download");
    }
}
