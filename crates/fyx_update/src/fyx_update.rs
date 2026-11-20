//! Fyx self-updater core (plan 6.2, design `docs/fyx/platform/self-update.md`). Pure: no network,
//! no file system, no UI. The host fetches the two files, asks [`offer`], downloads the artifact,
//! checks it with [`check_artifact`] and runs the steps of [`install_plan`] on restart.
//!
//! Trust comes from Fyx's own Ed25519 key only: the manifest's exact bytes are signed (no
//! canonical JSON), and the manifest pins each artifact's SHA-256. No Apple or Windows signature
//! is checked yet (no Apple Developer account); that check is added beside this one, never
//! instead of it.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use ring::signature::{ED25519, UnparsedPublicKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// `schema` of a manifest this build understands.
pub const MANIFEST_SCHEMA: &str = "fyx-release/1";
/// Where the manifests live: the latest GitHub release of the Fyx repository, one pair of files
/// per channel (`fyx-<channel>.json` and `fyx-<channel>.json.sig`).
pub const RELEASES_BASE: &str = "https://github.com/shelenhovskyi/fyx/releases/latest/download";
/// A manifest is valid for at most this long after `published_at`, so an old one cannot be
/// replayed forever to hold a client on a vulnerable version (freeze attack).
pub const MAX_MANIFEST_VALIDITY_SECS: u64 = 30 * 24 * 60 * 60;
/// Clock skew tolerated for a `published_at` in the future.
pub const MAX_CLOCK_SKEW_SECS: u64 = 10 * 60;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("the manifest signature is not valid for any Fyx release key")]
    BadSignature,
    #[error("the signature file is not one base64 Ed25519 signature with a key id")]
    MalformedSignature,
    #[error("the manifest is not valid JSON for {MANIFEST_SCHEMA}: {0}")]
    MalformedManifest(String),
    #[error("the manifest is for schema {0:?}, not {MANIFEST_SCHEMA}")]
    UnknownSchema(String),
    #[error("the manifest is for channel {found:?}, not {expected:?}")]
    WrongChannel { expected: String, found: String },
    #[error("the manifest expired (published {published_at}, now {now})")]
    Expired { published_at: u64, now: u64 },
    #[error("the manifest is published in the future ({published_at}, now {now})")]
    FromTheFuture { published_at: u64, now: u64 },
    #[error("the download is {found} bytes, the manifest says {expected}")]
    WrongSize { expected: u64, found: u64 },
    #[error("the download's SHA-256 is {found}, the manifest says {expected}")]
    WrongHash { expected: String, found: String },
}

/// A public release key compiled into the build: `key_id` names it in signature files.
#[derive(Clone, Copy, Debug)]
pub struct ReleaseKey {
    pub key_id: &'static str,
    pub ed25519: [u8; 32],
}

/// The release keys of this build. Empty until the owner generates the key (plan 6.2 needs one
/// key of its own, not the registry's): with no key every manifest is refused.
pub const RELEASE_KEYS: &[ReleaseKey] = &[];

/// `fyx-<channel>.json`: what the newest release of a channel is and where its files are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub channel: String,
    pub version: Version,
    /// Unix seconds, set when the release is signed.
    pub published_at: u64,
    /// The release page, for "What's new".
    pub notes_url: String,
    pub artifacts: Vec<Artifact>,
}

/// One downloadable build of the release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// A Rust target triple, such as `aarch64-apple-darwin` or `x86_64-pc-windows-msvc`.
    pub target: String,
    pub kind: ArtifactKind,
    pub url: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    /// A `.tar.gz` holding one `.app` (macOS, ad-hoc signed for now).
    MacAppTarGz,
    /// A `.zip` holding the install folder (Windows).
    WindowsZip,
    /// A `.tar.gz` holding the install folder (Linux).
    LinuxTarGz,
}

/// `fyx-<channel>.json.sig`: `<key id> <base64 signature>` over the manifest file's exact bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetachedSignature {
    pub key_id: String,
    pub signature: Vec<u8>,
}

impl DetachedSignature {
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut words = text.split_whitespace();
        let (Some(key_id), Some(encoded), None) = (words.next(), words.next(), words.next()) else {
            return Err(Error::MalformedSignature);
        };
        let signature = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| Error::MalformedSignature)?;
        if signature.len() != 64 {
            return Err(Error::MalformedSignature);
        }
        Ok(Self {
            key_id: key_id.to_owned(),
            signature,
        })
    }
}

/// The manifest, once its signature checked out against `keys` and its fields against this
/// build: `channel` and the time `now` (Unix seconds).
pub fn verify_manifest(
    manifest_bytes: &[u8],
    signature: &DetachedSignature,
    keys: &[ReleaseKey],
    channel: &str,
    now: u64,
) -> Result<Manifest, Error> {
    let key = keys
        .iter()
        .find(|key| key.key_id == signature.key_id)
        .ok_or(Error::BadSignature)?;
    UnparsedPublicKey::new(&ED25519, key.ed25519)
        .verify(manifest_bytes, &signature.signature)
        .map_err(|_| Error::BadSignature)?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| Error::MalformedManifest(error.to_string()))?;
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(Error::UnknownSchema(manifest.schema));
    }
    if manifest.channel != channel {
        return Err(Error::WrongChannel {
            expected: channel.to_owned(),
            found: manifest.channel,
        });
    }
    let published_at = manifest.published_at;
    if published_at > now.saturating_add(MAX_CLOCK_SKEW_SECS) {
        return Err(Error::FromTheFuture { published_at, now });
    }
    if now.saturating_sub(published_at) > MAX_MANIFEST_VALIDITY_SECS {
        return Err(Error::Expired { published_at, now });
    }
    Ok(manifest)
}

/// What to tell the user about a verified manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    /// A newer version with a build for this target. Installing still waits for the user.
    Update {
        version: Version,
        artifact: Artifact,
        notes_url: String,
    },
    UpToDate,
    /// Newer, but without a build for this target.
    NoBuildForTarget(Version),
}

/// The offer for this build. `highest_seen` is the newest version any verified manifest showed
/// before (stored by the host), so a replayed older manifest never offers a downgrade.
pub fn offer(
    manifest: &Manifest,
    current: &Version,
    highest_seen: Option<&Version>,
    target: &str,
) -> Offer {
    let floor = highest_seen
        .filter(|seen| *seen > current)
        .unwrap_or(current);
    if manifest.version <= *floor {
        return Offer::UpToDate;
    }
    match manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.target == target)
    {
        Some(artifact) => Offer::Update {
            version: manifest.version.clone(),
            artifact: artifact.clone(),
            notes_url: manifest.notes_url.clone(),
        },
        None => Offer::NoBuildForTarget(manifest.version.clone()),
    }
}

/// Checks a downloaded artifact against the manifest: size, then SHA-256.
pub fn check_artifact(bytes: &[u8], artifact: &Artifact) -> Result<(), Error> {
    let found = bytes.len() as u64;
    if found != artifact.size {
        return Err(Error::WrongSize {
            expected: artifact.size,
            found,
        });
    }
    let found = hex::encode(Sha256::digest(bytes));
    if !found.eq_ignore_ascii_case(&artifact.sha256) {
        return Err(Error::WrongHash {
            expected: artifact.sha256.clone(),
            found,
        });
    }
    Ok(())
}

/// One file-system step of an install or a rollback, run by the host on restart (after Fyx's
/// windows are closed). Each is a rename on one volume, so it is atomic and undoable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Remove a folder left from an earlier update (the version before the previous one).
    RemoveIfPresent(PathBuf),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
}

/// Where an update keeps its files, all under `<data dir>/updates`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The installed app: `/Applications/Fyx.app`, or the Windows/Linux install folder.
    pub installed: PathBuf,
    /// The checked, unpacked new version, beside `installed` (same volume).
    pub staged: PathBuf,
    /// The version before, kept for rollback.
    pub previous: PathBuf,
}

impl Layout {
    /// `staged` and `previous` next to `installed`, so every rename stays on one volume.
    pub fn beside(installed: &Path) -> Self {
        let name = installed
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Fyx".to_owned());
        let sibling = |suffix: &str| installed.with_file_name(format!(".{name}.{suffix}"));
        Self {
            installed: installed.to_path_buf(),
            staged: sibling("fyx-update"),
            previous: sibling("fyx-previous"),
        }
    }
}

/// Installs `staged`: the current version becomes `previous`, the staged one `installed`.
pub fn install_plan(layout: &Layout) -> Vec<Step> {
    vec![
        Step::RemoveIfPresent(layout.previous.clone()),
        Step::Rename {
            from: layout.installed.clone(),
            to: layout.previous.clone(),
        },
        Step::Rename {
            from: layout.staged.clone(),
            to: layout.installed.clone(),
        },
    ]
}

/// Restores `previous` (after a failed start of the new version, or on the user's request); the
/// version it replaces is kept as `staged` for a later look.
pub fn rollback_plan(layout: &Layout) -> Vec<Step> {
    vec![
        Step::RemoveIfPresent(layout.staged.clone()),
        Step::Rename {
            from: layout.installed.clone(),
            to: layout.staged.clone(),
        },
        Step::Rename {
            from: layout.previous.clone(),
            to: layout.installed.clone(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const TARGET: &str = "aarch64-apple-darwin";

    fn key_pair() -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("a test key")
    }

    fn keys(pair: &Ed25519KeyPair) -> Vec<ReleaseKey> {
        let mut ed25519 = [0; 32];
        ed25519.copy_from_slice(pair.public_key().as_ref());
        vec![ReleaseKey {
            key_id: "fyx-release-test",
            ed25519,
        }]
    }

    fn artifact(bytes: &[u8]) -> Artifact {
        Artifact {
            target: TARGET.into(),
            kind: ArtifactKind::MacAppTarGz,
            url: format!("{RELEASES_BASE}/Fyx-aarch64.app.tar.gz"),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        }
    }

    fn manifest(version: &str, published_at: u64) -> Manifest {
        Manifest {
            schema: MANIFEST_SCHEMA.into(),
            channel: "stable".into(),
            version: Version::parse(version).expect("a version"),
            published_at,
            notes_url: "https://github.com/shelenhovskyi/fyx/releases/tag/v1.0.1".into(),
            artifacts: vec![artifact(b"the 1.0.1 app")],
        }
    }

    fn signed(manifest: &Manifest, pair: &Ed25519KeyPair) -> (Vec<u8>, DetachedSignature) {
        let bytes = serde_json::to_vec_pretty(manifest).expect("json");
        let text = format!(
            "fyx-release-test {}\n",
            base64::engine::general_purpose::STANDARD.encode(pair.sign(&bytes))
        );
        (bytes, DetachedSignature::parse(&text).expect("a signature"))
    }

    /// Plan 6.2 "done when": a test channel offers 1.0.1.
    #[test]
    fn a_signed_newer_release_is_offered() {
        let pair = key_pair();
        let (bytes, signature) = signed(&manifest("1.0.1", NOW - 60), &pair);
        let verified =
            verify_manifest(&bytes, &signature, &keys(&pair), "stable", NOW).expect("verified");
        let current = Version::new(1, 0, 0);
        assert!(matches!(
            offer(&verified, &current, None, TARGET),
            Offer::Update { ref version, .. } if *version == Version::new(1, 0, 1)
        ));
        assert_eq!(
            offer(&verified, &current, None, "x86_64-pc-windows-msvc"),
            Offer::NoBuildForTarget(Version::new(1, 0, 1))
        );
        assert_eq!(
            offer(&verified, &current, Some(&Version::new(1, 0, 2)), TARGET),
            Offer::UpToDate,
            "a replayed older manifest offers nothing"
        );
        assert_eq!(
            offer(&verified, &Version::new(1, 0, 1), None, TARGET),
            Offer::UpToDate
        );
    }

    #[test]
    fn a_tampered_or_foreign_manifest_is_refused() {
        let pair = key_pair();
        let (mut bytes, signature) = signed(&manifest("1.0.1", NOW), &pair);
        let keys = keys(&pair);
        if let Some(byte) = bytes.iter_mut().find(|byte| **byte == b'1') {
            *byte = b'9';
        }
        assert_eq!(
            verify_manifest(&bytes, &signature, &keys, "stable", NOW),
            Err(Error::BadSignature)
        );
        let (bytes, signature) = signed(&manifest("1.0.1", NOW), &pair);
        assert_eq!(
            verify_manifest(&bytes, &signature, &[], "stable", NOW),
            Err(Error::BadSignature),
            "no compiled-in key, no update"
        );
        assert!(matches!(
            verify_manifest(&bytes, &signature, &keys, "preview", NOW),
            Err(Error::WrongChannel { .. })
        ));
        let (old, old_signature) = signed(
            &manifest("1.0.1", NOW - MAX_MANIFEST_VALIDITY_SECS - 1),
            &pair,
        );
        assert!(matches!(
            verify_manifest(&old, &old_signature, &keys, "stable", NOW),
            Err(Error::Expired { .. })
        ));
        let (future, future_signature) = signed(&manifest("1.0.1", NOW + 3600), &pair);
        assert!(matches!(
            verify_manifest(&future, &future_signature, &keys, "stable", NOW),
            Err(Error::FromTheFuture { .. })
        ));
        for bad in ["", "only-an-id", "id not-base64!", "id QUJD", "id a b"] {
            assert_eq!(
                DetachedSignature::parse(bad),
                Err(Error::MalformedSignature),
                "{bad:?}"
            );
        }
    }

    /// Plan 6.2 "done when": a tampered package is refused.
    #[test]
    fn a_tampered_package_is_refused() {
        let artifact = artifact(b"the 1.0.1 app");
        assert_eq!(check_artifact(b"the 1.0.1 app", &artifact), Ok(()));
        assert!(matches!(
            check_artifact(b"the 1.0.1 apX", &artifact),
            Err(Error::WrongHash { .. })
        ));
        assert!(matches!(
            check_artifact(b"short", &artifact),
            Err(Error::WrongSize { .. })
        ));
    }

    /// Plan 6.2 "done when": rollback restores the previous version.
    #[test]
    fn rollback_undoes_the_install() {
        let layout = Layout::beside(Path::new("/Applications/Fyx.app"));
        assert_eq!(
            layout.staged,
            Path::new("/Applications/.Fyx.app.fyx-update")
        );
        assert_eq!(
            layout.previous,
            Path::new("/Applications/.Fyx.app.fyx-previous")
        );
        // Simulate both plans on a set of names.
        let mut present = vec![layout.installed.clone(), layout.staged.clone()];
        let run = |steps: Vec<Step>, present: &mut Vec<PathBuf>| {
            for step in steps {
                match step {
                    Step::RemoveIfPresent(path) => present.retain(|p| *p != path),
                    Step::Rename { from, to } => {
                        assert!(present.contains(&from), "{} missing", from.display());
                        assert!(!present.contains(&to), "{} in the way", to.display());
                        present.retain(|p| *p != from);
                        present.push(to);
                    }
                }
            }
        };
        run(install_plan(&layout), &mut present);
        present.sort();
        let mut expected = vec![layout.installed.clone(), layout.previous.clone()];
        expected.sort();
        assert_eq!(present, expected, "new installed, old kept");
        run(rollback_plan(&layout), &mut present);
        present.sort();
        let mut expected = vec![layout.installed, layout.staged];
        expected.sort();
        assert_eq!(
            present, expected,
            "previous restored, the failed one set aside"
        );
    }
}
